//! Full-screen post-processing: tone mapping, bloom, anti-aliasing, and
//! user-registered passes.
//!
//! [`PostProcessChain`] owns the GPU plumbing for a stack of full-screen
//! effects. Each effect samples the previous pass's texture and writes the next
//! one; the chain ping-pongs between two scratch targets so effects never read
//! and write the same texture.
//!
//! Effects are chosen declaratively with [`Effect`]. The high-level scene graph
//! (`tpt-nova-scene-graph`) lowers its serializable effect stack into this list,
//! which keeps wgpu types out of scene data while still letting the renderer own
//! the pipelines. Custom effects implement [`PostProcessPass`] and are added with
//! [`PostProcessChain::add_custom_pass`].
//!
//! ```no_run
//! use tpt_nova_render::post::{BloomSettings, Effect, PostProcessChain, ToneMapping};
//!
//! # fn demo(device: &wgpu::Device, queue: &wgpu::Queue, src: &wgpu::TextureView, dst: &wgpu::TextureView) {
//! let mut chain = PostProcessChain::new(device, wgpu::TextureFormat::Bgra8UnormSrgb, 1280, 720);
//! chain.set_effects(
//!     device,
//!     &[
//!         Effect::Bloom(BloomSettings::default()),
//!         Effect::ToneMapping(ToneMapping::aces()),
//!     ],
//! );
//! let mut encoder = device.create_command_encoder(&Default::default());
//! chain.process(device, queue, &mut encoder, src, dst);
//! queue.submit(Some(encoder.finish()));
//! # }
//! ```

use wgpu::{
    BindGroup, BindGroupLayout, BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType,
    Color, CommandEncoder, Device, FragmentState, LoadOp, Operations, PipelineLayoutDescriptor,
    PrimitiveState, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerDescriptor, ShaderModule, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, Texture, TextureDescriptor, TextureFormat, TextureUsages,
    TextureView, TextureViewDescriptor, VertexState,
};

/// Tone-mapping curve applied to the HDR-ish scene color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToneMapping {
    /// Narkowicz' ACES filmic fit — the default, filmic highlight roll-off.
    Aces { exposure: f32 },
    /// Simple Reinhard `c / (1 + c)`.
    Reinhard { exposure: f32 },
}

impl ToneMapping {
    pub fn aces() -> Self {
        ToneMapping::Aces { exposure: 1.0 }
    }

    pub fn reinhard() -> Self {
        ToneMapping::Reinhard { exposure: 1.0 }
    }

    pub fn exposure(&self) -> f32 {
        match self {
            ToneMapping::Aces { exposure } | ToneMapping::Reinhard { exposure } => *exposure,
        }
    }

    pub fn with_exposure(mut self, exposure: f32) -> Self {
        match &mut self {
            ToneMapping::Aces { exposure: e } | ToneMapping::Reinhard { exposure: e } => {
                *e = exposure
            }
        }
        self
    }
}

impl Default for ToneMapping {
    fn default() -> Self {
        ToneMapping::aces()
    }
}

/// Bloom parameters: how bright a pixel must be to glow, and how strong the
/// glow is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomSettings {
    pub threshold: f32,
    pub intensity: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        BloomSettings {
            threshold: 0.8,
            intensity: 0.6,
        }
    }
}

/// A post-process effect the chain knows how to build.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    ToneMapping(ToneMapping),
    Bloom(BloomSettings),
    /// One-pass FXAA (luma-edge-aware 5-tap blend).
    Fxaa,
}

/// A user-defined full-screen pass.
///
/// Implementations own their pipelines and bind groups, and encode one or more
/// render passes into the supplied encoder. `input` is the previous pass's
/// output; `output` is where the pass must write. The two views are always
/// different textures.
pub trait PostProcessPass: Send {
    fn label(&self) -> &'static str;

    /// Called when the chain's target size changes (and once when registered).
    fn resize(&mut self, device: &Device, width: u32, height: u32);

    /// Record the pass reading `input` and writing `output`.
    fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    );
}

// ---------------------------------------------------------------------------
// Chain
// ---------------------------------------------------------------------------

/// A stack of full-screen effects plus its ping-pong render targets.
pub struct PostProcessChain {
    format: TextureFormat,
    width: u32,
    height: u32,
    builtins: Vec<BuiltinEffect>,
    custom: Vec<Box<dyn PostProcessPass>>,
    scratch: Option<(Texture, Texture, TextureView, TextureView)>,
    blit: BlitPass,
}

impl PostProcessChain {
    pub fn new(device: &Device, format: TextureFormat, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        PostProcessChain {
            format,
            width,
            height,
            builtins: Vec::new(),
            custom: Vec::new(),
            scratch: None,
            blit: BlitPass::new(device, format),
        }
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Replace the built-in effect stack (custom passes are preserved).
    pub fn set_effects(&mut self, device: &Device, effects: &[Effect]) {
        self.builtins = effects
            .iter()
            .map(|effect| match effect {
                Effect::ToneMapping(mapping) => BuiltinEffect::ToneMap(Box::new(ToneMapPass::new(
                    device,
                    self.format,
                    *mapping,
                ))),
                Effect::Bloom(settings) => {
                    BuiltinEffect::Bloom(Box::new(BloomPass::new(device, self.format, *settings)))
                }
                Effect::Fxaa => BuiltinEffect::Fxaa(Box::new(FxaaPass::new(device, self.format))),
            })
            .collect();
        self.resize_passes(device);
    }

    /// Number of built-in effects in the stack.
    pub fn effect_count(&self) -> usize {
        self.builtins.len()
    }

    pub fn custom_pass_count(&self) -> usize {
        self.custom.len()
    }

    /// Register a custom full-screen pass. Custom passes run after the built-in
    /// stack, in registration order.
    pub fn add_custom_pass(&mut self, device: &Device, mut pass: Box<dyn PostProcessPass>) {
        pass.resize(device, self.width, self.height);
        self.custom.push(pass);
    }

    /// Resize the render targets. Call on surface resize before the next
    /// [`Self::process`].
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.scratch = None;
        self.resize_passes(device);
    }

    fn resize_passes(&mut self, device: &Device) {
        for effect in &mut self.builtins {
            effect.resize(device, self.width, self.height);
        }
        for pass in &mut self.custom {
            pass.resize(device, self.width, self.height);
        }
    }

    fn ensure_scratch(&mut self, device: &Device) {
        if self.scratch.is_some() {
            return;
        }
        let make = |label: &str| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            (texture, view)
        };
        let (a_tex, a_view) = make("post-process-scratch-a");
        let (b_tex, b_view) = make("post-process-scratch-b");
        self.scratch = Some((a_tex, b_tex, a_view, b_view));
    }

    /// Encode the whole stack from `input` into `output`.
    ///
    /// With no effects and no custom passes this is a blit, so the chain can be
    /// left enabled unconditionally. `input` and `output` must be distinct
    /// views with the chain's format and size.
    pub fn process(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let total = self.builtins.len() + self.custom.len();
        if total == 0 {
            self.blit.apply(device, queue, encoder, input, output);
            return;
        }
        self.ensure_scratch(device);

        let builtin_count = self.builtins.len();
        let scratch = self
            .scratch
            .as_ref()
            .expect("scratch targets created above");
        for i in 0..total {
            // Pick the ping-pong view written by the previous pass.
            let src = if i == 0 {
                input
            } else if (i - 1) % 2 == 0 {
                &scratch.2
            } else {
                &scratch.3
            };
            let dst = if i + 1 == total {
                output
            } else if i % 2 == 0 {
                &scratch.2
            } else {
                &scratch.3
            };
            if i < builtin_count {
                self.builtins[i].apply(device, queue, encoder, src, dst);
            } else {
                self.custom[i - builtin_count].apply(device, queue, encoder, src, dst);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in passes
// ---------------------------------------------------------------------------

enum BuiltinEffect {
    // Boxed so the variants stay similar in size (clippy::large_enum_variant).
    ToneMap(Box<ToneMapPass>),
    Bloom(Box<BloomPass>),
    Fxaa(Box<FxaaPass>),
}

impl BuiltinEffect {
    fn resize(&mut self, device: &Device, width: u32, height: u32) {
        match self {
            BuiltinEffect::ToneMap(p) => p.resize(device, width, height),
            BuiltinEffect::Bloom(p) => p.resize(device, width, height),
            BuiltinEffect::Fxaa(p) => p.resize(device, width, height),
        }
    }

    fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let pass: &mut dyn PostProcessPass = match self {
            BuiltinEffect::ToneMap(p) => &mut **p,
            BuiltinEffect::Bloom(p) => &mut **p,
            BuiltinEffect::Fxaa(p) => &mut **p,
        };
        pass.apply(device, queue, encoder, input, output);
    }
}

/// Shared texture+sampler+uniform bind group layout.
fn effect_layout(device: &Device, label: &str) -> BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn fullscreen_shader(device: &Device, label: &str, source: &str) -> ShaderModule {
    device.create_shader_module(ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(source.into()),
    })
}

fn fullscreen_pipeline(
    device: &Device,
    label: &str,
    format: TextureFormat,
    layout: &BindGroupLayout,
    shader: &ShaderModule,
) -> RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn linear_sampler(device: &Device, label: &str) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label: Some(label),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

fn uniform_buffer(device: &Device, label: &str) -> Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn run_fullscreen(
    encoder: &mut CommandEncoder,
    label: &str,
    pipeline: &RenderPipeline,
    bind_group: &BindGroup,
    output: &TextureView,
) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: output,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(Color::TRANSPARENT),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

// -- blit -------------------------------------------------------------------

struct BlitPass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: Sampler,
    uniform: Buffer,
}

impl BlitPass {
    fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = fullscreen_shader(device, "post-blit-shader", BLIT_WGSL);
        let layout = effect_layout(device, "post-blit-bgl");
        let pipeline = fullscreen_pipeline(device, "post-blit", format, &layout, &shader);
        BlitPass {
            pipeline,
            layout,
            sampler: linear_sampler(device, "post-blit-sampler"),
            uniform: uniform_buffer(device, "post-blit-uniform"),
        }
    }
}

impl PostProcessPass for BlitPass {
    fn label(&self) -> &'static str {
        "blit"
    }

    fn resize(&mut self, _device: &Device, _width: u32, _height: u32) {}

    fn apply(
        &mut self,
        device: &Device,
        _queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-blit-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        run_fullscreen(
            encoder,
            "post-blit-pass",
            &self.pipeline,
            &bind_group,
            output,
        );
    }
}

// -- tone mapping -----------------------------------------------------------

struct ToneMapPass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: Sampler,
    uniform: Buffer,
    mapping: ToneMapping,
}

impl ToneMapPass {
    fn new(device: &Device, format: TextureFormat, mapping: ToneMapping) -> Self {
        let shader = fullscreen_shader(device, "post-tonemap-shader", TONEMAP_WGSL);
        let layout = effect_layout(device, "post-tonemap-bgl");
        let pipeline = fullscreen_pipeline(device, "post-tonemap", format, &layout, &shader);
        ToneMapPass {
            pipeline,
            layout,
            sampler: linear_sampler(device, "post-tonemap-sampler"),
            uniform: uniform_buffer(device, "post-tonemap-uniform"),
            mapping,
        }
    }
}

impl PostProcessPass for ToneMapPass {
    fn label(&self) -> &'static str {
        "tone-mapping"
    }

    fn resize(&mut self, _device: &Device, _width: u32, _height: u32) {}

    fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let mode = match self.mapping {
            ToneMapping::Aces { .. } => 0u32,
            ToneMapping::Reinhard { .. } => 1u32,
        };
        let params: [u32; 4] = [self.mapping.exposure().to_bits(), mode, 0, 0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-tonemap-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        run_fullscreen(
            encoder,
            "post-tonemap-pass",
            &self.pipeline,
            &bind_group,
            output,
        );
    }
}

// -- FXAA -------------------------------------------------------------------

struct FxaaPass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: Sampler,
    uniform: Buffer,
    texel: [f32; 2],
}

impl FxaaPass {
    fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = fullscreen_shader(device, "post-fxaa-shader", FXAA_WGSL);
        let layout = effect_layout(device, "post-fxaa-bgl");
        let pipeline = fullscreen_pipeline(device, "post-fxaa", format, &layout, &shader);
        FxaaPass {
            pipeline,
            layout,
            sampler: linear_sampler(device, "post-fxaa-sampler"),
            uniform: uniform_buffer(device, "post-fxaa-uniform"),
            texel: [1.0, 1.0],
        }
    }
}

impl PostProcessPass for FxaaPass {
    fn label(&self) -> &'static str {
        "fxaa"
    }

    fn resize(&mut self, _device: &Device, width: u32, height: u32) {
        self.texel = [1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32];
    }

    fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let params = [self.texel[0], self.texel[1], 0.05, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-fxaa-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        run_fullscreen(
            encoder,
            "post-fxaa-pass",
            &self.pipeline,
            &bind_group,
            output,
        );
    }
}

// -- bloom ------------------------------------------------------------------

struct BloomPass {
    settings: BloomSettings,
    threshold_pipeline: RenderPipeline,
    threshold_layout: BindGroupLayout,
    blur_pipeline: RenderPipeline,
    blur_layout: BindGroupLayout,
    composite_pipeline: RenderPipeline,
    composite_layout: BindGroupLayout,
    sampler: Sampler,
    threshold_uniform: Buffer,
    blur_uniform: Buffer,
    composite_uniform: Buffer,
    textures: Option<[Texture; 2]>,
    views: Option<[TextureView; 2]>,
    texel: [f32; 2],
}

impl BloomPass {
    fn new(device: &Device, format: TextureFormat, settings: BloomSettings) -> Self {
        let threshold_shader =
            fullscreen_shader(device, "post-bloom-threshold-shader", THRESHOLD_WGSL);
        let blur_shader = fullscreen_shader(device, "post-bloom-blur-shader", BLUR_WGSL);
        let composite_shader =
            fullscreen_shader(device, "post-bloom-composite-shader", COMPOSITE_WGSL);

        let threshold_layout = effect_layout(device, "post-bloom-threshold-bgl");
        let blur_layout = effect_layout(device, "post-bloom-blur-bgl");
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-bloom-composite-bgl"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        BloomPass {
            settings,
            threshold_pipeline: fullscreen_pipeline(
                device,
                "post-bloom-threshold",
                TextureFormat::Rgba16Float,
                &threshold_layout,
                &threshold_shader,
            ),
            threshold_layout,
            blur_pipeline: fullscreen_pipeline(
                device,
                "post-bloom-blur",
                TextureFormat::Rgba16Float,
                &blur_layout,
                &blur_shader,
            ),
            blur_layout,
            composite_pipeline: fullscreen_pipeline(
                device,
                "post-bloom-composite",
                format,
                &composite_layout,
                &composite_shader,
            ),
            composite_layout,
            sampler: linear_sampler(device, "post-bloom-sampler"),
            threshold_uniform: uniform_buffer(device, "post-bloom-threshold-uniform"),
            blur_uniform: uniform_buffer(device, "post-bloom-blur-uniform"),
            composite_uniform: uniform_buffer(device, "post-bloom-composite-uniform"),
            textures: None,
            views: None,
            texel: [1.0, 1.0],
        }
    }

    fn ensure_targets(&mut self, device: &Device, width: u32, height: u32) {
        let (w, h) = ((width / 2).max(1), (height / 2).max(1));
        self.texel = [1.0 / w as f32, 1.0 / h as f32];
        if self.textures.is_some() {
            return;
        }
        let make = |label: &str| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            (texture, view)
        };
        let (a_tex, a_view) = make("post-bloom-bright-a");
        let (b_tex, b_view) = make("post-bloom-bright-b");
        self.textures = Some([a_tex, b_tex]);
        self.views = Some([a_view, b_view]);
    }
}

impl PostProcessPass for BloomPass {
    fn label(&self) -> &'static str {
        "bloom"
    }

    fn resize(&mut self, device: &Device, width: u32, height: u32) {
        self.textures = None;
        self.views = None;
        self.ensure_targets(device, width, height);
    }

    fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        output: &TextureView,
    ) {
        let views = self.views.as_ref().expect("bloom targets");
        let bright_params = [self.settings.threshold, 0.0, 0.0, 0.0];
        queue.write_buffer(
            &self.threshold_uniform,
            0,
            bytemuck::cast_slice(&bright_params),
        );

        // 1. Bright pass, full-res input -> half-res bright A.
        let threshold_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-bloom-threshold-bg"),
            layout: &self.threshold_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.threshold_uniform.as_entire_binding(),
                },
            ],
        });
        run_fullscreen(
            encoder,
            "post-bloom-threshold-pass",
            &self.threshold_pipeline,
            &threshold_bg,
            &views[0],
        );

        // 2. Separable blur: A -> B (horizontal), B -> A (vertical).
        for (i, (dir, dst)) in [
            ([self.texel[0], 0.0], &views[1]),
            ([0.0, self.texel[1]], &views[0]),
        ]
        .into_iter()
        .enumerate()
        {
            queue.write_buffer(
                &self.blur_uniform,
                0,
                bytemuck::cast_slice(&[dir[0], dir[1], 0.0, 0.0]),
            );
            let src = if i == 0 { &views[0] } else { &views[1] };
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post-bloom-blur-bg"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.blur_uniform.as_entire_binding(),
                    },
                ],
            });
            run_fullscreen(
                encoder,
                "post-bloom-blur-pass",
                &self.blur_pipeline,
                &bg,
                dst,
            );
        }

        // 3. Composite: output = scene + blurred bright * intensity.
        queue.write_buffer(
            &self.composite_uniform,
            0,
            bytemuck::cast_slice(&[self.settings.intensity, 0.0, 0.0, 0.0]),
        );
        let composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-bloom-composite-bg"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.composite_uniform.as_entire_binding(),
                },
            ],
        });
        run_fullscreen(
            encoder,
            "post-bloom-composite-pass",
            &self.composite_pipeline,
            &composite_bg,
            output,
        );
    }
}

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_sampler, in.uv);
}
"#;

const TONEMAP_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

struct Params {
    exposure: f32,
    mode: u32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

fn aces_fitted(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(src_tex, src_sampler, in.uv).rgb * params.exposure;
    if (params.mode == 1u) {
        c = c / (vec3<f32>(1.0) + c);
    } else {
        c = aces_fitted(c);
    }
    return vec4<f32>(c, 1.0);
}
"#;

const FXAA_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

struct Params {
    texel: vec2<f32>,
    threshold: f32,
    _pad: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let center = textureSample(src_tex, src_sampler, uv);
    let nw = textureSample(src_tex, src_sampler, uv + vec2<f32>(-1.0, -1.0) * params.texel);
    let ne = textureSample(src_tex, src_sampler, uv + vec2<f32>(1.0, -1.0) * params.texel);
    let sw = textureSample(src_tex, src_sampler, uv + vec2<f32>(-1.0, 1.0) * params.texel);
    let se = textureSample(src_tex, src_sampler, uv + vec2<f32>(1.0, 1.0) * params.texel);

    let lmin = min(luma(center.rgb), min(min(luma(nw.rgb), luma(ne.rgb)), min(luma(sw.rgb), luma(se.rgb))));
    let lmax = max(luma(center.rgb), max(max(luma(nw.rgb), luma(ne.rgb)), max(luma(sw.rgb), luma(se.rgb))));
    if (lmax - lmin <= params.threshold) {
        return center;
    }
    let avg = (nw + ne + sw + se) * 0.25;
    return vec4<f32>(mix(center.rgb, avg.rgb, 0.5), 1.0);
}
"#;

const THRESHOLD_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

struct Params {
    threshold: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_sampler, in.uv).rgb;
    let bright = max(c - vec3<f32>(params.threshold), vec3<f32>(0.0));
    return vec4<f32>(bright, 1.0);
}
"#;

const BLUR_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

struct Params {
    dir: vec2<f32>,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // 9-tap Gaussian.
    var weights = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var color = textureSample(src_tex, src_sampler, in.uv).rgb * weights[0];
    for (var i = 1u; i < 5u; i = i + 1u) {
        let off = params.dir * f32(i);
        color = color
            + textureSample(src_tex, src_sampler, in.uv + off).rgb * weights[i]
            + textureSample(src_tex, src_sampler, in.uv - off).rgb * weights[i];
    }
    return vec4<f32>(color, 1.0);
}
"#;

const COMPOSITE_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

struct Params {
    intensity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSample(scene_tex, scene_sampler, in.uv).rgb;
    let bloom = textureSample(bloom_tex, scene_sampler, in.uv).rgb;
    return vec4<f32>(scene + bloom * params.intensity, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn try_device() -> Option<(Device, Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("post-test-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            ..Default::default()
        }))
        .ok()
    }

    const SIZE: u32 = 64;
    const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

    fn target(device: &Device, label: &str) -> (Texture, TextureView) {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        (texture, view)
    }

    fn upload(queue: &Queue, texture: &Texture, rgba: [u8; 4]) {
        let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for _ in 0..(SIZE * SIZE) {
            data.extend_from_slice(&rgba);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    fn read_center(device: &Device, queue: &Queue, view_texture: &Texture) -> [u8; 4] {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post-test-readback"),
            size: (SIZE * SIZE * 4) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: view_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range().unwrap();
        let idx = ((SIZE / 2) * SIZE + (SIZE / 2)) as usize * 4;
        let px = [data[idx], data[idx + 1], data[idx + 2], data[idx + 3]];
        drop(data);
        readback.unmap();
        px
    }

    fn run_chain(device: &Device, queue: &Queue, effects: &[Effect], src: &Texture, dst: &Texture) {
        let mut chain = PostProcessChain::new(device, FORMAT, SIZE, SIZE);
        chain.set_effects(device, effects);
        let src_view = src.create_view(&TextureViewDescriptor::default());
        let dst_view = dst.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        chain.process(device, queue, &mut encoder, &src_view, &dst_view);
        queue.submit(Some(encoder.finish()));
    }

    #[test]
    fn empty_chain_blits_input_to_output() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping post-process test: no adapter");
            return;
        };
        let (src, _) = target(&device, "post-test-src");
        let (dst, _) = target(&device, "post-test-dst");
        upload(&queue, &src, [40, 90, 160, 255]);
        run_chain(&device, &queue, &[], &src, &dst);
        assert_eq!(read_center(&device, &queue, &dst), [40, 90, 160, 255]);
    }

    #[test]
    fn aces_tone_mapping_matches_reference() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping post-process test: no adapter");
            return;
        };
        let (src, _) = target(&device, "post-test-src");
        let (dst, _) = target(&device, "post-test-dst");
        upload(&queue, &src, [128, 128, 128, 255]);
        run_chain(
            &device,
            &queue,
            &[Effect::ToneMapping(ToneMapping::aces())],
            &src,
            &dst,
        );
        let px = read_center(&device, &queue, &dst)[0];
        // ACES(0.502) ≈ 0.6175; allow for 8-bit rounding.
        assert!((150..=165).contains(&px), "ACES output was {px}");
    }

    #[test]
    fn bloom_brightens_pixels_above_threshold() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping post-process test: no adapter");
            return;
        };
        let (src, _) = target(&device, "post-test-src");
        let (dst, _) = target(&device, "post-test-dst");
        upload(&queue, &src, [153, 153, 153, 255]); // 0.6 linear
        run_chain(
            &device,
            &queue,
            &[Effect::Bloom(BloomSettings {
                threshold: 0.5,
                intensity: 0.5,
            })],
            &src,
            &dst,
        );
        let px = read_center(&device, &queue, &dst)[0];
        assert!(px > 158, "bloom should add glow above threshold (got {px})");
    }

    #[test]
    fn fxaa_leaves_flat_colors_untouched() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping post-process test: no adapter");
            return;
        };
        let (src, _) = target(&device, "post-test-src");
        let (dst, _) = target(&device, "post-test-dst");
        upload(&queue, &src, [70, 120, 200, 255]);
        run_chain(&device, &queue, &[Effect::Fxaa], &src, &dst);
        let px = read_center(&device, &queue, &dst);
        assert!(
            (px[0] as i32 - 70).abs() <= 2,
            "fxaa changed flat color: {px:?}"
        );
        assert!(
            (px[2] as i32 - 200).abs() <= 2,
            "fxaa changed flat color: {px:?}"
        );
    }

    #[test]
    fn custom_pass_runs_after_builtins() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping post-process test: no adapter");
            return;
        };

        struct SolidPass {
            pipeline: RenderPipeline,
            bind_group: BindGroup,
        }

        impl PostProcessPass for SolidPass {
            fn label(&self) -> &'static str {
                "solid"
            }
            fn resize(&mut self, _device: &Device, _w: u32, _h: u32) {}
            fn apply(
                &mut self,
                _device: &Device,
                _queue: &Queue,
                encoder: &mut CommandEncoder,
                _input: &TextureView,
                output: &TextureView,
            ) {
                run_fullscreen(
                    encoder,
                    "solid-pass",
                    &self.pipeline,
                    &self.bind_group,
                    output,
                );
            }
        }

        // A full-screen triangle that paints opaque green.
        const SOLID_WGSL: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}
@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
}
"#;
        let shader = fullscreen_shader(&device, "solid-shader", SOLID_WGSL);
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("solid-bgl"),
            entries: &[],
        });
        let pipeline = fullscreen_pipeline(&device, "solid", FORMAT, &bgl, &shader);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("solid-bg"),
            layout: &bgl,
            entries: &[],
        });

        let (src, _) = target(&device, "post-test-src");
        let (dst, _) = target(&device, "post-test-dst");
        upload(&queue, &src, [10, 10, 10, 255]);

        let mut chain = PostProcessChain::new(&device, FORMAT, SIZE, SIZE);
        chain.set_effects(&device, &[Effect::ToneMapping(ToneMapping::aces())]);
        chain.add_custom_pass(
            &device,
            Box::new(SolidPass {
                pipeline,
                bind_group,
            }),
        );
        assert_eq!(chain.effect_count(), 1);
        assert_eq!(chain.custom_pass_count(), 1);

        let src_view = src.create_view(&TextureViewDescriptor::default());
        let dst_view = dst.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        chain.process(&device, &queue, &mut encoder, &src_view, &dst_view);
        queue.submit(Some(encoder.finish()));

        let px = read_center(&device, &queue, &dst);
        assert!(px[1] > 200 && px[0] < 40, "custom pass not last: {px:?}");
    }
}
