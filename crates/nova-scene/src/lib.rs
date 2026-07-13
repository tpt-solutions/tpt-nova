//! Scene (de)serialization: dump and restore the full ECS world to disk.
//!
//! A [`SceneFile`] is a versioned, human-readable snapshot of every entity and
//! its supported components. It can be written as RON (default, diff-friendly)
//! or JSON, and read back into a fresh [`World`] with entity references
//! (parent/children) remapped to the newly-allocated handles.
//!
//! ## Versioning strategy
//!
//! Every file carries a [`SceneFile::version`]. The loader refuses to open a
//! file newer than [`CURRENT_SCENE_VERSION`] (the running engine can't know how
//! to interpret future fields) and runs [`migrate`] to upgrade older files
//! forward, one version at a time, before instantiating them. New optional
//! component fields default to `None` via `#[serde(default)]`, so additive
//! changes are backward compatible without a migration; migrations exist for
//! renames/removals/semantic changes.

use std::path::Path;

use nova_ecs::scene_graph::{Children, Parent};
use nova_ecs::transform::{Camera, GlobalTransform, Mesh, Transform};
use nova_ecs::{Entity, World};
use nova_physics::{Collider2D, RigidBody2D};
use serde::{Deserialize, Serialize};

/// The scene schema version this build reads and writes.
pub const CURRENT_SCENE_VERSION: u32 = 1;

/// Errors raised while loading a scene.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("scene version {found} is newer than supported version {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ron error: {0}")]
    Ron(#[from] ron::error::SpannedError),
    #[error("ron serialize error: {0}")]
    RonSer(#[from] ron::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown scene file extension (expected .ron or .json)")]
    UnknownExtension,
}

/// A complete, versioned snapshot of an ECS world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneFile {
    pub version: u32,
    pub entities: Vec<EntityRecord>,
}

/// One entity's serializable components. New component types are added as
/// `#[serde(default)]` optional fields to stay backward compatible.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityRecord {
    /// The entity's original index, used only to remap parent/child references
    /// on load. Not a stable cross-session identity.
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<Mesh>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_body: Option<RigidBody2D>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider: Option<Collider2D>,
    /// Parent entity, by original id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// Child entities, by original id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<u32>,
}

/// Serialization format for a scene on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneFormat {
    Ron,
    Json,
}

// ---- Dumping ------------------------------------------------------------

/// Capture the full world into a [`SceneFile`].
///
/// [`GlobalTransform`] is intentionally *not* stored: it is derived state,
/// recomputed by the scene-graph propagation system after load.
pub fn dump_world(world: &World) -> SceneFile {
    let mut entities = Vec::new();
    for e in world.entities() {
        let parent = world.get_component::<Parent>(e).map(|p| p.0.index);
        let children = world
            .get_component::<Children>(e)
            .map(|c| c.0.iter().map(|e| e.index).collect())
            .unwrap_or_default();
        entities.push(EntityRecord {
            id: e.index,
            transform: world.get_component::<Transform>(e).copied(),
            mesh: world.get_component::<Mesh>(e).copied(),
            camera: world.get_component::<Camera>(e).copied(),
            rigid_body: world.get_component::<RigidBody2D>(e).copied(),
            collider: world.get_component::<Collider2D>(e).copied(),
            parent,
            children,
        });
    }
    // Stable ordering so serialized output is deterministic.
    entities.sort_by_key(|r| r.id);
    SceneFile {
        version: CURRENT_SCENE_VERSION,
        entities,
    }
}

// ---- Loading ------------------------------------------------------------

/// Instantiate a [`SceneFile`] into a brand-new [`World`].
///
/// Runs [`migrate`] first, then spawns entities and remaps parent/child
/// references to freshly-allocated handles.
pub fn load_world(mut scene: SceneFile) -> Result<World, SceneError> {
    migrate(&mut scene)?;

    let mut world = World::new();
    let mut remap: std::collections::HashMap<u32, Entity> = std::collections::HashMap::new();

    // First pass: spawn and record id -> new Entity.
    for record in &scene.entities {
        let e = world.spawn();
        remap.insert(record.id, e);
    }

    // Second pass: attach components + remapped relationships.
    for record in &scene.entities {
        let e = remap[&record.id];
        if let Some(t) = record.transform {
            world.add_component(e, t);
            // Give every transformed entity a GlobalTransform for propagation.
            world.add_component(e, GlobalTransform::identity());
        }
        if let Some(m) = record.mesh {
            world.add_component(e, m);
        }
        if let Some(c) = record.camera {
            world.add_component(e, c);
        }
        if let Some(rb) = record.rigid_body {
            world.add_component(e, rb);
        }
        if let Some(col) = record.collider {
            world.add_component(e, col);
        }
        if let Some(pid) = record.parent {
            if let Some(&pe) = remap.get(&pid) {
                world.add_component(e, Parent(pe));
            }
        }
        if !record.children.is_empty() {
            let kids: Vec<Entity> = record
                .children
                .iter()
                .filter_map(|cid| remap.get(cid).copied())
                .collect();
            world.add_component(e, Children(kids));
        }
    }

    Ok(world)
}

/// Upgrade a scene loaded from an older schema version to
/// [`CURRENT_SCENE_VERSION`], in place.
///
/// Add a match arm per version bump. Each arm mutates `scene` from version `N`
/// to `N + 1` and updates `scene.version`.
pub fn migrate(scene: &mut SceneFile) -> Result<(), SceneError> {
    if scene.version > CURRENT_SCENE_VERSION {
        return Err(SceneError::UnsupportedVersion {
            found: scene.version,
            supported: CURRENT_SCENE_VERSION,
        });
    }
    while scene.version < CURRENT_SCENE_VERSION {
        // Apply per-version migrations here as they are introduced, e.g.:
        //   if scene.version == 1 { migrate_v1_to_v2(scene); }
        // Versions without schema changes simply bump forward.
        scene.version += 1;
    }
    Ok(())
}

// ---- String (de)serialization ------------------------------------------

pub fn to_string(scene: &SceneFile, format: SceneFormat) -> Result<String, SceneError> {
    match format {
        SceneFormat::Ron => Ok(ron::ser::to_string_pretty(
            scene,
            ron::ser::PrettyConfig::default(),
        )?),
        SceneFormat::Json => Ok(serde_json::to_string_pretty(scene)?),
    }
}

pub fn from_str(text: &str, format: SceneFormat) -> Result<SceneFile, SceneError> {
    match format {
        SceneFormat::Ron => Ok(ron::from_str(text)?),
        SceneFormat::Json => Ok(serde_json::from_str(text)?),
    }
}

// ---- File helpers -------------------------------------------------------

fn format_for_path(path: &Path) -> Result<SceneFormat, SceneError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ron") => Ok(SceneFormat::Ron),
        Some("json") => Ok(SceneFormat::Json),
        _ => Err(SceneError::UnknownExtension),
    }
}

/// Dump `world` to a file; format is chosen by the `.ron`/`.json` extension.
pub fn save_to_file<P: AsRef<Path>>(world: &World, path: P) -> Result<(), SceneError> {
    let path = path.as_ref();
    let format = format_for_path(path)?;
    let scene = dump_world(world);
    let text = to_string(&scene, format)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Load a world from a file; format is chosen by the `.ron`/`.json` extension.
pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<World, SceneError> {
    let path = path.as_ref();
    let format = format_for_path(path)?;
    let text = std::fs::read_to_string(path)?;
    let scene = from_str(&text, format)?;
    load_world(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::MeshKind;
    use nova_ecs::Vec3;
    use nova_physics::ColliderShape;

    fn sample_world() -> (World, Entity, Entity) {
        let mut world = World::new();
        let parent = world.spawn();
        world.add_component(
            parent,
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        );
        world.add_component(
            parent,
            Mesh {
                kind: MeshKind::Cube,
            },
        );

        let child = world.spawn();
        world.add_component(child, Transform::from_translation(Vec3::new(0.5, 0.0, 0.0)));
        world.add_component(child, RigidBody2D::dynamic());
        world.add_component(child, Collider2D::new(ColliderShape::ball(0.25)));

        world.add_component(child, Parent(parent));
        world.add_component(parent, Children(vec![child]));
        (world, parent, child)
    }

    #[test]
    fn ron_roundtrip_preserves_components_and_hierarchy() {
        let (world, _p, _c) = sample_world();
        let text = to_string(&dump_world(&world), SceneFormat::Ron).unwrap();
        let restored = load_world(from_str(&text, SceneFormat::Ron).unwrap()).unwrap();

        assert_eq!(restored.entity_count(), 2);
        // Find the entity that has a Mesh (the parent) and confirm its child link.
        let meshed = restored.query_1::<Mesh>();
        assert_eq!(meshed.len(), 1);
        let parent = meshed[0].0;
        let kids = restored.get_component::<Children>(parent).unwrap();
        assert_eq!(kids.0.len(), 1);
        let child = kids.0[0];
        let back = restored.get_component::<Parent>(child).unwrap();
        assert_eq!(back.0, parent);
        assert!(restored.get_component::<RigidBody2D>(child).is_some());
    }

    #[test]
    fn json_roundtrip_matches_ron() {
        let (world, _p, _c) = sample_world();
        let scene = dump_world(&world);
        let json = to_string(&scene, SceneFormat::Json).unwrap();
        let ron = to_string(&scene, SceneFormat::Ron).unwrap();
        let from_json = from_str(&json, SceneFormat::Json).unwrap();
        let from_ron = from_str(&ron, SceneFormat::Ron).unwrap();
        assert_eq!(from_json.entities.len(), from_ron.entities.len());
        assert_eq!(from_json.version, CURRENT_SCENE_VERSION);
    }

    #[test]
    fn rejects_future_versions() {
        let mut scene = dump_world(&World::new());
        scene.version = CURRENT_SCENE_VERSION + 1;
        match load_world(scene) {
            Err(SceneError::UnsupportedVersion { .. }) => {}
            other => panic!("expected UnsupportedVersion, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn file_roundtrip_by_extension() {
        let (world, _p, _c) = sample_world();
        let dir = std::env::temp_dir();
        let ron_path = dir.join("nova_scene_test.ron");
        let json_path = dir.join("nova_scene_test.json");
        save_to_file(&world, &ron_path).unwrap();
        save_to_file(&world, &json_path).unwrap();
        assert_eq!(load_from_file(&ron_path).unwrap().entity_count(), 2);
        assert_eq!(load_from_file(&json_path).unwrap().entity_count(), 2);
        let _ = std::fs::remove_file(ron_path);
        let _ = std::fs::remove_file(json_path);
    }

    #[test]
    fn migrates_older_version_forward_to_current() {
        let (world, _p, _c) = sample_world();
        let mut scene = dump_world(&world);
        assert_eq!(scene.version, CURRENT_SCENE_VERSION);
        // Pretend this file was written by an older build.
        scene.version = CURRENT_SCENE_VERSION - 1;
        let text = to_string(&scene, SceneFormat::Ron).unwrap();

        let parsed = from_str(&text, SceneFormat::Ron).unwrap();
        assert_eq!(parsed.version, CURRENT_SCENE_VERSION - 1);

        let restored = load_world(parsed).unwrap();
        assert_eq!(restored.entity_count(), 2);
        assert!(restored.query_1::<Mesh>().len() == 1);
    }

    #[test]
    fn corrupt_json_scene_is_rejected() {
        let err = from_str("{ this is not valid json", SceneFormat::Json);
        assert!(err.is_err());
    }

    #[test]
    fn corrupt_ron_scene_is_rejected() {
        let err = from_str("SceneFile(version: 1, entities: (", SceneFormat::Ron);
        assert!(err.is_err());
    }

    #[test]
    fn unknown_extension_is_rejected() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_scene_test.txt");
        let (world, _p, _c) = sample_world();
        let err = save_to_file(&world, &path);
        assert!(matches!(err, Err(SceneError::UnknownExtension)));
    }
}
