//! Splat telemetry: attach a [`crate::SplatSummary`] to each splat entity's dump.
//!
//! `tpt-nova-telemetry::dump_world` only knows the core component kinds, so this
//! module adds the splat entry afterwards. It is a plain function rather than a
//! dependency edge in the other direction, so telemetry stays splat-agnostic.

use tpt_nova_ecs::World;
use tpt_nova_telemetry::TelemetryFrame;

use crate::SplatCloud;

/// Key under which the splat summary appears in an entity's telemetry components.
pub const SPLAT_SUMMARY_KEY: &str = "SplatSummary";

/// Add a `SplatSummary` component entry to every splat entity present in `frame`.
/// Entities already in the frame are updated; others are skipped.
pub fn augment_frame(world: &World, frame: &mut TelemetryFrame) {
    for (entity, cloud) in world.query_1::<SplatCloud>() {
        let id = format!("{entity}");
        let Some(dump) = frame.entities.iter_mut().find(|d| d.id == id) else {
            continue;
        };
        if let Ok(value) = serde_json::to_value(cloud.summary()) {
            dump.components.insert(SPLAT_SUMMARY_KEY.to_string(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Splat;
    use tpt_nova_telemetry::dump_world;

    #[test]
    fn splat_summary_appears_in_telemetry_frame() {
        let mut world = World::new();
        let e = world.spawn();
        let cloud = SplatCloud::new(vec![Splat {
            position: [0.0; 3],
            scale: [0.1; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            color: [1.0; 4],
            opacity: 0.8,
        }]);
        world.add_component(e, cloud);
        let mut frame = dump_world(&world, 0, 0);
        augment_frame(&world, &mut frame);
        let dump = frame
            .entities
            .iter()
            .find(|d| d.id == format!("{e}"))
            .unwrap();
        let summary = dump.components.get(SPLAT_SUMMARY_KEY).unwrap();
        assert_eq!(summary["count"], 1);
    }
}
