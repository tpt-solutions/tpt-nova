//! Forward PBR rendering with shadow-casting lights (Phase 3 cinematic core).
//!
//! [`PbrRenderer`] is a self-contained wgpu renderer that draws the ECS world
//! with physically-inspired (Lambert + ambient) shading and renders a shadow
//! map for the first shadow-casting [`Light`](nova_ecs::Light). It reads the
//! resolved [`ActiveCamera`](nova_ecs::ActiveCamera) view-projection so it
//! composes with the virtual-camera system, and reads [`Light`]s for shading
//! and shadow projection.
//!
//! The GPU code is compile-verified against the workspace's wgpu 30 API; it is
//! exposed alongside the simpler cube renderer so the runtime can pick a
//! pipeline. Per-object model matrices are uploaded from each entity's
//! [`GlobalTransform`](nova_ecs::GlobalTransform).

use std::sync::Arc;

use nova_ecs::light::Light;
use nova_ecs::transform::{GlobalTransform, Mesh};
use nova_ecs::world::World;
use nova_ecs::{ActiveCamera, Mat4, Vec3};
use wgpu::util::DeviceExt;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_SIZE: u32 = 1024;

/// Per-draw + per-frame constants shared between the shadow and main passes.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    params: [f32; 4], // x = intensity, y = shadow_extent
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

impl Vertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

/// A forward PBR renderer with one shadow-casting directional light.
pub struct PbrRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    main_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    // Kept alive for the lifetime of the bind groups that reference them.
    #[allow(dead_code)]
    main_bgl: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    shadow_bgl: wgpu::BindGroupLayout,
    globals_buffer: wgpu::Buffer,
    shadow_view: wgpu::TextureView,
    #[allow(dead_code)]
    shadow_sampler: wgpu::Sampler,
    main_depth_view: wgpu::TextureView,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    main_bind_group: wgpu::BindGroup,
    shadow_bind_group: wgpu::BindGroup,
}

impl PbrRenderer {
    pub fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nova-pbr-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            }))?;

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface
                .get_capabilities(&adapter)
                .formats
                .iter()
                .copied()
                .next()
                .unwrap_or(wgpu::TextureFormat::Bgra8Unorm),
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            color_space: wgpu::SurfaceColorSpace::Auto,
            desired_maximum_frame_latency: 2,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-shader"),
            source: wgpu::ShaderSource::Wgsl(PBR_SHADER.into()),
        });

        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr-shadow-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let main_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr-main-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });

        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr-shadow-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("pbr-shadow-pl"),
                    bind_group_layouts: &[Some(&shadow_bgl)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[Some(Vertex::layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Front),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let main_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr-main-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("pbr-main-pl"),
                    bind_group_layouts: &[Some(&main_bgl)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(Vertex::layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pbr-globals"),
            size: std::mem::size_of::<Globals>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pbr-shadow-map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pbr-shadow-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::Less),
            ..Default::default()
        });

        let main_depth_view = create_depth_view(&device, &config);

        let main_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr-main-bg"),
            layout: &main_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pbr-shadow-bg"),
            layout: &shadow_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let (vertices, indices) = build_cube_pbr();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pbr-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pbr-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Ok(PbrRenderer {
            window,
            surface,
            device,
            queue,
            config,
            main_pipeline,
            shadow_pipeline,
            main_bgl,
            shadow_bgl,
            globals_buffer,
            shadow_view,
            shadow_sampler,
            main_depth_view,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            main_bind_group,
            shadow_bind_group,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.main_depth_view = create_depth_view(&self.device, &self.config);
    }

    /// Render the world with PBR shading and one shadow-casting light.
    pub fn render(&mut self, world: &World) -> anyhow::Result<()> {
        // Resolve the active camera (or fall back to identity).
        let camera_vp = world
            .resource::<ActiveCamera>()
            .map(|a| a.0.view_proj())
            .unwrap_or(Mat4::IDENTITY);
        let camera_pos = world
            .resource::<ActiveCamera>()
            .map(|a| a.0.translation)
            .unwrap_or(Vec3::ZERO);

        // Pick the first light as the shadow-casting directional light.
        let light_info = world
            .query_2::<Light, GlobalTransform>()
            .into_iter()
            .next()
            .map(|(_, light, gt)| {
                let (_, rot, _) = gt.0.to_scale_rotation_translation();
                let dir = light.direction(rot);
                let extent = light.shadow_extent.max(0.5);
                let eye = -dir * extent;
                let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
                let proj =
                    Mat4::orthographic_rh(-extent, extent, -extent, extent, 0.1, extent * 2.0);
                (light, dir, proj * view, extent)
            });

        let (light_dir, light_color, light_intensity, light_vp, shadow_extent) = match light_info {
            Some((l, d, vp, ext)) => (d, l.color, l.intensity, vp, ext),
            None => (
                Vec3::new(0.4, -0.8, 0.3),
                Vec3::new(1.0, 1.0, 1.0),
                1.0,
                Mat4::IDENTITY,
                10.0,
            ),
        };

        // --- Shadow pass: render scene depth from the light's POV ---------
        {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("pbr-shadow-encoder"),
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pbr-shadow-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

            for (_, _, gt) in world.query_2::<Mesh, GlobalTransform>() {
                if gt.0 == Mat4::ZERO {
                    continue;
                }
                self.write_globals(
                    camera_vp,
                    light_vp,
                    gt.0,
                    camera_pos,
                    light_dir,
                    light_color,
                    light_intensity,
                    shadow_extent,
                );
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            drop(pass);
            self.queue.submit(std::iter::once(encoder.finish()));
        }

        // --- Main pass: shaded geometry, sampling the shadow map ----------
        let frame = self.surface.get_current_texture();
        let texture = match frame {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                log::warn!("surface texture unavailable: {other:?}");
                return Ok(());
            }
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pbr-main-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pbr-main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.07,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.main_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.main_pipeline);
            pass.set_bind_group(0, &self.main_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

            for (_, _, gt) in world.query_2::<Mesh, GlobalTransform>() {
                if gt.0 == Mat4::ZERO {
                    continue;
                }
                self.write_globals(
                    camera_vp,
                    light_vp,
                    gt.0,
                    camera_pos,
                    light_dir,
                    light_color,
                    light_intensity,
                    shadow_extent,
                );
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(texture);
        Ok(())
    }

    /// Upload per-object globals to the uniform buffer (the bind group already
    /// points at it, set by the caller before drawing).
    #[allow(clippy::too_many_arguments)]
    fn write_globals(
        &self,
        view_proj: Mat4,
        light_vp: Mat4,
        model: Mat4,
        camera_pos: Vec3,
        light_dir: Vec3,
        light_color: Vec3,
        intensity: f32,
        shadow_extent: f32,
    ) {
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            light_view_proj: light_vp.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, 0.0],
            light_dir: [light_dir.x, light_dir.y, light_dir.z, 0.0],
            light_color: [light_color.x, light_color.y, light_color.z, 0.0],
            params: [intensity, shadow_extent, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::cast_slice(&[globals]));
    }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pbr-depth-texture"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Unit cube with per-face normals (positions + normals).
fn build_cube_pbr() -> (Vec<Vertex>, Vec<u16>) {
    let s = 0.5f32;
    // Each face: four corner positions plus a shared normal.
    type Face = ([f32; 3], [f32; 3], [f32; 3], [f32; 3], [f32; 3]);
    let faces: [Face; 6] = [
        (
            [s, -s, -s],
            [s, s, -s],
            [s, s, s],
            [s, -s, s],
            [1.0, 0.0, 0.0],
        ),
        (
            [-s, -s, s],
            [-s, s, s],
            [-s, s, -s],
            [-s, -s, -s],
            [-1.0, 0.0, 0.0],
        ),
        (
            [-s, s, -s],
            [-s, s, s],
            [s, s, s],
            [s, s, -s],
            [0.0, 1.0, 0.0],
        ),
        (
            [-s, -s, s],
            [-s, -s, -s],
            [s, -s, -s],
            [s, -s, s],
            [0.0, -1.0, 0.0],
        ),
        (
            [-s, -s, s],
            [s, -s, s],
            [s, s, s],
            [-s, s, s],
            [0.0, 0.0, 1.0],
        ),
        (
            [s, -s, -s],
            [-s, -s, -s],
            [-s, s, -s],
            [s, s, -s],
            [0.0, 0.0, -1.0],
        ),
    ];
    let mut vertices = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for (a, b, c, d, n) in faces.iter() {
        let base = vertices.len() as u16;
        vertices.push(Vertex {
            position: *a,
            normal: *n,
        });
        vertices.push(Vertex {
            position: *b,
            normal: *n,
        });
        vertices.push(Vertex {
            position: *c,
            normal: *n,
        });
        vertices.push(Vertex {
            position: *d,
            normal: *n,
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

const PBR_SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    params: vec4<f32>, // x = intensity, y = shadow_extent
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) shadow_coord: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> VsOut {
    var out: VsOut;
    let world = (globals.model * vec4<f32>(position, 1.0)).xyz;
    out.world = world;
    out.normal = (globals.model * vec4<f32>(normal, 0.0)).xyz;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.shadow_coord = globals.light_view_proj * vec4<f32>(world, 1.0);
    return out;
}

@vertex
fn vs_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    let world = (globals.model * vec4<f32>(position, 1.0)).xyz;
    return globals.light_view_proj * vec4<f32>(world, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(-globals.light_dir.xyz);
    var shadow = 1.0;
    if (in.shadow_coord.w > 0.0) {
        let p = in.shadow_coord.xyz / in.shadow_coord.w;
        let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
        if (uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0) {
            shadow = textureSampleCompare(shadow_map, shadow_sampler, uv, p.z - 0.0025);
        }
    }
    let diff = max(dot(n, l), 0.0);
    let ambient = 0.15;
    let color = globals.light_color.rgb * globals.params.x * (diff * shadow + ambient);
    return vec4<f32>(color, 1.0);
}
"#;
