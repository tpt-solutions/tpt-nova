//! wgpu render pipeline for Gaussian Splats.
//!
//! Enabled by the `render` feature. Draws a [`SplatCloud`] as camera-facing
//! billboards: each Gaussian becomes a screen-space quad tinted by its color and
//! sized by its largest std-dev. This is a visually-faithful *approximation* of
//! true anisotropic Gaussian splatting (which integrates the 3×3 covariance
//! through the Jacobian) — adequate for in-engine previews and as the hook
//! point for a full covariance splat shader later.
//!
//! To integrate with `nova-render`, build a [`SplatPipeline`] from the same
//! `device`/`queue`/`surface_format` the renderer owns, call [`SplatPipeline::set_cloud`]
//! whenever the cloud changes, and invoke [`SplatPipeline::render`] inside the
//! renderer's frame after the opaque passes (supplying the camera view-proj).

use nova_ecs::Mat4;
use wgpu::util::DeviceExt;
use wgpu::{Device, Queue, RenderPipeline, TextureView};

use crate::SplatCloud;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    center: [f32; 3],
    _pad0: f32,
    color: [f32; 4],
    scale: f32,
    _pad1: [f32; 3],
}

impl Instance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

const VERTS: &[([f32; 2], u16)] = &[
    ([-1.0, -1.0], 0),
    ([1.0, -1.0], 1),
    ([1.0, 1.0], 2),
    ([-1.0, 1.0], 3),
];
const INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

/// A drawable Gaussian Splat cloud.
pub struct SplatPipeline {
    pipeline: RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: Option<wgpu::Buffer>,
    instance_count: u32,
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
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-bgl"),
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
            label: Some("splat-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let quad: Vec<[f32; 2]> = VERTS.iter().map(|(p, _)| *p).collect();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splat-quad"),
            contents: bytemuck::cast_slice(&quad),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splat-indices"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
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
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        }],
                    }),
                    Some(Instance::layout()),
                ],
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

        SplatPipeline {
            pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            instance_buffer: None,
            instance_count: 0,
        }
    }

    /// Upload (or replace) the instance buffer for `cloud`. Call when the cloud
    /// changes; the GPU buffer is re-created as needed.
    pub fn set_cloud(&mut self, device: &Device, _queue: &Queue, cloud: &SplatCloud) {
        let instances: Vec<Instance> = cloud
            .splats
            .iter()
            .map(|s| Instance {
                center: s.position,
                _pad0: 0.0,
                color: s.color,
                scale: s.max_scale().max(0.01),
                _pad1: [0.0; 3],
            })
            .collect();
        self.instance_count = instances.len() as u32;
        if instances.is_empty() {
            self.instance_buffer = None;
            return;
        }
        let bytes = bytemuck::cast_slice(&instances);
        self.instance_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("splat-instances"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
    }

    /// Record the splat pass into `encoder`, drawing all uploaded instances with
    /// the given camera `view_proj`.
    pub fn render(
        &self,
        queue: &Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &TextureView,
        depth_view: &TextureView,
        view_proj: Mat4,
    ) {
        if self.instance_count == 0 || self.instance_buffer.is_none() {
            return;
        }
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms {
                view_proj: view_proj.to_cols_array_2d(),
            }]),
        );

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
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.as_ref().unwrap().slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..INDICES.len() as u32, 0, 0..self.instance_count);
    }
}

const SHADER: &str = r#"
struct Uniforms { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) center: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) scale: f32,
) -> VsOut {
    let clip_center = u.view_proj * vec4<f32>(center, 1.0);
    // Perspective-correct screen-space billboard: offset in NDC scaled by w so
    // the quad keeps a constant on-screen size.
    let offset = vec4<f32>(corner * scale, 0.0, 0.0) * clip_center.w;
    var out: VsOut;
    out.clip = clip_center + offset;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
