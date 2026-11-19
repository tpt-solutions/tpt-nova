//! Loaders for Gaussian Splat captures.
//!
//! Three on-disk layouts are supported:
//!
//! * `.splat` — the compact 32-byte-per-Gaussian layout popularized by
//!   antimatter15's viewer ([`load_splat_bytes`]).
//! * `.ply` — the standard 3D Gaussian Splatting export, which stores one
//!   `vertex` element whose properties carry the raw (pre-activation) Gaussian
//!   parameters ([`load_ply_bytes`]). Both ASCII and `binary_little_endian`
//!   PLY are accepted. Higher-order SH bands (`f_rest_*`) are read when present.
//! * `.compressed.ply` — the quantized, chunked layout (see [`crate::compress`]).
//!   [`load_ply_bytes`] detects it automatically from its `packed_position`
//!   vertex property, so `.ply` callers need not special-case it.

use std::path::Path;

use crate::sh::{self, SH_FLOATS};
use crate::{normalize_quat, sh_dc_to_linear, sigmoid, Splat, SplatCloud, SplatError};

/// Load a splat cloud from a file, dispatching on the extension.
pub fn load_file<P: AsRef<Path>>(path: P) -> Result<SplatCloud, SplatError> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("splat") => load_splat_bytes(&std::fs::read(path)?),
        Some("ply") if name.ends_with(".compressed.ply") => {
            crate::compress::load_compressed_ply_bytes(&std::fs::read(path)?)
        }
        Some("ply") => load_ply_bytes(&std::fs::read(path)?),
        _ => Err(SplatError::UnknownExtension),
    }
}

// ---- .splat ---------------------------------------------------------------

/// Decode a `.splat` byte buffer (32 bytes per Gaussian, row-major).
pub fn load_splat_bytes(bytes: &[u8]) -> Result<SplatCloud, SplatError> {
    if !bytes.len().is_multiple_of(32) {
        return Err(SplatError::MalformedSplat(format!(
            "file length {} is not a multiple of 32",
            bytes.len()
        )));
    }
    let count = bytes.len() / 32;
    let mut splats = Vec::with_capacity(count);
    for i in 0..count {
        let rec = &bytes[i * 32..i * 32 + 32];
        splats.push(Splat::from_splat_bytes(rec)?);
    }
    if splats.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    Ok(SplatCloud::new(splats))
}

// ---- .ply -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlyType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl PlyType {
    pub(crate) fn size(self) -> usize {
        match self {
            PlyType::I8 | PlyType::U8 => 1,
            PlyType::I16 | PlyType::U16 => 2,
            PlyType::I32 | PlyType::U32 | PlyType::F32 => 4,
            PlyType::F64 => 8,
        }
    }

    /// Decode one scalar of this type from `buf` at offset, advancing `offset`.
    /// Unsigned 8-bit values are normalized to `0..=1` (they are used as colors);
    /// every other type passes through unchanged.
    pub(crate) fn read(self, buf: &[u8], offset: &mut usize) -> Result<f64, SplatError> {
        let sz = self.size();
        if *offset + sz > buf.len() {
            return Err(SplatError::MalformedPly("unexpected end of data".into()));
        }
        let slice = &buf[*offset..*offset + sz];
        *offset += sz;
        let v = match self {
            PlyType::I8 => i8::from_le_bytes([slice[0]]) as f64,
            PlyType::U8 => slice[0] as f64 / 255.0,
            PlyType::I16 => i16::from_le_bytes([slice[0], slice[1]]) as f64,
            PlyType::U16 => u16::from_le_bytes([slice[0], slice[1]]) as f64,
            PlyType::I32 => i32::from_le_bytes(slice.try_into().unwrap()) as f64,
            PlyType::U32 => u32::from_le_bytes(slice.try_into().unwrap()) as f64,
            PlyType::F32 => f32::from_le_bytes(slice.try_into().unwrap()) as f64,
            PlyType::F64 => f64::from_le_bytes(slice.try_into().unwrap()),
        };
        Ok(v)
    }
}

/// Parse a PLY scalar `property <type> <name>` header line.
fn parse_property(line: &str) -> Option<(PlyType, String)> {
    let mut parts = line.split_whitespace();
    let _ = parts.next()?; // "property"
    let ty = match parts.next()? {
        "char" | "int8" => PlyType::I8,
        "uchar" | "uint8" => PlyType::U8,
        "short" | "int16" => PlyType::I16,
        "ushort" | "uint16" => PlyType::U16,
        "int" | "int32" => PlyType::I32,
        "uint" | "uint32" => PlyType::U32,
        "float" | "float32" => PlyType::F32,
        "double" | "float64" => PlyType::F64,
        _ => return None,
    };
    let name = parts.next()?.to_string();
    Some((ty, name))
}

/// One PLY element: name, declared row count, and its typed properties.
pub(crate) type PlyElement = (String, u64, Vec<(PlyType, String)>);

/// A parsed PLY header plus the byte offset where the body starts.
pub(crate) struct PlyHeader {
    pub format: String,
    pub elements: Vec<PlyElement>,
    pub body_start: usize,
}

/// Parse the ASCII header of a PLY file. Only the header is decoded as text, so
/// arbitrary binary bodies never need to be valid UTF-8.
pub(crate) fn parse_ply_header(bytes: &[u8]) -> Result<PlyHeader, SplatError> {
    let marker = b"end_header";
    let marker_pos = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .ok_or_else(|| SplatError::MalformedPly("missing end_header".into()))?;
    let after = &bytes[marker_pos + marker.len()..];
    let newline_off = after.iter().position(|b| *b == b'\n').unwrap_or(0);
    let body_start = marker_pos + marker.len() + newline_off + 1;

    let header_str = std::str::from_utf8(&bytes[..body_start.saturating_sub(1)])
        .map_err(|_| SplatError::MalformedPly("header is not valid UTF-8".into()))?;

    let mut format = "ascii".to_string();
    let mut elements: Vec<PlyElement> = Vec::new();
    let mut current: Option<PlyElement> = None;

    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed == "end_header" {
            if let Some(c) = current.take() {
                elements.push(c);
            }
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("format ") {
            format = rest
                .split_whitespace()
                .next()
                .unwrap_or("ascii")
                .to_string();
        } else if let Some(rest) = trimmed.strip_prefix("element ") {
            if let Some(c) = current.take() {
                elements.push(c);
            }
            let mut it = rest.split_whitespace();
            let name = it.next().unwrap_or_default().to_string();
            let count = it.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            current = Some((name, count, Vec::new()));
        } else if trimmed.starts_with("property ") {
            if let Some((ty, name)) = parse_property(trimmed) {
                if let Some(c) = current.as_mut() {
                    c.2.push((ty, name));
                }
            }
        }
    }
    Ok(PlyHeader {
        format,
        elements,
        body_start,
    })
}

/// Decode a `.ply` byte buffer into a [`SplatCloud`].
///
/// Only the `vertex` element is interpreted; any subsequent elements (e.g.
/// `camera`, `edge`) are skipped. Properties beyond the recognized Gaussian
/// set are tolerated and ignored. Files using the quantized layout are routed
/// to [`crate::compress`] automatically.
pub fn load_ply_bytes(bytes: &[u8]) -> Result<SplatCloud, SplatError> {
    let header = parse_ply_header(bytes)?;
    let vertex = header
        .elements
        .iter()
        .find(|e| e.0 == "vertex")
        .ok_or_else(|| SplatError::MalformedPly("no vertex element".into()))?;
    if vertex.2.iter().any(|(_, n)| n == "packed_position") {
        return crate::compress::load_compressed_ply_bytes(bytes);
    }

    let format = header.format.as_str();
    let body_start = header.body_start;
    let count = vertex.1 as usize;
    if count == 0 {
        return Err(SplatError::EmptyCloud);
    }

    // Map property names to column indices.
    let cols: std::collections::HashMap<String, usize> = vertex
        .2
        .iter()
        .enumerate()
        .map(|(i, (_, n))| (n.clone(), i))
        .collect();

    // Higher-order SH: derive the degree from how many f_rest_* properties exist.
    let rest_count = vertex
        .2
        .iter()
        .filter(|(_, n)| n.starts_with("f_rest_"))
        .count();
    let sh_degree = sh::degree_from_rest_count(rest_count).unwrap_or(0);
    let rest_per_channel = sh::rest_coeffs_per_channel(sh_degree);

    let mut splats = Vec::with_capacity(count);
    let mut sh_blocks: Vec<[f32; SH_FLOATS]> =
        Vec::with_capacity(if sh_degree > 0 { count } else { 0 });

    let mut push_row = |row: &[f64]| {
        splats.push(decode_vertex(&cols, row));
        if sh_degree > 0 {
            sh_blocks.push(decode_sh(&cols, row, sh_degree, rest_per_channel));
        }
    };

    if format == "ascii" {
        let ascii_body = std::str::from_utf8(&bytes[body_start..])
            .map_err(|_| SplatError::MalformedPly("ascii body not UTF-8".into()))?;
        let mut rows_seen = 0usize;
        for line in ascii_body.lines().take(count) {
            let values: Vec<f64> = line
                .split_whitespace()
                .map(|t| t.parse::<f64>().unwrap_or(0.0))
                .collect();
            if values.len() < vertex.2.len() {
                return Err(SplatError::MalformedPly(
                    "vertex row has too few values".into(),
                ));
            }
            push_row(&values);
            rows_seen += 1;
        }
        if rows_seen != count {
            return Err(SplatError::MalformedPly(
                "truncated ascii vertex rows".into(),
            ));
        }
    } else if format == "binary_little_endian" {
        let rest = &bytes[body_start..];
        let mut offset = 0usize;
        for _ in 0..count {
            let mut row = Vec::with_capacity(vertex.2.len());
            for (ty, _) in &vertex.2 {
                row.push(ty.read(rest, &mut offset)?);
            }
            push_row(&row);
        }
        // Skip any subsequent elements' binary data so we stay consistent with
        // the element counts declared in the header.
        for el in header.elements.iter().skip(1) {
            if el.0 == "vertex" {
                continue;
            }
            for _ in 0..el.1 {
                for (ty, _) in &el.2 {
                    ty.read(rest, &mut offset)?;
                }
            }
        }
    } else {
        return Err(SplatError::MalformedPly(format!(
            "unsupported PLY format: {format}"
        )));
    }

    if splats.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    if sh_degree > 0 {
        return SplatCloud::with_sh(splats, sh_blocks, sh_degree);
    }
    Ok(SplatCloud::new(splats))
}

/// Gather one vertex's SH block from `f_dc_*` and `f_rest_*` columns.
///
/// `f_rest` is channel-major in 3DGS exports: the first `rest_per_channel`
/// values are red's higher bands, the next block green's, then blue's.
fn decode_sh(
    cols: &std::collections::HashMap<String, usize>,
    row: &[f64],
    degree: u8,
    rest_per_channel: usize,
) -> [f32; SH_FLOATS] {
    let dc = [
        lookup_column(cols, row, "f_dc_0").unwrap_or(0.0) as f32,
        lookup_column(cols, row, "f_dc_1").unwrap_or(0.0) as f32,
        lookup_column(cols, row, "f_dc_2").unwrap_or(0.0) as f32,
    ];
    let total = 3 * rest_per_channel;
    let mut rest = Vec::with_capacity(total);
    for i in 0..total {
        let v = lookup_column(cols, row, &format!("f_rest_{i}")).unwrap_or(0.0);
        rest.push(v as f32);
    }
    sh::pack_sh(dc, &rest, degree)
}

/// Look up a named property's value from a decoded row by column index.
fn lookup_column(
    cols: &std::collections::HashMap<String, usize>,
    row: &[f64],
    name: &str,
) -> Option<f64> {
    cols.get(name).and_then(|&i| row.get(i).copied())
}

/// Turn one decoded PLY vertex row into a [`Splat`], applying 3DGS activations
/// when the canonical property names are present.
fn decode_vertex(cols: &std::collections::HashMap<String, usize>, row: &[f64]) -> Splat {
    let get = lookup_column;
    let x = lookup_column(cols, row, "x").unwrap_or(0.0);
    let y = lookup_column(cols, row, "y").unwrap_or(0.0);
    let z = lookup_column(cols, row, "z").unwrap_or(0.0);

    // Color: prefer 3DGS SH DC bands; fall back to raw RGB bytes.
    let (r, g, b) = if get(cols, row, "f_dc_0").is_some() {
        (
            sh_dc_to_linear(get(cols, row, "f_dc_0").unwrap_or(0.0) as f32),
            sh_dc_to_linear(get(cols, row, "f_dc_1").unwrap_or(0.0) as f32),
            sh_dc_to_linear(get(cols, row, "f_dc_2").unwrap_or(0.0) as f32),
        )
    } else {
        let rv = get(cols, row, "red")
            .or_else(|| get(cols, row, "r"))
            .unwrap_or(0.5);
        let gv = get(cols, row, "green")
            .or_else(|| get(cols, row, "g"))
            .unwrap_or(0.5);
        let bv = get(cols, row, "blue")
            .or_else(|| get(cols, row, "b"))
            .unwrap_or(0.5);
        (rv as f32, gv as f32, bv as f32)
    };

    // Opacity.
    let opacity = if let Some(o) = get(cols, row, "opacity") {
        sigmoid(o as f32)
    } else if let Some(a) = get(cols, row, "alpha") {
        (a as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // Scale: 3DGS stores `ln` scale; generic captures may store a `radius`.
    let scale = if get(cols, row, "scale_0").is_some() {
        [
            get(cols, row, "scale_0").unwrap_or(0.0).exp(),
            get(cols, row, "scale_1").unwrap_or(0.0).exp(),
            get(cols, row, "scale_2").unwrap_or(0.0).exp(),
        ]
    } else if get(cols, row, "scale_x").is_some() {
        [
            get(cols, row, "scale_x").unwrap_or(0.0).exp(),
            get(cols, row, "scale_y").unwrap_or(0.0).exp(),
            get(cols, row, "scale_z").unwrap_or(0.0).exp(),
        ]
    } else {
        let r = get(cols, row, "radius").unwrap_or(0.01);
        [r, r, r]
    };

    // Rotation quaternion (w, x, y, z).
    let rotation = if get(cols, row, "rot_0").is_some() {
        normalize_quat([
            get(cols, row, "rot_0").unwrap_or(0.0) as f32,
            get(cols, row, "rot_1").unwrap_or(0.0) as f32,
            get(cols, row, "rot_2").unwrap_or(0.0) as f32,
            get(cols, row, "rot_3").unwrap_or(0.0) as f32,
        ])
    } else {
        [1.0, 0.0, 0.0, 0.0]
    };

    Splat {
        position: [x as f32, y as f32, z as f32],
        scale: [scale[0] as f32, scale[1] as f32, scale[2] as f32],
        rotation,
        color: [r, g, b, opacity],
        opacity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_splat() -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        b[4..8].copy_from_slice(&0.0f32.to_le_bytes());
        b[8..12].copy_from_slice(&0.0f32.to_le_bytes());
        // scale_0..2 = ln(2) so the scale decodes to 2.
        let ln2 = 2.0f32.ln();
        b[12..16].copy_from_slice(&ln2.to_le_bytes());
        b[16..20].copy_from_slice(&ln2.to_le_bytes());
        b[20..24].copy_from_slice(&ln2.to_le_bytes());
        b[24] = 255;
        b[25] = 0;
        b[26] = 128;
        b[27] = 255;
        b[28] = 255;
        b[29] = 128;
        b[30] = 128;
        b[31] = 128;
        b
    }

    #[test]
    fn loads_splat_bytes() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&single_splat());
        buf.extend_from_slice(&single_splat());
        let cloud = load_splat_bytes(&buf).unwrap();
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.splats[0].position, [1.0, 0.0, 0.0]);
        assert!((cloud.splats[0].scale[0] - 2.0).abs() < 1e-4);
        assert!(!cloud.has_sh());
    }

    #[test]
    fn rejects_misaligned_splat_file() {
        let buf = [0u8; 33];
        assert!(load_splat_bytes(&buf).is_err());
    }

    #[test]
    fn empty_splat_buffer_errors() {
        assert!(matches!(load_splat_bytes(&[]), Err(SplatError::EmptyCloud)));
    }

    #[test]
    fn loads_ascii_ply_with_3dgs_properties() {
        let header = "\
ply
format ascii 1.0
element vertex 2
property float x
property float y
property float z
property float f_dc_0
property float f_dc_1
property float f_dc_2
property float opacity
property float scale_0
property float scale_1
property float scale_2
property float rot_0
property float rot_1
property float rot_2
property float rot_3
end_header
";
        // Vertex 0: at origin, gray, opacity raw 0 (-> sigmoid 0.5), scale ln(1)=0,
        // identity quaternion. Vertex 1: offset +5 in z.
        let body = "\
0 0 0 0 0 0 0 0 0 0 0 1 0 0 0
0 0 5 0 0 0 0 0 0 0 0 1 0 0 0
";
        let buf = format!("{header}{body}").into_bytes();
        let cloud = load_ply_bytes(&buf).unwrap();
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.splats[0].position, [0.0, 0.0, 0.0]);
        assert_eq!(cloud.splats[1].position, [0.0, 0.0, 5.0]);
        // sigmoid(0) = 0.5.
        assert!((cloud.splats[0].opacity - 0.5).abs() < 1e-4);
        // f_dc_0 = 0 -> sh_dc_to_linear(0) = 0.5 gray.
        assert!((cloud.splats[0].color[0] - 0.5).abs() < 1e-4);
        assert!(!cloud.has_sh());
    }

    #[test]
    fn ascii_ply_with_f_rest_attaches_sh_degree_one() {
        // Three f_rest per channel (degree 1) -> 9 f_rest properties.
        let mut header = String::from(
            "ply\nformat ascii 1.0\nelement vertex 1\n\
             property float x\nproperty float y\nproperty float z\n\
             property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\n",
        );
        for i in 0..9 {
            header.push_str(&format!("property float f_rest_{i}\n"));
        }
        header.push_str("property float opacity\nend_header\n");
        // dc = (0.5, 0, 0); red's first higher band = 0.25 (f_rest_0).
        let mut row = String::from("0 0 0 0.5 0 0");
        row.push_str(" 0.25 0 0 0 0 0 0 0 0");
        row.push_str(" 0\n");
        let buf = format!("{header}{row}").into_bytes();
        let cloud = load_ply_bytes(&buf).unwrap();
        assert_eq!(cloud.sh_degree, 1);
        assert!(cloud.has_sh());
        assert!((cloud.sh[0][0] - 0.5).abs() < 1e-6, "raw DC kept");
        assert!(
            (cloud.sh[0][1] - 0.25).abs() < 1e-6,
            "red band 1 is f_rest_0"
        );
    }

    #[test]
    fn loads_binary_little_endian_ply() {
        // Minimal binary PLY with just x,y,z + red/green/blue (uchar).
        let mut buf = Vec::new();
        buf.extend_from_slice(b"ply\n");
        buf.extend_from_slice(b"format binary_little_endian 1.0\n");
        buf.extend_from_slice(b"element vertex 2\n");
        buf.extend_from_slice(b"property float x\n");
        buf.extend_from_slice(b"property float y\n");
        buf.extend_from_slice(b"property float z\n");
        buf.extend_from_slice(b"property uchar red\n");
        buf.extend_from_slice(b"property uchar green\n");
        buf.extend_from_slice(b"property uchar blue\n");
        buf.extend_from_slice(b"end_header\n");
        for v in [
            [0.0f32, 0.0, 0.0, 255.0, 0.0, 0.0],
            [1.0, 2.0, 3.0, 0.0, 255.0, 0.0],
        ] {
            buf.extend_from_slice(&v[0].to_le_bytes());
            buf.extend_from_slice(&v[1].to_le_bytes());
            buf.extend_from_slice(&v[2].to_le_bytes());
            buf.push(v[3] as u8);
            buf.push(v[4] as u8);
            buf.push(v[5] as u8);
        }
        let cloud = load_ply_bytes(&buf).unwrap();
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.splats[0].position, [0.0, 0.0, 0.0]);
        assert!((cloud.splats[0].color[0] - 1.0).abs() < 1e-5);
        assert_eq!(cloud.splats[1].position, [1.0, 2.0, 3.0]);
        assert!((cloud.splats[1].color[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ply_without_vertex_element_errors() {
        let buf = b"ply\nformat ascii 1.0\nend_header\n".to_vec();
        assert!(load_ply_bytes(&buf).is_err());
    }

    #[test]
    fn unknown_extension_errors() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_splat_test.txt");
        std::fs::write(&path, b"nope").unwrap();
        let err = load_file(&path).unwrap_err();
        assert!(matches!(err, SplatError::UnknownExtension));
        let _ = std::fs::remove_file(&path);
    }
}
