//! 2D UI overlay rendering for TPT Nova.
//!
//! This turns a [`nova_ui::DrawList`] (the framework-agnostic list of colored
//! rectangles the editor produces) into a real GPU pass drawn on top of the 3D
//! scene. The math is a trivial pixel->clip-space orthographic projection, so it
//! is fully decoupled from the ECS; [`build_ui_vertices`] is pure and
//! unit-testable without a device.
//!
//! Text runs in the draw list are intentionally skipped for now: rendering glyphs
//! needs a font atlas / SDF pipeline. Panels, buttons, gizmo handles and the
//! highlight marquee are all rectangles, so the editor is already visible and
//! interactive. A glyph atlas is a follow-up that plugs into the same vertex
//! stream.

use nova_ui::{DrawCommand, DrawList};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiVertex {
    position: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiUniform {
    viewport: [f32; 2],
}

const UI_SHADER: &str = r#"
struct UiUniform { viewport: vec2<f32> };
@group(0) @binding(0) var<uniform> u: UiUniform;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
) -> VsOut {
    var out: VsOut;
    // Pixel coordinates, y-down, to NDC. x in [-1,1]; y is flipped so +y is down.
    let x = (position.x / u.viewport.x) * 2.0 - 1.0;
    let y = 1.0 - (position.y / u.viewport.y) * 2.0;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Convert a UI draw list into GPU vertex/index buffers.
///
/// Only [`DrawCommand::Rect`] commands are emitted (each as two triangles); text
/// runs are ignored until a glyph atlas is wired in. `viewport` is the surface
/// size in pixels, used only for sanity (the actual projection happens in the
/// shader via the uniform). Returns empty buffers when there is nothing to draw.
pub fn build_ui_vertices(draw: &DrawList, _viewport: (u32, u32)) -> (Vec<UiVertex>, Vec<u16>) {
    let mut verts: Vec<UiVertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for cmd in draw {
        let (rect, color) = match cmd {
            DrawCommand::Rect { rect, color, .. } => (rect, color),
            DrawCommand::Text { .. } => continue,
        };
        let min = rect.min;
        let max = rect.max;
        // Skip zero/negative-area rects (degenerate layout).
        if max.x <= min.x || max.y <= min.y {
            continue;
        }
        let c = [color.r, color.g, color.b, color.a];
        let base = verts.len() as u16;
        verts.push(UiVertex {
            position: [min.x, min.y],
            color: c,
        });
        verts.push(UiVertex {
            position: [max.x, min.y],
            color: c,
        });
        verts.push(UiVertex {
            position: [max.x, max.y],
            color: c,
        });
        verts.push(UiVertex {
            position: [min.x, max.y],
            color: c,
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, indices)
}

/// Owns the 2D UI pipeline and draws a [`DrawList`] over an existing color
/// attachment (the 3D scene). Created once and reused every frame; per-frame
/// vertex/index buffers are allocated to the exact draw size.
pub struct UiOverlay {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl UiOverlay {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui-shader"),
            source: wgpu::ShaderSource::Wgsl(UI_SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-uniform"),
            size: std::mem::size_of::<UiUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-bgl"),
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
            label: Some("ui-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        UiOverlay {
            device,
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
        }
    }

    /// Draw `draw` over `color_view` inside `encoder`. No-op when the list is
    /// empty or contains only text. Uses alpha blending so it composites over
    /// the 3D scene.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        draw: &DrawList,
        viewport: (u32, u32),
    ) {
        let (verts, indices) = build_ui_vertices(draw, viewport);
        if verts.is_empty() || indices.is_empty() {
            return;
        }
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[UiUniform {
                viewport: [viewport.0 as f32, viewport.1 as f32],
            }]),
        );

        let vbuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui-vertices"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let ibuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, vbuf.slice(..));
        pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use nova_ui::{Color, DrawCommand, Rect};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> DrawCommand {
        DrawCommand::Rect {
            rect: Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, h)),
            color: Color::rgb(1.0, 0.0, 0.0),
            rounding: 0.0,
        }
    }

    #[test]
    fn builds_two_triangles_per_rect() {
        let draw = vec![rect(0.0, 0.0, 10.0, 20.0), rect(50.0, 60.0, 5.0, 5.0)];
        let (verts, indices) = build_ui_vertices(&draw, (800, 600));
        assert_eq!(verts.len(), 8); // 4 verts per rect * 2
        assert_eq!(indices.len(), 12); // 6 indices per rect * 2
                                       // First rect's four corners.
        assert_eq!(verts[0].position, [0.0, 0.0]);
        assert_eq!(verts[1].position, [10.0, 0.0]);
        assert_eq!(verts[2].position, [10.0, 20.0]);
        assert_eq!(verts[3].position, [0.0, 20.0]);
        // Red color throughout.
        assert_eq!(verts[0].color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn text_commands_are_skipped() {
        let mut draw = vec![rect(0.0, 0.0, 10.0, 10.0)];
        draw.push(DrawCommand::Text {
            pos: Vec2::ZERO,
            text: "hi".into(),
            color: Color::rgb(1.0, 1.0, 1.0),
            size: 16.0,
        });
        let (verts, indices) = build_ui_vertices(&draw, (800, 600));
        assert_eq!(verts.len(), 4);
        assert_eq!(indices.len(), 6);
    }

    #[test]
    fn degenerate_rects_are_dropped() {
        let draw = vec![
            rect(0.0, 0.0, 0.0, 10.0), // zero width
            rect(0.0, 0.0, 10.0, 0.0), // zero height
        ];
        let (verts, indices) = build_ui_vertices(&draw, (800, 600));
        assert!(verts.is_empty());
        assert!(indices.is_empty());
    }
}
