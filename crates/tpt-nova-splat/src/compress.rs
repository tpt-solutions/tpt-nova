//! Quantized, chunked `.compressed.ply` splat format.
//!
//! A full 3DGS capture stores ~248 bytes per Gaussian. This layout packs each
//! splat into 16 bytes by quantizing against per-chunk bounds (256 splats per
//! chunk), in the spirit of the PlayCanvas `compressed.ply` convention:
//!
//! * `element chunk` — one row per chunk with 18 floats: min/max position
//!   (`min_x`…`max_z`), min/max log-scale (`min_scale_x`…`max_scale_z`) and
//!   min/max SH-DC color (`min_r`…`max_b`).
//! * `element vertex` — one row per splat with four `uint` fields:
//!   - `packed_position`: x:11 | y:10 | z:11 bits, lerped inside the chunk bounds.
//!   - `packed_rotation`: 2-bit index of the largest quaternion component (w,x,y,z
//!     order) plus three 10-bit values for the others, each in `±1/√2`; the largest
//!     component is reconstructed as `√(1 − Σ others²)` and is always positive.
//!   - `packed_scale`: x:11 | y:10 | z:11 bits of the log-scale, lerped in the chunk.
//!   - `packed_color`: r:8 | g:8 | b:8 | a:8, where rgb lerp the chunk's SH-DC range
//!     and `a` is opacity.
//!
//! The reader here is validated by round-trip tests against [`encode_compressed_ply`].
//! It has not yet been checked against a third-party-exported `.compressed.ply`
//! asset; field order and bit placement follow the convention above and should be
//! confirmed against a real file before relying on interchange.

use crate::loader::parse_ply_header;
use crate::{normalize_quat, sh_dc_to_linear, Splat, SplatCloud, SplatError};

/// Splats per quantization chunk.
pub const CHUNK_SIZE: usize = 256;

const SH_C0: f32 = 0.282_094_8;
const CHUNK_PROPS: [&str; 18] = [
    "min_x",
    "min_y",
    "min_z",
    "max_x",
    "max_y",
    "max_z",
    "min_scale_x",
    "min_scale_y",
    "min_scale_z",
    "max_scale_x",
    "max_scale_y",
    "max_scale_z",
    "min_r",
    "min_g",
    "min_b",
    "max_r",
    "max_g",
    "max_b",
];

fn lerp(min: f32, max: f32, t: f32) -> f32 {
    min + (max - min) * t
}

/// Quantize `v` into `[min, max]` as an integer in `0..=levels`.
fn quantize(v: f32, min: f32, max: f32, levels: f32) -> u32 {
    let span = max - min;
    if span <= 1e-12 {
        return 0;
    }
    (((v - min) / span).clamp(0.0, 1.0) * levels).round() as u32
}

fn pack_xyz(v: [f32; 3], min: [f32; 3], max: [f32; 3]) -> u32 {
    let x = quantize(v[0], min[0], max[0], 2047.0);
    let y = quantize(v[1], min[1], max[1], 1023.0);
    let z = quantize(v[2], min[2], max[2], 2047.0);
    (x << 21) | (y << 11) | z
}

fn unpack_xyz(p: u32, min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    let x = ((p >> 21) & 0x7ff) as f32 / 2047.0;
    let y = ((p >> 11) & 0x3ff) as f32 / 1023.0;
    let z = (p & 0x7ff) as f32 / 2047.0;
    [
        lerp(min[0], max[0], x),
        lerp(min[1], max[1], y),
        lerp(min[2], max[2], z),
    ]
}

fn pack_rotation(q: [f32; 4]) -> u32 {
    let q = normalize_quat(q);
    let mut largest = 0usize;
    for i in 1..4 {
        if q[i].abs() > q[largest].abs() {
            largest = i;
        }
    }
    // The largest component is reconstructed as positive, so flip the quaternion
    // when it is negative (q and -q are the same rotation).
    let sign = if q[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut bits = 0u32;
    bits |= (largest as u32) << 30;
    let mut shift = 20i32;
    for (i, &v) in q.iter().enumerate() {
        if i == largest {
            continue;
        }
        // Map ±1/√2 onto [-1, 1], then onto 10 bits.
        let t = (sign * v) * std::f32::consts::SQRT_2;
        let b = quantize(t, -1.0, 1.0, 1023.0);
        bits |= b << shift;
        shift -= 10;
    }
    bits
}

fn unpack_rotation(p: u32) -> [f32; 4] {
    let largest = ((p >> 30) & 3) as usize;
    let vals = [
        ((p >> 20) & 0x3ff) as f32,
        ((p >> 10) & 0x3ff) as f32,
        (p & 0x3ff) as f32,
    ]
    .map(|b| (b / 1023.0 * 2.0 - 1.0) * std::f32::consts::FRAC_1_SQRT_2);
    let mut q = [0.0f32; 4];
    let mut k = 0usize;
    for (i, slot) in q.iter_mut().enumerate() {
        if i == largest {
            continue;
        }
        *slot = vals[k];
        k += 1;
    }
    let sum_sq: f32 = vals.iter().map(|v| v * v).sum();
    q[largest] = (1.0 - sum_sq).max(0.0).sqrt();
    normalize_quat(q)
}

/// Encode a cloud into the chunked quantized `.compressed.ply` layout.
///
/// Only the base Gaussian attributes are stored (SH bands above DC are dropped);
/// the DC color is recovered from the linear color the splat carries.
pub fn encode_compressed_ply(cloud: &SplatCloud) -> Result<Vec<u8>, SplatError> {
    if cloud.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    let n = cloud.len();
    let chunk_count = n.div_ceil(CHUNK_SIZE);

    let mut chunk_rows: Vec<[f32; 18]> = Vec::with_capacity(chunk_count);
    let mut vertex_rows: Vec<[u32; 4]> = Vec::with_capacity(n);

    for chunk in cloud.splats.chunks(CHUNK_SIZE) {
        let mut pmin = [f32::INFINITY; 3];
        let mut pmax = [f32::NEG_INFINITY; 3];
        let mut smin = [f32::INFINITY; 3];
        let mut smax = [f32::NEG_INFINITY; 3];
        let mut cmin = [f32::INFINITY; 3];
        let mut cmax = [f32::NEG_INFINITY; 3];
        let dcs: Vec<[f32; 3]> = chunk
            .iter()
            .map(|s| [s.color[0], s.color[1], s.color[2]].map(|c| (c - 0.5) / SH_C0))
            .collect();
        for (s, dc) in chunk.iter().zip(&dcs) {
            for i in 0..3 {
                pmin[i] = pmin[i].min(s.position[i]);
                pmax[i] = pmax[i].max(s.position[i]);
                let ls = s.scale[i].max(1e-12).ln();
                smin[i] = smin[i].min(ls);
                smax[i] = smax[i].max(ls);
                cmin[i] = cmin[i].min(dc[i]);
                cmax[i] = cmax[i].max(dc[i]);
            }
        }
        let mut row = [0.0f32; 18];
        row[0..3].copy_from_slice(&pmin);
        row[3..6].copy_from_slice(&pmax);
        row[6..9].copy_from_slice(&smin);
        row[9..12].copy_from_slice(&smax);
        row[12..15].copy_from_slice(&cmin);
        row[15..18].copy_from_slice(&cmax);
        chunk_rows.push(row);

        for (s, dc) in chunk.iter().zip(&dcs) {
            let ls = [
                s.scale[0].max(1e-12).ln(),
                s.scale[1].max(1e-12).ln(),
                s.scale[2].max(1e-12).ln(),
            ];
            let r = quantize(dc[0], cmin[0], cmax[0], 255.0);
            let g = quantize(dc[1], cmin[1], cmax[1], 255.0);
            let b = quantize(dc[2], cmin[2], cmax[2], 255.0);
            let a = quantize(s.opacity, 0.0, 1.0, 255.0);
            vertex_rows.push([
                pack_xyz(s.position, pmin, pmax),
                pack_rotation(s.rotation),
                pack_xyz(ls, smin, smax),
                (r << 24) | (g << 16) | (b << 8) | a,
            ]);
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"ply\nformat binary_little_endian 1.0\n");
    out.extend_from_slice(format!("element chunk {chunk_count}\n").as_bytes());
    for p in CHUNK_PROPS {
        out.extend_from_slice(format!("property float {p}\n").as_bytes());
    }
    out.extend_from_slice(format!("element vertex {n}\n").as_bytes());
    for p in [
        "packed_position",
        "packed_rotation",
        "packed_scale",
        "packed_color",
    ] {
        out.extend_from_slice(format!("property uint {p}\n").as_bytes());
    }
    out.extend_from_slice(b"end_header\n");
    for row in &chunk_rows {
        for v in row {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for row in &vertex_rows {
        for v in row {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    Ok(out)
}

/// Decode a chunked `.compressed.ply` byte buffer into a [`SplatCloud`].
pub fn load_compressed_ply_bytes(bytes: &[u8]) -> Result<SplatCloud, SplatError> {
    let header = parse_ply_header(bytes)?;
    if header.format != "binary_little_endian" {
        return Err(SplatError::MalformedPly(
            "compressed ply must be binary_little_endian".into(),
        ));
    }
    let chunk = header
        .elements
        .iter()
        .find(|e| e.0 == "chunk")
        .ok_or_else(|| SplatError::MalformedPly("compressed ply has no chunk element".into()))?;
    let vertex = header
        .elements
        .iter()
        .find(|e| e.0 == "vertex")
        .ok_or_else(|| SplatError::MalformedPly("no vertex element".into()))?;
    let n = vertex.1 as usize;
    if n == 0 {
        return Err(SplatError::EmptyCloud);
    }
    let chunk_count = chunk.1 as usize;
    if chunk_count < n.div_ceil(CHUNK_SIZE) {
        return Err(SplatError::MalformedPly(format!(
            "chunk element has {chunk_count} rows, need {}",
            n.div_ceil(CHUNK_SIZE)
        )));
    }

    let body = &bytes[header.body_start..];
    let mut off = 0usize;

    // Chunk rows: read by property name so declaration order does not matter.
    let chunk_index = |name: &str| chunk.2.iter().position(|(_, n)| n == name);
    let chunk_cols: Vec<Option<usize>> = CHUNK_PROPS.iter().map(|p| chunk_index(p)).collect();
    let mut chunks: Vec<[f32; 18]> = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        let mut raw = Vec::with_capacity(chunk.2.len());
        for (ty, _) in &chunk.2 {
            raw.push(ty.read(body, &mut off)? as f32);
        }
        let mut row = [0.0f32; 18];
        for (k, col) in chunk_cols.iter().enumerate() {
            if let Some(c) = col {
                row[k] = raw[*c];
            }
        }
        chunks.push(row);
    }

    // Vertex rows: the four packed uint fields, looked up by name.
    let vertex_index = |name: &str| vertex.2.iter().position(|(_, n)| n == name);
    let (Some(ip), Some(ir), Some(is), Some(ic)) = (
        vertex_index("packed_position"),
        vertex_index("packed_rotation"),
        vertex_index("packed_scale"),
        vertex_index("packed_color"),
    ) else {
        return Err(SplatError::MalformedPly(
            "vertex element is missing packed_* fields".into(),
        ));
    };
    let mut splats = Vec::with_capacity(n);
    for i in 0..n {
        let mut raw = [0u32; 4];
        let mut row_vals = Vec::with_capacity(vertex.2.len());
        for (ty, _) in &vertex.2 {
            row_vals.push(ty.read(body, &mut off)?);
        }
        raw[0] = row_vals[ip] as u32;
        raw[1] = row_vals[ir] as u32;
        raw[2] = row_vals[is] as u32;
        raw[3] = row_vals[ic] as u32;
        let c = &chunks[i / CHUNK_SIZE];
        let position = unpack_xyz(raw[0], [c[0], c[1], c[2]], [c[3], c[4], c[5]]);
        let ls = unpack_xyz(raw[2], [c[6], c[7], c[8]], [c[9], c[10], c[11]]);
        let scale = [ls[0].exp(), ls[1].exp(), ls[2].exp()];
        let rotation = unpack_rotation(raw[1]);
        let rgb_dc = [
            lerp(c[12], c[15], ((raw[3] >> 24) & 0xff) as f32 / 255.0),
            lerp(c[13], c[16], ((raw[3] >> 16) & 0xff) as f32 / 255.0),
            lerp(c[14], c[17], ((raw[3] >> 8) & 0xff) as f32 / 255.0),
        ];
        let opacity = (raw[3] & 0xff) as f32 / 255.0;
        let color = [
            sh_dc_to_linear(rgb_dc[0]),
            sh_dc_to_linear(rgb_dc[1]),
            sh_dc_to_linear(rgb_dc[2]),
            opacity,
        ];
        splats.push(Splat {
            position,
            scale,
            rotation,
            color,
            opacity,
        });
    }
    Ok(SplatCloud::new(splats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud(n: usize) -> SplatCloud {
        let mut splats = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32;
            let angle = t * 0.1;
            let h = angle * 0.5;
            splats.push(Splat {
                position: [t.sin() * 3.0, (t * 0.37).cos() * 2.0, t * 0.01 - 1.0],
                scale: [0.05 + (t * 0.01).abs(), 0.04, 0.03 + (t * 0.002).abs()],
                rotation: normalize_quat([h.cos(), 0.3 * h.sin(), 0.5 * h.sin(), 0.8 * h.sin()]),
                color: [(t * 0.03).sin().abs(), (t * 0.05).cos().abs(), 0.25, 1.0],
                opacity: 0.2 + 0.7 * ((t * 0.07).sin().abs()),
            });
        }
        SplatCloud::new(splats)
    }

    #[test]
    fn round_trip_stays_within_quantization_error() {
        let src = cloud(600); // spans three chunks, last one partial
        let bytes = encode_compressed_ply(&src).unwrap();
        let dec = load_compressed_ply_bytes(&bytes).unwrap();
        assert_eq!(dec.len(), src.len());
        for (a, b) in src.splats.iter().zip(&dec.splats) {
            for i in 0..3 {
                assert!((a.position[i] - b.position[i]).abs() < 0.01, "pos");
                let rel = (a.scale[i] / b.scale[i]).ln().abs();
                assert!(rel < 0.02, "scale rel err {rel}");
            }
            // Quaternions equal up to sign.
            let dot: f32 = (0..4).map(|i| a.rotation[i] * b.rotation[i]).sum();
            assert!(dot.abs() > 0.999, "rotation dot {dot}");
            assert!((a.opacity - b.opacity).abs() < 0.005);
            for c in 0..3 {
                assert!((a.color[c] - b.color[c]).abs() < 0.01, "color");
            }
        }
    }

    #[test]
    fn compressed_is_much_smaller_than_full_precision() {
        let src = cloud(1024);
        let bytes = encode_compressed_ply(&src).unwrap();
        // Vertex rows are 16 bytes; full 3DGS rows would be ~248 bytes.
        assert!(bytes.len() < 1024 * 20, "size {}", bytes.len());
    }

    #[test]
    fn plain_ply_loader_routes_compressed_layout() {
        let bytes = encode_compressed_ply(&cloud(10)).unwrap();
        let dec = crate::load_ply_bytes(&bytes).unwrap();
        assert_eq!(dec.len(), 10);
    }

    #[test]
    fn rejects_empty_and_non_binary_input() {
        assert!(matches!(
            encode_compressed_ply(&SplatCloud::default()),
            Err(SplatError::EmptyCloud)
        ));
        let ascii = b"ply\nformat ascii 1.0\nelement chunk 1\nelement vertex 1\nproperty uint packed_position\nend_header\n0\n";
        assert!(load_compressed_ply_bytes(ascii).is_err());
    }

    #[test]
    fn rotation_packing_round_trips_all_largest_components() {
        for q in [
            [0.9f32, 0.1, 0.2, 0.3],
            [0.1, 0.9, 0.2, 0.3],
            [0.1, 0.2, 0.9, 0.3],
            [-0.1, 0.2, 0.3, -0.9],
        ] {
            let n = normalize_quat(q);
            let back = unpack_rotation(pack_rotation(n));
            let sign = if back[0] * n[0] + back[1] * n[1] + back[2] * n[2] + back[3] * n[3] < 0.0 {
                -1.0
            } else {
                1.0
            };
            for i in 0..4 {
                assert!((sign * back[i] - n[i]).abs() < 2e-3, "{q:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn dc_inversion_matches_decoder_activation() {
        // The encoder's DC inversion must agree with the decoder's activation.
        let lin = 0.8f32;
        let dc = (lin - 0.5) / SH_C0;
        assert!((sh_dc_to_linear(dc) - lin).abs() < 1e-6);
    }
}
