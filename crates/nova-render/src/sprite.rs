//! 2D sprite rendering: a texture atlas, CPU-side quad batching, and a wgpu
//! pipeline that draws all sprites sharing one atlas in a single draw call.
//!
//! The batching and atlas math are pure and unit-tested; the GPU
//! [`SpriteRenderer`] consumes the batched vertex/index buffers. Sprites are
//! positioned from an entity's [`Transform`](nova_ecs::transform::Transform)
//! (its X/Y translation) plus a [`Sprite`] component describing size, atlas
//! region, and tint.

use std::collections::HashMap;

use glam::{Mat4, Vec2};
use nova_ecs::component::Component;
use nova_ecs::transform::Transform;
use nova_ecs::World;
use wgpu::util::DeviceExt;

/// A single sprite vertex uploaded to the GPU.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

impl SpriteVertex {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SpriteVertex>() as wgpu::BufferAddress,
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
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// Screen-space orthographic projection uniform.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteUniforms {
    pub view_proj: [[f32; 4]; 4],
}

/// A named rectangular region packed into a single atlas texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Maps region ids to pixel rectangles inside one texture, and converts them to
/// normalized UV coordinates.
#[derive(Debug, Clone)]
pub struct TextureAtlas {
    pub width: u32,
    pub height: u32,
    regions: HashMap<u32, AtlasRegion>,
}

impl TextureAtlas {
    pub fn new(width: u32, height: u32) -> Self {
        TextureAtlas {
            width: width.max(1),
            height: height.max(1),
            regions: HashMap::new(),
        }
    }

    /// Register (or replace) a region under `id`.
    pub fn insert(&mut self, id: u32, region: AtlasRegion) -> &mut Self {
        self.regions.insert(id, region);
        self
    }

    pub fn region(&self, id: u32) -> Option<AtlasRegion> {
        self.regions.get(&id).copied()
    }

    /// Return the `[u0, v0, u1, v1]` UV rectangle for a region id, or the whole
    /// texture if the id is unknown.
    pub fn uv(&self, id: u32) -> [f32; 4] {
        match self.regions.get(&id) {
            Some(r) => {
                let w = self.width as f32;
                let h = self.height as f32;
                [
                    r.x as f32 / w,
                    r.y as f32 / h,
                    (r.x + r.w) as f32 / w,
                    (r.y + r.h) as f32 / h,
                ]
            }
            None => [0.0, 0.0, 1.0, 1.0],
        }
    }
}

/// A 2D sprite component. Position comes from the entity's `Transform`; this
/// holds the visual parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    /// Atlas region id to sample.
    pub region: u32,
    /// Size in world/screen units.
    pub size: Vec2,
    /// RGBA tint multiplied with the texture.
    pub color: [f32; 4],
    /// Sort key; higher draws on top.
    pub z: f32,
}

impl Default for Sprite {
    fn default() -> Self {
        Sprite {
            region: 0,
            size: Vec2::splat(32.0),
            color: [1.0, 1.0, 1.0, 1.0],
            z: 0.0,
        }
    }
}

impl Component for Sprite {}

/// Push one sprite quad (centered on `center`) into the given buffers.
fn push_quad(
    verts: &mut Vec<SpriteVertex>,
    indices: &mut Vec<u16>,
    center: Vec2,
    sprite: &Sprite,
    atlas: &TextureAtlas,
) {
    let half = sprite.size * 0.5;
    let [u0, v0, u1, v1] = atlas.uv(sprite.region);
    let base = verts.len() as u16;

    // Corners: top-left, top-right, bottom-right, bottom-left (y-down).
    let tl = center + Vec2::new(-half.x, -half.y);
    let tr = center + Vec2::new(half.x, -half.y);
    let br = center + Vec2::new(half.x, half.y);
    let bl = center + Vec2::new(-half.x, half.y);

    verts.push(SpriteVertex {
        pos: tl.into(),
        uv: [u0, v0],
        color: sprite.color,
    });
    verts.push(SpriteVertex {
        pos: tr.into(),
        uv: [u1, v0],
        color: sprite.color,
    });
    verts.push(SpriteVertex {
        pos: br.into(),
        uv: [u1, v1],
        color: sprite.color,
    });
    verts.push(SpriteVertex {
        pos: bl.into(),
        uv: [u0, v1],
        color: sprite.color,
    });

    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Batch a list of positioned sprites into interleaved vertex + index buffers.
///
/// Sprites are drawn back-to-front by their `z` so overlapping alpha blends
/// correctly. All sprites are assumed to share one atlas -> one draw call.
pub fn batch_sprites(
    sprites: &[(Vec2, Sprite)],
    atlas: &TextureAtlas,
) -> (Vec<SpriteVertex>, Vec<u16>) {
    let mut ordered: Vec<&(Vec2, Sprite)> = sprites.iter().collect();
    ordered.sort_by(|a, b| {
        a.1.z
            .partial_cmp(&b.1.z)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut verts = Vec::with_capacity(ordered.len() * 4);
    let mut indices = Vec::with_capacity(ordered.len() * 6);
    for (center, sprite) in ordered {
        push_quad(&mut verts, &mut indices, *center, sprite, atlas);
    }
    (verts, indices)
}

/// Collect all `(Transform, Sprite)` entities into batchable sprite data,
/// using the transform's X/Y as the sprite center.
pub fn collect_world_sprites(world: &World) -> Vec<(Vec2, Sprite)> {
    world
        .query_2::<Transform, Sprite>()
        .into_iter()
        .map(|(_, t, s)| (Vec2::new(t.translation.x, t.translation.y), *s))
        .collect()
}

/// A y-down orthographic projection mapping `(0,0)`..(w,h) pixels to clip space.
pub fn screen_ortho(width: f32, height: f32) -> Mat4 {
    Mat4::orthographic_rh(0.0, width.max(1.0), height.max(1.0), 0.0, -1.0, 1.0)
}

/// WGSL for the sprite pipeline.
pub const SPRITE_SHADER: &str = r#"
struct Uniforms { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(atlas_tex, atlas_samp, in.uv) * in.color;
}
"#;

/// GPU pipeline + growable buffers for drawing batched sprites.
///
/// Usage per frame: [`SpriteRenderer::set_atlas`] once when the texture is
/// known, then [`SpriteRenderer::prepare`] with the batched buffers and screen
/// size, then [`SpriteRenderer::draw`] inside a render pass.
pub struct SpriteRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
}

impl SpriteRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite-shader"),
            source: wgpu::ShaderSource::Wgsl(SPRITE_SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-uniforms"),
            size: std::mem::size_of::<SpriteUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite-uniform-bgl"),
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

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite-uniform-bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sprite-texture-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite-pipeline-layout"),
            bind_group_layouts: &[Some(&uniform_bgl), Some(&texture_bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(SpriteVertex::layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        SpriteRenderer {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            texture_bind_group: None,
            vertex_buffer: None,
            index_buffer: None,
            index_count: 0,
        }
    }

    /// Bind the atlas texture + sampler the sprites sample from.
    pub fn set_atlas(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        self.texture_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite-texture-bg"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }));
    }

    /// Upload batched geometry and the screen projection for this frame.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[SpriteVertex],
        indices: &[u16],
        screen_width: f32,
        screen_height: f32,
    ) {
        let uniforms = SpriteUniforms {
            view_proj: screen_ortho(screen_width, screen_height).to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        self.index_count = indices.len() as u32;
        if vertices.is_empty() || indices.is_empty() {
            return;
        }
        self.vertex_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sprite-vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.index_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sprite-indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );
    }

    /// Record the sprite draw into an active render pass.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let (Some(vb), Some(ib), Some(tex)) = (
            self.vertex_buffer.as_ref(),
            self.index_buffer.as_ref(),
            self.texture_bind_group.as_ref(),
        ) else {
            return;
        };
        if self.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, tex, &[]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_uv_is_normalized() {
        let mut atlas = TextureAtlas::new(100, 200);
        atlas.insert(
            1,
            AtlasRegion {
                x: 10,
                y: 20,
                w: 30,
                h: 40,
            },
        );
        let uv = atlas.uv(1);
        assert!((uv[0] - 0.1).abs() < 1e-6);
        assert!((uv[1] - 0.1).abs() < 1e-6);
        assert!((uv[2] - 0.4).abs() < 1e-6);
        assert!((uv[3] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn unknown_region_uses_full_texture() {
        let atlas = TextureAtlas::new(64, 64);
        assert_eq!(atlas.uv(999), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn batch_produces_four_verts_six_indices_per_sprite() {
        let atlas = TextureAtlas::new(64, 64);
        let sprites = vec![
            (Vec2::new(0.0, 0.0), Sprite::default()),
            (Vec2::new(50.0, 50.0), Sprite::default()),
        ];
        let (v, i) = batch_sprites(&sprites, &atlas);
        assert_eq!(v.len(), 8);
        assert_eq!(i.len(), 12);
        // Second quad indices reference the second vertex block.
        assert_eq!(i[6], 4);
    }

    #[test]
    fn quad_corners_are_centered_on_position() {
        let atlas = TextureAtlas::new(64, 64);
        let s = Sprite {
            size: Vec2::new(10.0, 20.0),
            ..Default::default()
        };
        let (v, _) = batch_sprites(&[(Vec2::new(100.0, 200.0), s)], &atlas);
        // top-left corner
        assert_eq!(v[0].pos, [95.0, 190.0]);
        // bottom-right corner
        assert_eq!(v[2].pos, [105.0, 210.0]);
    }

    #[test]
    fn sprites_are_sorted_back_to_front() {
        let atlas = TextureAtlas::new(64, 64);
        let front = Sprite {
            z: 10.0,
            color: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        };
        let back = Sprite {
            z: -5.0,
            color: [0.0, 1.0, 0.0, 1.0],
            ..Default::default()
        };
        let (v, _) = batch_sprites(&[(Vec2::ZERO, front), (Vec2::ZERO, back)], &atlas);
        // The lower-z (back) sprite is emitted first.
        assert_eq!(v[0].color, [0.0, 1.0, 0.0, 1.0]);
    }
}
