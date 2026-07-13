//! Mesh loaders for smart ingestion.
//!
//! [`load`] dispatches by file extension to [`load_gltf`] (`.gltf`/`.glb`, via
//! the `gltf` crate) or [`load_obj`] (`.obj`, via `tobj`). Both return a list
//! of [`MeshData`] — plain, GPU-free geometry the rest of the pipeline (VHACD
//! decomposition, auto-rig, collider generation) consumes.

use std::path::Path;

use glam::Vec3;
use thiserror::Error;

/// Errors raised while ingesting a mesh file.
#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("gltf error: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("obj error: {0}")]
    Obj(#[from] tobj::LoadError),
    #[error("unsupported mesh file extension (expected .glb/.gltf/.obj)")]
    UnsupportedExtension,
    #[error("mesh contained no triangles")]
    EmptyMesh,
}

/// A single triangle mesh extracted from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Axis-aligned bounds of the mesh vertices.
    pub fn bounds(&self) -> Aabb {
        Aabb::from_points(&self.vertices)
    }
}

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_points(points: &[[f32; 3]]) -> Aabb {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for p in points {
            let v = Vec3::new(p[0], p[1], p[2]);
            min = min.min(v);
            max = max.max(v);
        }
        if points.is_empty() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        Aabb { min, max }
    }

    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

/// Load a mesh file, dispatching on its extension.
pub fn load(path: &Path) -> Result<Vec<MeshData>, IngestError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("glb") | Some("gltf") => load_gltf(path),
        Some("obj") => load_obj(path),
        _ => Err(IngestError::UnsupportedExtension),
    }
}

/// Load all meshes from a glTF 2.0 file (JSON `.gltf` or binary `.glb`).
pub fn load_gltf(path: &Path) -> Result<Vec<MeshData>, IngestError> {
    let (document, buffers, _) = gltf::import(path)?;

    let mut meshes = Vec::new();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|b| Some(&buffers[b.index()].0));
            let positions = match reader.read_positions() {
                Some(p) => p.into_iter().collect::<Vec<[f32; 3]>>(),
                None => continue,
            };
            let indices = match reader.read_indices() {
                Some(i) => i.into_u32().collect::<Vec<u32>>(),
                None => continue,
            };
            if indices.len() < 3 {
                continue;
            }
            meshes.push(MeshData {
                name: mesh.name().unwrap_or("mesh").to_string(),
                vertices: positions,
                indices,
            });
        }
    }

    if meshes.is_empty() {
        return Err(IngestError::EmptyMesh);
    }
    Ok(meshes)
}

/// Load all objects from a Wavefront `.obj` file.
pub fn load_obj(path: &Path) -> Result<Vec<MeshData>, IngestError> {
    let (models, _) = tobj::load_obj(
        path,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
    )?;

    let mut meshes = Vec::new();
    for model in models {
        let m = &model.mesh;
        if m.indices.len() < 3 {
            continue;
        }
        let vertices: Vec<[f32; 3]> = m.positions.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        meshes.push(MeshData {
            name: model.name.clone(),
            vertices,
            indices: m.indices.clone(),
        });
    }

    if meshes.is_empty() {
        return Err(IngestError::EmptyMesh);
    }
    Ok(meshes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_extension_is_rejected() {
        let err = load(std::path::Path::new("model.txt"));
        assert!(matches!(err, Err(IngestError::UnsupportedExtension)));
    }

    #[test]
    fn garbage_glb_is_rejected() {
        let path = std::env::temp_dir().join("nova_ingest_garbage_test.glb");
        std::fs::write(&path, b"this is definitely not a glb file").unwrap();
        let result = load_gltf(&path);
        assert!(result.is_err(), "garbage glb must fail to parse");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn garbage_obj_is_rejected() {
        let path = std::env::temp_dir().join("nova_ingest_garbage_test.obj");
        std::fs::write(&path, b"not a wavefront obj at all").unwrap();
        let result = load_obj(&path);
        assert!(result.is_err(), "garbage obj must fail to parse");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn aabb_bounds_are_correct() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 2.0, 3.0], [-1.0, -2.0, -3.0]];
        let b = Aabb::from_points(&pts);
        assert_eq!(b.min, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(b.max, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.center(), Vec3::ZERO);
        assert_eq!(b.size(), Vec3::new(2.0, 4.0, 6.0));
    }

    #[test]
    fn empty_aabb_is_zero() {
        let b = Aabb::from_points(&[]);
        assert_eq!(b.min, Vec3::ZERO);
        assert_eq!(b.max, Vec3::ZERO);
    }
}
