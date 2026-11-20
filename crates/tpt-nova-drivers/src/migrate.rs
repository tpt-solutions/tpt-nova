//! Migration chain composer (Phase 8, Layer 3).
//!
//! Serialized component data written by an older driver version is upgraded one
//! version at a time: `from → … → to`, where each hop is executed by the *target*
//! version's [`Driver::migrate_from`]. The composer refuses any path that has a
//! missing hop, so a schema change can never be applied without a registered
//! migration for every step.

use serde_json::Value;
use tpt_nova_ecs::ComponentTypeId;

use crate::driver::{DriverError, DriverVersion};
use crate::registry::RegistryState;

/// Why a migration could not be composed or applied.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MigrationError {
    #[error("version {version} is not registered for this component")]
    UnknownVersion { version: DriverVersion },
    #[error("no migration registered from {from} to {to}")]
    NoPath {
        from: DriverVersion,
        to: DriverVersion,
    },
    #[error("migration {from} -> {to} failed: {source}")]
    Failed {
        from: DriverVersion,
        to: DriverVersion,
        source: DriverError,
    },
}

/// Upgrade `data` from `from` to `to` through every registered version in
/// between. Downgrades are not supported.
///
/// `from == to` returns the data unchanged. The path visits each registered
/// version in order; each hop must be answered by the target version's
/// `migrate_from`.
pub fn migrate(
    state: &RegistryState,
    component: ComponentTypeId,
    from: DriverVersion,
    to: DriverVersion,
    data: Value,
) -> Result<Value, MigrationError> {
    let versions = state.versions(component);
    for v in [from, to] {
        if !versions.contains(&v) {
            return Err(MigrationError::UnknownVersion { version: v });
        }
    }
    if from == to {
        return Ok(data);
    }
    if from > to {
        return Err(MigrationError::NoPath { from, to });
    }

    // The chain is every registered version from `from` through `to`, in order.
    let chain: Vec<DriverVersion> = versions
        .into_iter()
        .filter(|v| *v >= from && *v <= to)
        .collect();

    let mut current = data;
    for pair in chain.windows(2) {
        let (prev, next) = (pair[0], pair[1]);
        let driver = state
            .get(component, next)
            .ok_or(MigrationError::UnknownVersion { version: next })?;
        match driver.migrate_from(prev, &current) {
            None => {
                return Err(MigrationError::NoPath {
                    from: prev,
                    to: next,
                })
            }
            Some(Err(source)) => {
                return Err(MigrationError::Failed {
                    from: prev,
                    to: next,
                    source,
                })
            }
            Some(Ok(upgraded)) => current = upgraded,
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{CapabilitySet, Driver, DriverContext, DriverId};
    use crate::registry::{DriverHandle, DriverRegistry};
    use std::sync::Arc;
    use tpt_nova_ecs::{Component, World};

    #[derive(Debug, Default)]
    struct Health;
    impl Component for Health {}

    /// Driver whose `migrate_from` adds a `"v{to}"` marker, or is absent.
    struct Step {
        version: DriverVersion,
        migrates: bool,
        fail: bool,
    }

    impl Driver for Step {
        fn id(&self) -> DriverId {
            DriverId::new("health").unwrap()
        }
        fn version(&self) -> DriverVersion {
            self.version
        }
        fn component(&self) -> ComponentTypeId {
            ComponentTypeId::of::<Health>()
        }
        fn requested_capabilities(&self) -> CapabilitySet {
            CapabilitySet::empty()
        }
        fn update(&self, _: &DriverContext, _: &mut World) -> Result<(), DriverError> {
            Ok(())
        }
        fn migrate_from(
            &self,
            _from: DriverVersion,
            data: &Value,
        ) -> Option<Result<Value, DriverError>> {
            if self.fail {
                return Some(Err(DriverError::Failed("bad field".into())));
            }
            if !self.migrates {
                return None;
            }
            let mut out = data.clone();
            let hops = out.get("hops").and_then(Value::as_u64).unwrap_or(0);
            out["hops"] = serde_json::json!(hops + 1);
            out["to"] = serde_json::json!(self.version.to_string());
            Some(Ok(out))
        }
    }

    fn registry_with(steps: &[(u32, bool, bool)]) -> DriverRegistry {
        let reg = DriverRegistry::new();
        for &(major, migrates, fail) in steps {
            let d: DriverHandle = Arc::new(Step {
                version: DriverVersion::new(major, 0, 0),
                migrates,
                fail,
            });
            reg.register(d).unwrap();
        }
        reg
    }

    fn v(major: u32) -> DriverVersion {
        DriverVersion::new(major, 0, 0)
    }

    #[test]
    fn composes_every_hop_across_the_chain() {
        // v1 -> v2 -> v3, each step registered.
        let reg = registry_with(&[(1, false, false), (2, true, false), (3, true, false)]);
        let c = ComponentTypeId::of::<Health>();
        let out = migrate(&reg.snapshot(), c, v(1), v(3), serde_json::json!({"hp": 5})).unwrap();
        assert_eq!(out["hops"], 2);
        assert_eq!(out["to"], "3.0.0");
        assert_eq!(out["hp"], 5, "existing fields are preserved");
    }

    #[test]
    fn refuses_a_path_with_a_missing_hop() {
        // v2 has no migration from v1, so 1 -> 3 cannot be composed.
        let reg = registry_with(&[(1, false, false), (2, false, false), (3, true, false)]);
        let c = ComponentTypeId::of::<Health>();
        assert_eq!(
            migrate(&reg.snapshot(), c, v(1), v(3), serde_json::json!({})),
            Err(MigrationError::NoPath {
                from: v(1),
                to: v(2)
            })
        );
    }

    #[test]
    fn surfaces_driver_migration_failures() {
        let reg = registry_with(&[(1, false, false), (2, false, true)]);
        let c = ComponentTypeId::of::<Health>();
        let err = migrate(&reg.snapshot(), c, v(1), v(2), serde_json::json!({})).unwrap_err();
        assert!(
            matches!(err, MigrationError::Failed { from, to, .. } if from == v(1) && to == v(2))
        );
    }

    #[test]
    fn rejects_unknown_versions_and_downgrades() {
        let reg = registry_with(&[(1, false, false), (2, true, false)]);
        let c = ComponentTypeId::of::<Health>();
        let snap = reg.snapshot();
        assert_eq!(
            migrate(&snap, c, v(1), v(9), serde_json::json!({})),
            Err(MigrationError::UnknownVersion { version: v(9) })
        );
        assert_eq!(
            migrate(&snap, c, v(2), v(1), serde_json::json!({})),
            Err(MigrationError::NoPath {
                from: v(2),
                to: v(1)
            })
        );
    }

    #[test]
    fn same_version_is_identity() {
        let reg = registry_with(&[(1, false, false)]);
        let c = ComponentTypeId::of::<Health>();
        let data = serde_json::json!({"hp": 1});
        assert_eq!(
            migrate(&reg.snapshot(), c, v(1), v(1), data.clone()),
            Ok(data)
        );
    }
}
