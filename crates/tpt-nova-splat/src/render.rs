//! wgpu render pipeline for Gaussian Splats (feature `render`).
//!
//! Each splat is drawn as an oriented screen-space ellipse that follows its
//! projected 3D covariance (EWA splatting): the perspective Jacobian maps the
//! world covariance to a 2D pixel covariance, its eigen-decomposition orients a
//! quad sized to ±3σ, and the fragment stage applies the Gaussian falloff.
//! Color is premultiplied and composited back-to-front with depth writes off, so
//! splats are depth-tested against the opaque scene but never occlude each other
//! through the depth buffer.
//!
//! The cloud is split into spatial chunks ([`crate::lod`]). Each chunk owns its
//! storage buffers, so very large clouds upload incrementally and chunks outside
//! the frustum are skipped. Per-chunk order is re-sorted only when the camera
//! changes. Higher-order SH is evaluated in the shader from the view direction,
//! capped by [`SplatQuality`].
//!
//! Integration: build a [`SplatPipeline`] from the renderer's `device` and
//! surface format, call [`SplatPipeline::set_cloud`] when the cloud changes, and
//! call [`SplatPipeline::render`] after the opaque passes.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::{Device, Queue, RenderPipeline, TextureView};

use crate::lod::{build_chunks, camera_position, visible_order, Frustum, LodConfig};
use crate::sh::{SplatQuality, SH_FLOATS};
use crate::{Aabb, SplatCloud};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    /// `(width, height, fx, fy)` — viewport in pixels and focal lengths in pixels.
    viewport: [f32; 4],
    /// `(sh_degree, 0, 0, 0)`.
    params: [f32; 4],
}

/// GPU layout of one splat. Matches `SplatGpu` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SplatGpu {
    /// xyz center, w opacity.
    center: [f32; 4],
    /// Upper-left covariance entries: c00, c01, c02, c11.
    cov_a: [f32; 4],
    /// c12, c22, unused, unused.
    cov_b: [f32; 4],
    /// Linear RGB (DC color) and unused.
    color: [f32; 4],
}

/// One spatial chunk's GPU resources and CPU-side bookkeeping.
struct GpuChunk {
    bind_group: wgpu::BindGroup,
    order_buf: wgpu::Buffer,
    /// Chunk-local splat centers, used for depth sorting.
    centers: Vec<[f32; 3]>,
    /// Global cloud indices, used for deterministic decimation.
    globals: Vec<u32>,
    aabb: Aabb,
    /// View-projection the current `order_buf` contents were computed for.
    cached_vp: Option<Mat4>,
    cached_degree: u8,
    /// Number of valid entries in `order_buf` for the cached state.
    visible: u32,
}

/// Camera inputs for one splat pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplatCamera {
    /// World → view.
    pub view: Mat4,
    /// World → clip (`proj * view`).
    pub view_proj: Mat4,
    /// Render target size in pixels.
    pub viewport_px: [f32; 2],
}

/// A drawable Gaussian Splat cloud.
pub struct SplatPipeline {
    pipeline: RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    chunks: Vec<GpuChunk>,
    cloud_degree: u8,
    lod: LodConfig,
    /// Runtime quality cap (SH degree). Defaults to the maximum.
    pub quality: SplatQuality,
}

impl SplatPipeline {
    /// Build the pipeline for the given surface color format.
    pub fn new(device: &Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("splat-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splat-uniforms"),
            size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_entry(1),
                storage_entry(2),
                storage_entry(3),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("splat-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("splat-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Premultiplied "over": src + dst·(1 − src.a).
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                // Test against the opaque scene, but do not write: splats are
                // composited back-to-front and must not hide one another.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        SplatPipeline {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            chunks: Vec::new(),
            cloud_degree: 0,
            lod: LodConfig::default(),
            quality: SplatQuality::default(),
        }
    }

    /// Upload (or replace) the cloud with the default chunking and LOD settings.
    pub fn set_cloud(&mut self, device: &Device, queue: &Queue, cloud: &SplatCloud) {
        self.set_cloud_with(device, queue, cloud, LodConfig::default());
    }

    /// Upload the cloud with explicit chunking and LOD settings.
    pub fn set_cloud_with(
        &mut self,
        device: &Device,
        _queue: &Queue,
        cloud: &SplatCloud,
        lod: LodConfig,
    ) {
        self.lod = lod;
        self.cloud_degree = if cloud.has_sh() { cloud.sh_degree } else { 0 };
        self.chunks.clear();
        for chunk in build_chunks(cloud, &lod) {
            let n = chunk.indices.len();
            let mut gpu = Vec::with_capacity(n);
            let mut sh_flat: Vec<f32> = Vec::with_capacity(n * SH_FLOATS);
            let mut centers = Vec::with_capacity(n);
            for &g in &chunk.indices {
                let s = &cloud.splats[g as usize];
                let c = s.covariance3d();
                gpu.push(SplatGpu {
                    center: [s.position[0], s.position[1], s.position[2], s.opacity],
                    cov_a: [c.x_axis.x, c.x_axis.y, c.x_axis.z, c.y_axis.y],
                    cov_b: [c.y_axis.z, c.z_axis.z, 0.0, 0.0],
                    color: [s.color[0], s.color[1], s.color[2], 0.0],
                });
                centers.push(s.position);
                if cloud.has_sh() {
                    sh_flat.extend_from_slice(&cloud.sh[g as usize]);
                } else {
                    sh_flat.extend(std::iter::repeat_n(0.0f32, SH_FLOATS));
                }
            }

            let splat_buf =
                device.create_buffer_init_compat(bytemuck::cast_slice(&gpu), "splat-data");
            let sh_buf =
                device.create_buffer_init_compat(bytemuck::cast_slice(&sh_flat), "splat-sh");
            let order_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("splat-order"),
                size: (n.max(1) * 4) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("splat-chunk-bg"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: splat_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: sh_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: order_buf.as_entire_binding(),
                    },
                ],
            });
            self.chunks.push(GpuChunk {
                bind_group,
                order_buf,
                centers,
                globals: chunk.indices,
                aabb: chunk.aabb,
                cached_vp: None,
                cached_degree: 0,
                visible: 0,
            });
        }
    }

    /// Number of spatial chunks currently uploaded.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Set the runtime quality (SH cap).
    pub fn set_quality(&mut self, quality: SplatQuality) {
        self.quality = quality;
    }

    /// Record the splat pass into `encoder` for the given camera.
    pub fn render(
        &mut self,
        queue: &Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &TextureView,
        depth_view: &TextureView,
        camera: SplatCamera,
    ) {
        if self.chunks.is_empty() {
            return;
        }
        let SplatCamera {
            view,
            view_proj,
            viewport_px,
        } = camera;
        let degree = self.quality.effective_degree(self.cloud_degree);
        // Focal lengths in pixels from the projection (proj = view_proj · view⁻¹).
        let proj = view_proj * view.inverse();
        let fx = proj.x_axis.x * viewport_px[0] * 0.5;
        let fy = proj.y_axis.y * viewport_px[1] * 0.5;
        let uniforms = Uniforms {
            view: view.to_cols_array_2d(),
            view_proj: view_proj.to_cols_array_2d(),
            viewport: [viewport_px[0], viewport_px[1], fx, fy],
            params: [degree as f32, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let frustum = Frustum::from_view_proj(view_proj);
        let cam = camera_position(view);
        let lod = self.lod;
        let mut draws: Vec<(f32, usize)> = Vec::new();
        for (ci, chunk) in self.chunks.iter_mut().enumerate() {
            if !frustum.intersects_aabb(&chunk.aabb) {
                chunk.visible = 0;
                continue;
            }
            let stale = chunk.cached_vp != Some(view_proj) || chunk.cached_degree != degree;
            if stale {
                let order = visible_order(
                    &chunk.centers,
                    &chunk.globals,
                    &chunk.aabb,
                    cam,
                    view,
                    &frustum,
                    &lod,
                );
                if !order.is_empty() {
                    queue.write_buffer(&chunk.order_buf, 0, bytemuck::cast_slice(&order));
                }
                chunk.visible = order.len() as u32;
                chunk.cached_vp = Some(view_proj);
                chunk.cached_degree = degree;
            }
            if chunk.visible > 0 {
                let c = Vec3::from(chunk.aabb.center());
                let depth = (view * c.extend(1.0)).z;
                draws.push((depth, ci));
            }
        }
        // Back-to-front across chunks (most negative view z first).
        draws.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if draws.is_empty() {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("splat-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        for (_, ci) in draws {
            let chunk = &self.chunks[ci];
            pass.set_bind_group(0, &chunk.bind_group, &[]);
            // Six vertices per instance: one quad per visible splat.
            pass.draw(0..6, 0..chunk.visible);
        }
    }
}

/// `create_buffer_init` helper for storage buffers (no padding needed beyond 4 bytes).
trait DeviceInitExt {
    fn create_buffer_init_compat(&self, contents: &[u8], label: &str) -> wgpu::Buffer;
}

impl DeviceInitExt for Device {
    fn create_buffer_init_compat(&self, contents: &[u8], label: &str) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        // Storage bindings must be non-empty; pad a zero-length upload to 4 bytes.
        let data: &[u8] = if contents.is_empty() {
            &[0u8; 4]
        } else {
            contents
        };
        self.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: data,
            usage: wgpu::BufferUsages::STORAGE,
        })
    }
}

const SHADER: &str = r#"
struct Uniforms {
    view: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
};

struct SplatGpu {
    center: vec4<f32>,
    cov_a: vec4<f32>,
    cov_b: vec4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> splats: array<SplatGpu>;
@group(0) @binding(2) var<storage, read> sh: array<f32>;
@group(0) @binding(3) var<storage, read> order: array<u32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) lambda: vec2<f32>,
};

const CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

fn sh_coef(base: u32, c: u32, k: u32) -> f32 {
    return sh[base + c * 16u + k];
}

// Reference 3DGS real-SH evaluation for one channel (matches sh.rs).
fn sh_channel(deg: u32, d: vec3<f32>, base: u32, c: u32) -> f32 {
    let x = d.x;
    let y = d.y;
    let z = d.z;
    var r = 0.2820948 * sh_coef(base, c, 0u);
    if (deg == 0u) {
        return r;
    }
    r = r - 0.4886025 * y * sh_coef(base, c, 1u)
          + 0.4886025 * z * sh_coef(base, c, 2u)
          - 0.4886025 * x * sh_coef(base, c, 3u);
    if (deg == 1u) {
        return r;
    }
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let yz = y * z;
    let xz = x * z;
    r = r + 1.0925484 * xy * sh_coef(base, c, 4u)
          - 1.0925484 * yz * sh_coef(base, c, 5u)
          + 0.3153916 * (2.0 * zz - xx - yy) * sh_coef(base, c, 6u)
          - 1.0925484 * xz * sh_coef(base, c, 7u)
          + 0.5462742 * (xx - yy) * sh_coef(base, c, 8u);
    if (deg == 2u) {
        return r;
    }
    r = r + -0.5900436 * y * (3.0 * xx - yy) * sh_coef(base, c, 9u)
          + 2.8906114 * xy * z * sh_coef(base, c, 10u)
          - 0.4570458 * y * (4.0 * zz - xx - yy) * sh_coef(base, c, 11u)
          + 0.3731763 * z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * sh_coef(base, c, 12u)
          - 0.4570458 * x * (4.0 * zz - xx - yy) * sh_coef(base, c, 13u)
          + 1.4453057 * z * (xx - yy) * sh_coef(base, c, 14u)
          - 0.5900436 * x * (xx - 3.0 * yy) * sh_coef(base, c, 15u);
    return r;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let li = order[ii];
    let s = splats[li];
    let center = s.center.xyz;

    var out: VsOut;
    out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    out.color = vec4<f32>(0.0);
    out.local = vec2<f32>(0.0);
    out.lambda = vec2<f32>(1.0, 1.0);

    let t = u.view * vec4<f32>(center, 1.0);
    let depth = -t.z;
    if (depth <= 0.2) {
        return out;
    }

    let fx = u.viewport.z;
    let fy = u.viewport.w;
    let vp = u.viewport.xy;
    let w = mat3x3<f32>(u.view[0].xyz, u.view[1].xyz, u.view[2].xyz);
    let cov = mat3x3<f32>(
        vec3<f32>(s.cov_a.x, s.cov_a.y, s.cov_a.z),
        vec3<f32>(s.cov_a.y, s.cov_a.w, s.cov_b.x),
        vec3<f32>(s.cov_a.z, s.cov_b.x, s.cov_b.y),
    );
    // Camera-space covariance and the perspective Jacobian (EWA splatting).
    let vcam = w * cov * transpose(w);
    let j0 = vec3<f32>(fx / depth, 0.0, fx * t.x / (depth * depth));
    let j1 = vec3<f32>(0.0, fy / depth, fy * t.y / (depth * depth));
    let a = dot(j0, vcam * j0) + 0.3;
    let b = dot(j0, vcam * j1);
    let c = dot(j1, vcam * j1) + 0.3;

    // Eigen-decomposition of the 2×2 pixel covariance [[a, b], [b, c]].
    let mid = 0.5 * (a + c);
    let disc = sqrt(max(0.1, mid * mid - (a * c - b * b)));
    let l1 = mid + disc;
    let l2 = max(mid - disc, 0.1);
    var d1: vec2<f32>;
    if (abs(b) > 1e-6) {
        d1 = normalize(vec2<f32>(b, l1 - a));
    } else if (a >= c) {
        d1 = vec2<f32>(1.0, 0.0);
    } else {
        d1 = vec2<f32>(0.0, 1.0);
    }
    let d2 = vec2<f32>(-d1.y, d1.x);
    let r1 = 3.0 * sqrt(l1);
    let r2 = 3.0 * sqrt(l2);

    let corner = CORNERS[vi];
    let offset_px = corner.x * r1 * d1 + corner.y * r2 * d2;
    let clip_c = u.view_proj * vec4<f32>(center, 1.0);
    let ndc_off = 2.0 * offset_px / vp;
    out.clip = vec4<f32>(clip_c.xy + ndc_off * clip_c.w, clip_c.z, clip_c.w);

    var rgb = s.color.xyz;
    let deg = u32(u.params.x);
    if (deg > 0u) {
        let cam = -(transpose(w) * u.view[3].xyz);
        let dir = normalize(center - cam);
        let base = li * 48u;
        rgb = vec3<f32>(
            max(sh_channel(deg, dir, base, 0u) + 0.5, 0.0),
            max(sh_channel(deg, dir, base, 1u) + 0.5, 0.0),
            max(sh_channel(deg, dir, base, 2u) + 0.5, 0.0),
        );
    }
    out.color = vec4<f32>(rgb, s.center.w);
    out.local = vec2<f32>(corner.x * r1, corner.y * r2);
    out.lambda = vec2<f32>(l1, l2);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let power = -0.5 * (in.local.x * in.local.x / in.lambda.x
                      + in.local.y * in.local.y / in.lambda.y);
    if (power > 0.0) {
        discard;
    }
    let alpha = in.color.w * exp(power);
    if (alpha < 1.0 / 255.0) {
        discard;
    }
    // Premultiplied output for the PREMULTIPLIED_ALPHA_BLENDING state.
    return vec4<f32>(in.color.xyz * alpha, alpha);
}
"#;

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::Splat;

    /// Software or hardware adapter, or `None` on machines with no GPU stack.
    fn try_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("splat-test-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            ..Default::default()
        }))
        .ok()
    }

    /// Compiles the shader and pipeline, uploads chunks, draws one red splat in
    /// front of the camera, and checks that its center pixel came out red.
    /// Skips (passes with a note) when no adapter is available.
    #[test]
    fn splat_pipeline_draws_red_splat_at_center() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping splat GPU smoke test: no adapter available");
            return;
        };
        const W: u32 = 64;
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let mut pipeline = SplatPipeline::new(&device, FORMAT);
        let cloud = SplatCloud::new(vec![Splat {
            position: [0.0; 3],
            scale: [0.5; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
            opacity: 1.0,
        }]);
        pipeline.set_cloud(&device, &queue, &cloud);

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("splat-test-color"),
            size: wgpu::Extent3d {
                width: W,
                height: W,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("splat-test-depth"),
            size: wgpu::Extent3d {
                width: W,
                height: W,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());

        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let camera = SplatCamera {
            view,
            view_proj: proj * view,
            viewport_px: [W as f32, W as f32],
        };

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            // Clear color to black and depth to far, so the splat pass has defined input.
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("splat-test-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
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
        }
        pipeline.render(&queue, &mut encoder, &color_view, &depth_view, camera);

        // Read back the full target: rows are 256 bytes, which satisfies COPY alignment.
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splat-test-readback"),
            size: (W * W * 4) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: Some(W),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: W,
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
        let idx = ((W / 2) * W + W / 2) as usize * 4;
        let (r, g) = (data[idx], data[idx + 1]);
        drop(data);
        readback.unmap();
        assert!(
            r > 200 && g < 60,
            "center pixel r={r} g={g}; expected red splat"
        );
    }
}
