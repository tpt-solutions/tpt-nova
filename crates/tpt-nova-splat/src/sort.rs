//! GPU depth sort for splat clouds (feature `render`).
//!
//! The CPU keeps the per-splat frustum/LOD filtering ([`crate::lod::visible_candidates`])
//! and hands the surviving local indices to [`GpuSorter`], which sorts them
//! back-to-front entirely on the GPU:
//!
//! 1. a **prepare** pass computes a sortable `u32` depth key per candidate from
//!    its view-space `z` (the standard float-bit flip keeps IEEE order),
//! 2. then **eight LSD radix passes** of four bits each (32-bit keys) reorder the
//!    index array, ping-ponging between two storage buffers.
//!
//! Each chunk gets its own small buffer set, so chunks sort independently — which
//! matches the renderer's per-chunk draw order (`SplatPipeline` already keeps
//! chunks back-to-front by their center). The sort is stable: equal keys keep
//! their pre-sort order, so the result is deterministic for a given camera.
//!
//! The passes run as a *single* workgroup of 256 invocations. That keeps the
//! algorithm simple and exact (no multi-workgroup barrier/prefix-sum dance) and
//! is fast enough for the default 4096-splat chunks; the dominant GPU cost is
//! the splat draw itself, not the sort.

use glam::Mat4;
use wgpu::{BindGroup, BindGroupLayout, Buffer, ComputePipeline, Device, Queue};

/// Invocations per workgroup (and elements per stable-scan segment).
const WG: u32 = 256;
/// Bits of the key handled per radix pass (16 buckets).
const RADIX_BITS: u32 = 4;
/// 32-bit keys → 8 passes of 4 bits.
const RADIX_PASSES: u32 = 32 / RADIX_BITS;
/// Uniform-buffer binding offset alignment required by wgpu.
const UNIFORM_STRIDE: u64 = 256;

/// Encode a view-space depth (`z`, negative in front of a right-handed camera)
/// as a `u32` whose numeric order matches `f32` order.
///
/// Sorting keys ascending therefore sorts depths ascending, i.e. the most
/// negative `z` (farthest in front) first — exactly the back-to-front order the
/// premultiplied-alpha splat blend needs. `-0.0` and `0.0` collide, which is
/// harmless for depth ordering.
pub fn depth_key(z: f32) -> u32 {
    let bits = z.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits ^ 0x8000_0000
    }
}

/// GPU resources for one chunk's depth sort.
pub(crate) struct ChunkSort {
    // Held alongside the bind groups so the buffers' lifetimes are explicit
    // (the bind groups already keep them alive; readback uses `keys_a` in tests).
    #[allow(dead_code)]
    keys_a: Buffer,
    #[allow(dead_code)]
    keys_b: Buffer,
    #[allow(dead_code)]
    idx_b: Buffer,
    prepare_bg: BindGroup,
    radix_bgs: Vec<BindGroup>,
    params: Buffer,
    prepare_params: Buffer,
}

/// The compute pipelines + layouts shared by every chunk's sort.
pub(crate) struct GpuSorter {
    prepare: ComputePipeline,
    radix: ComputePipeline,
    prepare_layout: BindGroupLayout,
    radix_layout: BindGroupLayout,
}

impl GpuSorter {
    pub(crate) fn new(device: &Device) -> Self {
        // Separate modules: each has a different `@group(0) @binding(0)`
        // uniform, which a single WGSL module cannot declare twice.
        let prepare_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("splat-sort-prepare-shader"),
            source: wgpu::ShaderSource::Wgsl(PREPARE_SHADER.into()),
        });
        let radix_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("splat-sort-radix-shader"),
            source: wgpu::ShaderSource::Wgsl(RADIX_SHADER.into()),
        });

        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let prepare_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-sort-prepare-bgl"),
            entries: &[
                uniform_entry,
                storage(1, true),  // splats
                storage(2, true),  // candidate indices
                storage(3, false), // keys out
            ],
        });
        let radix_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat-sort-radix-bgl"),
            entries: &[
                uniform_entry,
                storage(1, true),  // keys in
                storage(2, false), // keys out
                storage(3, true),  // indices in
                storage(4, false), // indices out
            ],
        });

        let prepare_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("splat-sort-prepare-pl"),
            bind_group_layouts: &[Some(&prepare_layout)],
            immediate_size: 0,
        });
        let radix_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("splat-sort-radix-pl"),
            bind_group_layouts: &[Some(&radix_layout)],
            immediate_size: 0,
        });

        let prepare = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("splat-sort-prepare"),
            layout: Some(&prepare_pl),
            module: &prepare_shader,
            entry_point: Some("cs_prepare"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let radix = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("splat-sort-radix"),
            layout: Some(&radix_pl),
            module: &radix_shader,
            entry_point: Some("cs_radix"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        GpuSorter {
            prepare,
            radix,
            prepare_layout,
            radix_layout,
        }
    }

    /// Allocate the per-chunk sort buffers and bind groups. `capacity` is the
    /// largest number of candidates the chunk can produce.
    pub(crate) fn new_chunk(
        &self,
        device: &Device,
        splats: &Buffer,
        order_buf: &Buffer,
        capacity: usize,
    ) -> ChunkSort {
        let bytes = (capacity.max(1) * 4) as u64;
        let storage_usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let make = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: storage_usage,
                mapped_at_creation: false,
            })
        };
        let keys_a = make("splat-sort-keys-a");
        let keys_b = make("splat-sort-keys-b");
        let idx_b = make("splat-sort-idx-b");

        let prepare_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splat-sort-prepare-params"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splat-sort-params"),
            size: UNIFORM_STRIDE * RADIX_PASSES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let prepare_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splat-sort-prepare-bg"),
            layout: &self.prepare_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: prepare_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: splats.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: order_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: keys_a.as_entire_binding(),
                },
            ],
        });

        // Pass `p` reads the buffer written by pass `p - 1`; even passes read A.
        let mut radix_bgs = Vec::with_capacity(RADIX_PASSES as usize);
        for pass in 0..RADIX_PASSES {
            let (keys_in, keys_out, idx_in, idx_out) = if pass % 2 == 0 {
                (&keys_a, &keys_b, order_buf, &idx_b)
            } else {
                (&keys_b, &keys_a, &idx_b, order_buf)
            };
            let uniform = wgpu::BufferBinding {
                buffer: &params,
                offset: pass as u64 * UNIFORM_STRIDE,
                size: None,
            };
            radix_bgs.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("splat-sort-radix-bg"),
                layout: &self.radix_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(uniform),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: keys_in.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: keys_out.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: idx_in.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: idx_out.as_entire_binding(),
                    },
                ],
            }));
        }

        ChunkSort {
            keys_a,
            keys_b,
            idx_b,
            prepare_bg,
            radix_bgs,
            params,
            prepare_params,
        }
    }

    /// Upload this frame's sort parameters and record the prepare + radix passes.
    ///
    /// `candidates` must already be written into the chunk's `order_buf`; after
    /// this call the same buffer holds the sorted (back-to-front) order.
    pub(crate) fn encode(
        &self,
        queue: &Queue,
        encoder: &mut wgpu::CommandEncoder,
        chunk: &ChunkSort,
        view: Mat4,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        let mut prepare = [0u8; 80];
        let view_cols = view.to_cols_array_2d();
        prepare[..64].copy_from_slice(bytemuck::cast_slice(&view_cols));
        prepare[64..68].copy_from_slice(&count.to_le_bytes());
        queue.write_buffer(&chunk.prepare_params, 0, &prepare);

        let mut params = vec![0u8; (UNIFORM_STRIDE * RADIX_PASSES as u64) as usize];
        for pass in 0..RADIX_PASSES {
            let at = pass as usize * UNIFORM_STRIDE as usize;
            params[at..at + 4].copy_from_slice(&count.to_le_bytes());
            params[at + 4..at + 8].copy_from_slice(&(pass * RADIX_BITS).to_le_bytes());
        }
        queue.write_buffer(&chunk.params, 0, &params);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("splat-sort-prepare-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.prepare);
            pass.set_bind_group(0, &chunk.prepare_bg, &[]);
            pass.dispatch_workgroups(count.div_ceil(WG), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("splat-sort-radix-passes"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.radix);
            for bg in &chunk.radix_bgs {
                pass.set_bind_group(0, bg, &[]);
                // A single workgroup: the shader loops over segments itself.
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
    }
}

const PREPARE_SHADER: &str = r#"
struct SplatGpu {
    center: vec4<f32>,
    cov_a: vec4<f32>,
    cov_b: vec4<f32>,
    color: vec4<f32>,
};

struct PrepareParams {
    view: mat4x4<f32>,
    // x = candidate count.
    counts: vec4<u32>,
};

@group(0) @binding(0) var<uniform> prepare_params: PrepareParams;
@group(0) @binding(1) var<storage, read> splats: array<SplatGpu>;
@group(0) @binding(2) var<storage, read> candidates: array<u32>;
@group(0) @binding(3) var<storage, read_write> keys_out: array<u32>;

@compute @workgroup_size(256)
fn cs_prepare(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = prepare_params.counts.x;
    if (i >= n) {
        return;
    }
    let center = splats[candidates[i]].center.xyz;
    let z = (prepare_params.view * vec4<f32>(center, 1.0)).z;
    // Flip the sign bit (and invert negatives) so u32 order == f32 order.
    let bits = bitcast<u32>(z);
    var key = bits ^ 0x80000000u;
    if ((bits & 0x80000000u) != 0u) {
        key = ~bits;
    }
    keys_out[i] = key;
}
"#;

const RADIX_SHADER: &str = r#"
struct RadixParams {
    // x = element count, y = bit shift for this pass.
    counts: vec4<u32>,
};

const WG: u32 = 256u;
const BUCKETS: u32 = 16u;

var<workgroup> digits: array<u32, 256>;
var<workgroup> hist: array<atomic<u32>, 16>;
// Running write cursor per bucket (starts at the bucket's prefix-sum base).
var<workgroup> bin_cur: array<u32, 16>;
var<workgroup> seg_cnt: array<u32, 16>;

@group(0) @binding(0) var<uniform> radix_params: RadixParams;
@group(0) @binding(1) var<storage, read> keys_in: array<u32>;
@group(0) @binding(2) var<storage, read_write> keys_scratch: array<u32>;
@group(0) @binding(3) var<storage, read> idx_in: array<u32>;
@group(0) @binding(4) var<storage, read_write> idx_out: array<u32>;

// One stable LSD radix pass over the chunk, in a single workgroup.
@compute @workgroup_size(256)
fn cs_radix(@builtin(local_invocation_id) lid: vec3<u32>) {
    let t = lid.x;
    let n = radix_params.counts.x;
    let shift = radix_params.counts.y;

    // 1. Histogram the whole chunk.
    if (t < BUCKETS) {
        atomicStore(&hist[t], 0u);
    }
    workgroupBarrier();
    for (var i = t; i < n; i = i + WG) {
        let d = (keys_in[i] >> shift) & 15u;
        atomicAdd(&hist[d], 1u);
    }
    workgroupBarrier();

    // 2. Exclusive prefix sum over the 16 buckets (cheap enough to be serial).
    if (t == 0u) {
        var acc = 0u;
        for (var b = 0u; b < BUCKETS; b = b + 1u) {
            let c = atomicLoad(&hist[b]);
            bin_cur[b] = acc;
            acc = acc + c;
        }
    }
    workgroupBarrier();

    // 3. Stable scatter, one 256-element segment at a time.
    var base = 0u;
    while (base < n) {
        let seg = min(WG, n - base);
        var d = BUCKETS;
        if (t < seg) {
            d = (keys_in[base + t] >> shift) & 15u;
        }
        digits[t] = d;
        workgroupBarrier();

        // Stable rank: how many earlier elements in this segment share our digit.
        var rank = 0u;
        if (t < seg) {
            for (var j = 0u; j < t; j = j + 1u) {
                if (digits[j] == d) {
                    rank = rank + 1u;
                }
            }
        }
        workgroupBarrier();

        if (t < seg) {
            let out_at = bin_cur[d] + rank;
            idx_out[out_at] = idx_in[base + t];
            keys_scratch[out_at] = keys_in[base + t];
        }
        workgroupBarrier();

        // Advance each bucket's cursor by its count in this segment.
        if (t < BUCKETS) {
            var c = 0u;
            for (var j = 0u; j < seg; j = j + 1u) {
                if (digits[j] == t) {
                    c = c + 1u;
                }
            }
            seg_cnt[t] = c;
        }
        workgroupBarrier();
        if (t < BUCKETS) {
            bin_cur[t] = bin_cur[t] + seg_cnt[t];
        }
        workgroupBarrier();
        base = base + seg;
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_keys_preserve_float_order() {
        let depths = [-100.0, -5.0, -0.5, -0.0, 0.0, 0.5, 5.0, 100.0];
        let keys: Vec<u32> = depths.iter().map(|&d| depth_key(d)).collect();
        for pair in keys.windows(2) {
            assert!(pair[0] <= pair[1], "keys not monotonic: {keys:?}");
        }
    }

    #[test]
    fn closest_splat_has_largest_key() {
        // Back-to-front draw = ascending key, so the farthest splat must sort first.
        let near = depth_key(-1.0);
        let far = depth_key(-20.0);
        assert!(far < near);
    }
}
