//! Structured telemetry for TPT Nova.
//!
//! The engine emits machine-readable JSON frames describing the full ECS
//! state. AI agents read this telemetry, decide on a change, and write a
//! control file (see `nova-app`) that the engine hot-applies — closing the
//! self-debugging loop without a human in the middle.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nova_ecs::scene_graph::Parent;
use nova_ecs::transform::{Camera, GlobalTransform, Mesh, Transform};
use nova_ecs::{Children, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level telemetry payload emitted each interval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryFrame {
    pub schema_version: u32,
    pub tick: u64,
    pub seed: u64,
    pub entities: Vec<EntityDump>,
}

/// One entity's dumpable components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDump {
    pub id: String,
    pub components: HashMap<String, Value>,
}

fn transform_value(t: &Transform) -> Value {
    serde_json::json!({
        "translation": [t.translation.x, t.translation.y, t.translation.z],
        "rotation": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
        "scale": [t.scale.x, t.scale.y, t.scale.z],
    })
}

fn global_transform_value(g: &GlobalTransform) -> Value {
    let m = g.0.to_cols_array();
    serde_json::json!({ "matrix": m })
}

fn camera_value(c: &Camera) -> Value {
    serde_json::json!({
        "fov_y": c.fov_y,
        "near": c.near,
        "far": c.far,
        "aspect": c.aspect,
    })
}

fn mesh_value(m: &Mesh) -> Value {
    serde_json::json!({ "kind": format!("{:?}", m.kind) })
}

fn parent_value(p: &Parent) -> Value {
    serde_json::json!({ "parent": format!("{}", p.0) })
}

fn children_value(c: &Children) -> Value {
    let ids: Vec<String> = c.0.iter().map(|e| format!("{}", e)).collect();
    serde_json::json!({ "children": ids })
}

/// Build a telemetry frame for the entire world.
pub fn dump_world(world: &World, tick: u64, seed: u64) -> TelemetryFrame {
    let mut entities = Vec::new();
    for e in world.entities() {
        let mut components: HashMap<String, Value> = HashMap::new();
        if let Some(t) = world.get_component::<Transform>(e) {
            components.insert("Transform".to_string(), transform_value(t));
        }
        if let Some(g) = world.get_component::<GlobalTransform>(e) {
            components.insert("GlobalTransform".to_string(), global_transform_value(g));
        }
        if let Some(m) = world.get_component::<Mesh>(e) {
            components.insert("Mesh".to_string(), mesh_value(m));
        }
        if let Some(c) = world.get_component::<Camera>(e) {
            components.insert("Camera".to_string(), camera_value(c));
        }
        if let Some(p) = world.get_component::<Parent>(e) {
            components.insert("Parent".to_string(), parent_value(p));
        }
        if let Some(c) = world.get_component::<Children>(e) {
            components.insert("Children".to_string(), children_value(c));
        }
        entities.push(EntityDump {
            id: format!("{}", e),
            components,
        });
    }
    TelemetryFrame {
        schema_version: 1,
        tick,
        seed,
        entities,
    }
}

/// A destination for telemetry frames.
pub trait TelemetrySink {
    fn publish(&mut self, frame: &TelemetryFrame) -> io::Result<()>;
}

/// Writes pretty-printed JSON to stdout.
pub struct StdoutSink;

impl TelemetrySink for StdoutSink {
    fn publish(&mut self, frame: &TelemetryFrame) -> io::Result<()> {
        let s = serde_json::to_string_pretty(frame)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        println!("{}", s);
        Ok(())
    }
}

/// Writes pretty-printed JSON to a file (overwrites each tick).
pub struct FileSink {
    path: PathBuf,
}

impl FileSink {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        FileSink {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl TelemetrySink for FileSink {
    fn publish(&mut self, frame: &TelemetryFrame) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        let s = serde_json::to_string_pretty(frame)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        file.write_all(s.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(())
    }
}

/// Writes each frame as a MessagePack blob to a file (overwrites each tick).
///
/// Prefer this over [`FileSink`] for high-frequency or large telemetry payloads
/// where the compact binary encoding matters more than human readability.
pub struct MsgPackFileSink {
    path: PathBuf,
}

impl MsgPackFileSink {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        MsgPackFileSink {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl TelemetrySink for MsgPackFileSink {
    fn publish(&mut self, frame: &TelemetryFrame) -> io::Result<()> {
        let bytes = rmp_serde::to_vec_named(frame)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        file.write_all(&bytes)?;
        Ok(())
    }
}

/// Encode a telemetry frame as MessagePack bytes (named fields, so it stays
/// self-describing and round-trips through the same serde types as JSON).
pub fn encode_msgpack(frame: &TelemetryFrame) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec_named(frame)
}

/// A helper that owns a sink and emits on a fixed tick interval.
pub struct TelemetryEmitter<S: TelemetrySink> {
    sink: S,
    interval: u64,
    last_tick: i64,
}

impl<S: TelemetrySink> TelemetryEmitter<S> {
    pub fn new(sink: S, interval_ticks: u64) -> Self {
        TelemetryEmitter {
            sink,
            interval: interval_ticks.max(1),
            last_tick: -1,
        }
    }

    /// Emit if `tick` is a multiple of the interval. Returns the sink error, if any.
    pub fn maybe_emit(&mut self, world: &World, tick: u64, seed: u64) -> io::Result<bool> {
        if !tick.is_multiple_of(self.interval) || (tick as i64) == self.last_tick {
            return Ok(false);
        }
        self.last_tick = tick as i64;
        let frame = dump_world(world, tick, seed);
        self.sink.publish(&frame)?;
        Ok(true)
    }
}

/// Convenience: open a file sink for the standard telemetry filename.
pub fn default_telemetry_path() -> PathBuf {
    PathBuf::from("nova-telemetry.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::{Mesh, MeshKind, Transform};
    use nova_ecs::{Vec3, World};

    fn sample_world() -> World {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(
            e,
            Mesh {
                kind: MeshKind::Cube,
            },
        );
        world
    }

    #[test]
    fn dump_contains_entity_components() {
        let world = sample_world();
        let frame = dump_world(&world, 7, 42);
        assert_eq!(frame.tick, 7);
        assert_eq!(frame.seed, 42);
        assert_eq!(frame.entities.len(), 1);
        assert!(frame.entities[0].components.contains_key("Transform"));
        assert!(frame.entities[0].components.contains_key("Mesh"));
    }

    #[test]
    fn msgpack_roundtrips_through_serde() {
        let world = sample_world();
        let frame = dump_world(&world, 3, 99);
        let bytes = encode_msgpack(&frame).expect("encode");
        let decoded: TelemetryFrame = rmp_serde::from_slice(&bytes).expect("decode");
        assert_eq!(decoded.tick, frame.tick);
        assert_eq!(decoded.seed, frame.seed);
        assert_eq!(decoded.entities.len(), frame.entities.len());
    }

    #[test]
    fn json_and_msgpack_agree_on_content() {
        let world = sample_world();
        let frame = dump_world(&world, 1, 1);
        let via_json: TelemetryFrame =
            serde_json::from_str(&serde_json::to_string(&frame).unwrap()).unwrap();
        let via_mp: TelemetryFrame =
            rmp_serde::from_slice(&encode_msgpack(&frame).unwrap()).unwrap();
        assert_eq!(via_json.entities.len(), via_mp.entities.len());
    }
}
