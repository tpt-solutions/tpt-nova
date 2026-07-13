//! wgpu rendering for TPT Nova.
//!
//! `nova-render` owns the GPU device, swap-chain surface, and the cube
//! pipeline. It reads entity transforms and camera state from the ECS each
//! frame and draws them — the ECS stays free of all GPU types.

use std::sync::Arc;

use nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind};
use nova_ecs::world::World;
use nova_ecs::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

pub mod pbr;
pub mod sprite;
pub use pbr::PbrRenderer;
pub use sprite::{
    batch_sprites, collect_world_sprites, AtlasRegion, Sprite, SpriteRenderer, SpriteVertex,
    TextureAtlas,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Owns the swap-chain surface, GPU device, and the cube pipeline.
pub struct Renderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
}

impl Renderer {
    /// Initialize the renderer against an existing window.
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
                label: Some("nova-device"),
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
            label: Some("cube-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform-buffer"),
            size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform-bgl"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cube-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cube-pipeline"),
            layout: Some(&pipeline_layout),
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

        let (vertices, indices) = build_cube();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let depth_view = create_depth_view(&device, &config);

        Ok(Renderer {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            uniform_buffer,
            bind_group,
            depth_view,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Resize the swap chain and depth buffer.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
    }

    /// Render one frame from ECS state.
    pub fn render(&mut self, world: &World) -> anyhow::Result<()> {
        // Find the first camera.
        let mut camera_view_proj = None;
        if let Some((_, cam, gt)) = world
            .query_2::<Camera, GlobalTransform>()
            .into_iter()
            .next()
        {
            let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
            camera_view_proj = Some(compute_view_proj(cam, gt, aspect));
        }
        let view_proj = camera_view_proj.unwrap_or(Mat4::IDENTITY);

        // Draw every mesh entity.
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
                label: Some("render-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render-pass"),
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
                    view: &self.depth_view,
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

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

            for (_, mesh, gt) in world.query_2::<Mesh, GlobalTransform>() {
                if mesh.kind != MeshKind::Cube {
                    continue;
                }
                let uniforms = Uniforms {
                    view_proj: view_proj.to_cols_array_2d(),
                    model: gt.0.to_cols_array_2d(),
                };
                self.queue
                    .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(texture);
        Ok(())
    }

    /// The window's current inner size.
    pub fn size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.window.inner_size()
    }
}

/// Build the view-projection matrix for a camera at `gt` with `aspect`.
///
/// Pure (no GPU): `proj * view` where `view = gt⁻¹`. Kept separate from
/// [`Renderer::render`] so the camera math is unit-testable.
pub(crate) fn compute_view_proj(cam: &Camera, gt: &GlobalTransform, aspect: f32) -> Mat4 {
    let mut proj = *cam;
    proj.aspect = aspect;
    let view = gt.0.inverse();
    proj.perspective() * view
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-texture"),
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

/// One cube face: four corner positions plus a shared normal.
type CubeFace = ([f32; 3], [f32; 3], [f32; 3], [f32; 3], [f32; 3]);

fn build_cube() -> (Vec<Vertex>, Vec<u16>) {
    // 6 faces x 4 verts. Normals per face.
    let s = 0.5f32;
    let faces: [CubeFace; 6] = [
        // +X
        (
            [s, -s, -s],
            [s, s, -s],
            [s, s, s],
            [s, -s, s],
            [1.0, 0.0, 0.0],
        ),
        // -X
        (
            [-s, -s, s],
            [-s, s, s],
            [-s, s, -s],
            [-s, -s, -s],
            [-1.0, 0.0, 0.0],
        ),
        // +Y
        (
            [-s, s, -s],
            [-s, s, s],
            [s, s, s],
            [s, s, -s],
            [0.0, 1.0, 0.0],
        ),
        // -Y
        (
            [-s, -s, s],
            [-s, -s, -s],
            [s, -s, -s],
            [s, -s, s],
            [0.0, -1.0, 0.0],
        ),
        // +Z
        (
            [-s, -s, s],
            [s, -s, s],
            [s, s, s],
            [-s, s, s],
            [0.0, 0.0, 1.0],
        ),
        // -Z
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

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.clip_pos = u.view_proj * u.model * vec4<f32>(position, 1.0);
    out.normal = (u.model * vec4<f32>(normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let light = normalize(vec3<f32>(0.5, 0.8, 0.6));
    let diff = max(dot(n, light), 0.0) * 0.8 + 0.2;
    let base = vec3<f32>(0.2, 0.6, 1.0);
    return vec4<f32>(base * diff, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::{Camera, GlobalTransform};
    use nova_ecs::Vec3;

    #[test]
    fn cube_has_correct_topology() {
        let (vertices, indices) = build_cube();
        // 6 faces * 4 verts, 6 faces * 2 tris * 3 indices.
        assert_eq!(vertices.len(), 24);
        assert_eq!(indices.len(), 36);
        // Every index must reference a real vertex.
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
        // Each face's four verts must share the same normal.
        for face in 0..6 {
            let n0 = vertices[face * 4].normal;
            for k in 1..4 {
                assert_eq!(
                    vertices[face * 4 + k].normal,
                    n0,
                    "face {face} normal mismatch"
                );
            }
        }
    }

    #[test]
    fn identity_camera_view_proj_equals_perspective() {
        let mut cam = Camera::default();
        cam.aspect = 1.6;
        let expected = cam.perspective();
        let vp = compute_view_proj(&Camera::default(), &GlobalTransform::identity(), 1.6);
        assert!(
            vp.abs_diff_eq(expected, 1e-5),
            "identity camera view_proj must equal its perspective projection"
        );
    }

    #[test]
    fn camera_translation_offsets_view() {
        // A camera translated +5 on Z should produce a different view_proj than
        // one at the origin, and the world origin should land behind it (negative
        // view-space z, i.e. in front of a camera looking down -Z).
        let at_origin = compute_view_proj(&Camera::default(), &GlobalTransform::identity(), 1.0);
        let mut gt = GlobalTransform::identity();
        gt.0 = Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0));
        let moved = compute_view_proj(&Camera::default(), &gt, 1.0);

        assert!(
            !at_origin.abs_diff_eq(moved, 1e-5),
            "view should change with camera pose"
        );

        // The view matrix is the camera transform's inverse, so the world origin
        // lands at view-space z = -5 (directly in front of a camera at +5 on Z,
        // which looks down -Z). Check the view portion directly.
        let view = gt.0.inverse();
        assert!(
            (view.w_axis.z + 5.0).abs() < 1e-4,
            "origin should sit 5 units in front of the +5 camera, got {}",
            view.w_axis.z
        );
    }
}
