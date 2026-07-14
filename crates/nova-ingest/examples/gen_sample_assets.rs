//! Generate the small, zero-config sample assets shipped with the repo:
//!
//! * `assets/cube.glb`  — a valid glTF 2.0 binary cube (8 verts, 12 tris).
//! * `assets/sample.splat` — a tiny 3DGS point cloud (the 32-byte/point layout).
//!
//! Run with `cargo run -p nova-ingest --example gen_sample_assets`. The files
//! are committed so `ingest_demo` (which defaults to `assets/cube.glb`) and the
//! splat loader are runnable with no extra downloads.

use std::path::PathBuf;

/// Cube corners, half-extent 0.5.
const CUBE_VERTS: [[f32; 3]; 8] = [
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, -0.5],
    [0.5, 0.5, -0.5],
    [-0.5, 0.5, -0.5],
    [-0.5, -0.5, 0.5],
    [0.5, -0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, 0.5],
];

/// 12 triangles (36 unsigned-int indices) covering the cube's faces.
const CUBE_INDICES: [u32; 36] = [
    0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 4, 5, 0, 5, 1, 1, 5, 6, 1, 6, 2, 2, 6, 7, 2, 7, 3, 3, 7,
    4, 3, 4, 0,
];

fn assets_dir() -> PathBuf {
    // examples run with cwd == crate dir; the workspace assets live two levels up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("..").join("..").join("assets")
}

fn write_cube_glb(path: &PathBuf) -> std::io::Result<()> {
    let mut bin = Vec::new();
    for v in CUBE_VERTS.iter() {
        bin.extend_from_slice(&v[0].to_le_bytes());
        bin.extend_from_slice(&v[1].to_le_bytes());
        bin.extend_from_slice(&v[2].to_le_bytes());
    }
    for i in CUBE_INDICES.iter() {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let bin_len = bin.len() as u32;

    let json = format!(
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"nova-sample-assets\"}},\
          \"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
          \"nodes\":[{{\"mesh\":0,\"name\":\"Cube\"}}],\
          \"meshes\":[{{\"name\":\"Cube\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}}],\
          \"buffers\":[{{\"byteLength\":{bin_len}}}],\
          \"bufferViews\":[\
            {{\"buffer\":0,\"byteOffset\":0,\"byteLength\":96,\"target\":34962}},\
            {{\"buffer\":0,\"byteOffset\":96,\"byteLength\":144,\"target\":34963}}],\
          \"accessors\":[\
            {{\"bufferView\":0,\"componentType\":5126,\"count\":8,\"type\":\"VEC3\",\
              \"min\":[-0.5,-0.5,-0.5],\"max\":[0.5,0.5,0.5]}},\
            {{\"bufferView\":1,\"componentType\":5125,\"count\":36,\"type\":\"SCALAR\"}}]}}"
    );
    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    let json_len = json_bytes.len() as u32;

    let mut bin_padded = bin.clone();
    while bin_padded.len() % 4 != 0 {
        bin_padded.push(0);
    }
    let bin_len_padded = bin_padded.len() as u32;

    let mut out = Vec::new();
    out.extend_from_slice(&0x4654_6C67u32.to_le_bytes()); // "glTF"
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    let total = 12 + (8 + json_bytes.len()) + (8 + bin_padded.len());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&bin_len_padded.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin_padded);

    std::fs::write(path, &out)
}

fn write_sample_splat(path: &PathBuf) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let ln_scale = 0.1f32.ln();
    for i in 0..64u32 {
        let x = ((i % 8) as f32 - 3.5) * 0.25;
        let y = ((i / 8) as f32 - 3.5) * 0.25;
        let z = ((i % 3) as f32 - 1.0) * 0.1;
        let mut rec = [0u8; 32];
        rec[0..4].copy_from_slice(&x.to_le_bytes());
        rec[4..8].copy_from_slice(&y.to_le_bytes());
        rec[8..12].copy_from_slice(&z.to_le_bytes());
        rec[12..16].copy_from_slice(&ln_scale.to_le_bytes());
        rec[16..20].copy_from_slice(&ln_scale.to_le_bytes());
        rec[20..24].copy_from_slice(&ln_scale.to_le_bytes());
        rec[24] = 80;
        rec[25] = 200;
        rec[26] = 120;
        rec[27] = 255;
        rec[28] = 255;
        rec[29] = 128;
        rec[30] = 128;
        rec[31] = 128;
        buf.extend_from_slice(&rec);
    }
    std::fs::write(path, &buf)
}

fn main() {
    let assets = assets_dir();
    std::fs::create_dir_all(&assets).expect("create assets dir");
    let glb = assets.join("cube.glb");
    let splat = assets.join("sample.splat");
    write_cube_glb(&glb).expect("write cube.glb");
    write_sample_splat(&splat).expect("write sample.splat");
    println!(
        "wrote {} ({} bytes)",
        glb.display(),
        std::fs::metadata(&glb).unwrap().len()
    );
    println!(
        "wrote {} ({} bytes)",
        splat.display(),
        std::fs::metadata(&splat).unwrap().len()
    );
}
