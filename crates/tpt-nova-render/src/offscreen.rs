//! Offscreen rendering with GPU readback.
//!
//! [`Renderer`](crate::Renderer) draws to a window surface, so a host that has no
//! window to give it (an appfront viewport, a headless export) cannot receive the
//! frame. [`OffscreenRenderer`] renders the same cube pass into an RGBA8 texture
//! and copies the pixels back to the CPU. It needs no window and no surface.

use glam::Mat4;
use tpt_nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind};
use tpt_nova_ecs::world::World;
use wgpu::util::DeviceExt;

use crate::{build_cube, clear_color, compute_view_proj, Uniforms, Vertex, DEPTH_FORMAT, SHADER};

/// The colour format of rendered frames. Pixels come back as sRGB-encoded RGBA8,
/// row by row from the top, with no padding.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// wgpu requires each row copied to a buffer to be a multiple of this many bytes.
const COPY_ROW_ALIGN: u32 = 256;

/// A rendered frame, row-major RGBA8 with no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// The pixel at `(x, y)`, or `None` if it is outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }
}

/// The colour and depth targets for one frame size.
struct Target {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Target {
            width,
            height,
            color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            color,
        }
    }
}

/// Renders the cube pass of a [`World`] into an offscreen RGBA8 texture and reads it
/// back. Holds its own GPU device, so it does not share state with a [`Renderer`](crate::Renderer).
pub struct OffscreenRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    target: Option<Target>,
}

impl OffscreenRenderer {
    /// Create the device and pipelines. Blocks on GPU setup, so it is native-only:
    /// a browser cannot block (use [`OffscreenRenderer::new_async`] there). Fails if
    /// no GPU adapter is available.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> anyhow::Result<Self> {
        pollster::block_on(Self::new_async())
    }

    /// Create the device and pipelines without blocking. This is the only
    /// constructor on the web.
    pub async fn new_async() -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("offscreen-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            })
            .await?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("offscreen-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("offscreen-uniform-bgl"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("offscreen-pipeline-layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("offscreen-cube-pipeline"),
            layout: Some(&layout),
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
                    format: OFFSCREEN_FORMAT,
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
            label: Some("offscreen-cube-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("offscreen-cube-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Ok(OffscreenRenderer {
            device,
            queue,
            pipeline,
            uniform_layout,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            target: None,
        })
    }

    /// Render `world` at `width` x `height` and return the frame. The first camera
    /// in the world supplies the view. Each cube gets its own uniform buffer, so
    /// every cube keeps its own transform in the same pass.
    pub fn render(&mut self, world: &World, width: u32, height: u32) -> anyhow::Result<Frame> {
        let (width, height) = (width.max(1), height.max(1));
        let needs_new_target = match &self.target {
            Some(t) => t.width != width || t.height != height,
            None => true,
        };
        if needs_new_target {
            self.target = Some(Target::new(&self.device, width, height));
        }
        let target = self.target.as_ref().expect("target was just created");

        let aspect = width as f32 / height as f32;
        let view_proj = world
            .query_2::<Camera, GlobalTransform>()
            .into_iter()
            .next()
            .map(|(_, cam, gt)| compute_view_proj(cam, gt, aspect))
            .unwrap_or(Mat4::IDENTITY);

        // One uniform buffer and bind group per cube, so each draw keeps its own
        // transform. A single shared buffer would make every draw use the last one
        // written, because queue writes take effect at submit.
        let mut draws = Vec::new();
        for (_, mesh, gt) in world.query_2::<Mesh, GlobalTransform>() {
            if mesh.kind != MeshKind::Cube {
                continue;
            }
            let uniforms = Uniforms {
                view_proj: view_proj.to_cols_array_2d(),
                model: gt.0.to_cols_array_2d(),
            };
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("offscreen-uniforms"),
                    contents: bytemuck::cast_slice(&[uniforms]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("offscreen-uniform-bg"),
                layout: &self.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            draws.push(bind_group);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offscreen-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color(world)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
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
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            for bind_group in &draws {
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }

        let row_bytes = width * 4;
        let padded_row = row_bytes.div_ceil(COPY_ROW_ALIGN) * COPY_ROW_ALIGN;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen-readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| anyhow::anyhow!("readback map failed: {e:?}"))?;
        let mut rgba = Vec::with_capacity((row_bytes * height) as usize);
        for row in 0..height {
            let start = (row * padded_row) as usize;
            rgba.extend_from_slice(&mapped[start..start + row_bytes as usize]);
        }
        drop(mapped);
        readback.unmap();

        Ok(Frame {
            width,
            height,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use tpt_nova_ecs::Entity;

    /// A GPU device, or `None` on a machine without one (the test then skips).
    fn renderer() -> Option<OffscreenRenderer> {
        OffscreenRenderer::new().ok()
    }

    /// A world with one camera looking at the origin from `(0, 0, 6)`.
    fn camera_world() -> World {
        let mut world = World::new();
        let cam = world.spawn();
        world.add_component(cam, Camera::default());
        // Camera at (0, 0, 6), no rotation: it looks down -Z at the origin.
        world.add_component(
            cam,
            GlobalTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, 6.0))),
        );
        world
    }

    fn add_cube(world: &mut World, position: Vec3) -> Entity {
        let e = world.spawn();
        world.add_component(
            e,
            Mesh {
                kind: MeshKind::Cube,
            },
        );
        world.add_component(e, GlobalTransform(Mat4::from_translation(position)));
        e
    }

    #[test]
    fn empty_world_reads_back_the_clear_colour() {
        let Some(mut r) = renderer() else { return };
        let world = camera_world();
        let frame = r.render(&world, 32, 32).unwrap();
        assert_eq!((frame.width, frame.height), (32, 32));
        assert_eq!(frame.rgba.len(), 32 * 32 * 4);
        let corner = frame.pixel(0, 0).unwrap();
        assert_eq!(
            corner,
            frame.pixel(31, 31).unwrap(),
            "background is uniform"
        );
        assert_eq!(frame.pixel(32, 0), None);
    }

    #[test]
    fn a_cube_at_the_origin_covers_the_centre() {
        let Some(mut r) = renderer() else { return };
        let mut world = camera_world();
        add_cube(&mut world, Vec3::ZERO);
        let frame = r.render(&world, 64, 64).unwrap();
        let background = frame.pixel(0, 0).unwrap();
        assert_ne!(
            frame.pixel(32, 32).unwrap(),
            background,
            "centre is the cube"
        );
    }

    #[test]
    fn every_cube_keeps_its_own_transform_in_one_pass() {
        let Some(mut r) = renderer() else { return };
        // Two cubes far apart. A shared uniform buffer would draw both at the same
        // place, leaving one of the two spots as background.
        let mut world = camera_world();
        add_cube(&mut world, Vec3::new(-1.5, 0.0, 0.0));
        add_cube(&mut world, Vec3::new(1.5, 0.0, 0.0));
        let frame = r.render(&world, 128, 64).unwrap();
        let background = frame.pixel(0, 0).unwrap();
        // Count non-background pixels in each half: a cube lands in each half.
        let count = |x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..frame.height).map(move |y| (x, y)))
                .filter(|&(x, y)| frame.pixel(x, y).unwrap() != background)
                .count()
        };
        assert!(count(0, 64) > 0, "left cube is drawn");
        assert!(count(64, 128) > 0, "right cube is drawn");
    }

    #[test]
    fn size_changes_rebuild_the_target() {
        let Some(mut r) = renderer() else { return };
        let world = camera_world();
        assert_eq!(r.render(&world, 16, 8).unwrap().rgba.len(), 16 * 8 * 4);
        assert_eq!(r.render(&world, 40, 24).unwrap().rgba.len(), 40 * 24 * 4);
    }

    #[test]
    fn frame_pixel_is_bounds_checked() {
        let f = Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(f.pixel(1, 0), Some([5, 6, 7, 8]));
        assert_eq!(f.pixel(0, 1), None);
    }
}
