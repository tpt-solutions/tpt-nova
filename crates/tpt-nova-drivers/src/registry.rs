//! Versioned driver registry with atomic, lock-free reads.
//!
//! The registry publishes one immutable [`RegistryState`] through an
//! [`ArcSwap`]. Readers take a snapshot (a cheap `Arc` clone) and see one
//! consistent state for as long as they hold it. Writers (register, activate,
//! rollback) are serialized by a mutex, build a new state, and publish it in one
//! swap.
//!
//! So a frame that started before an [`DriverRegistry::activate`] keeps the
//! driver it began with, and the next frame's snapshot sees the new one. No
//! frame ever observes a half-applied swap or a component with no active driver.
//!
//! Two gates apply to activation:
//! - **Capability policy (Layer 2):** a driver can only become active if every
//!   capability it requests is in the registry's granted policy.
//! - **Rollback history (Layer 3):** each activation records the version it
//!   replaced, so [`DriverRegistry::rollback`] can revert in one call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use tpt_nova_ecs::ComponentTypeId;

use crate::driver::{Capability, CapabilitySet, Driver, DriverId, DriverVersion};

/// A shared, type-erased driver.
pub type DriverHandle = Arc<dyn Driver>;

/// Errors from registry writes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("driver {id} {version} is already registered for this component")]
    Duplicate {
        id: DriverId,
        version: DriverVersion,
    },
    #[error("no driver {version} is registered for this component")]
    UnknownVersion { version: DriverVersion },
    #[error("capability {0:?} requested by the driver is not granted by the policy")]
    CapabilityDenied(Capability),
    #[error("no earlier active version is available to roll back to")]
    NoPreviousVersion,
}

/// One immutable published state: every registered driver, the active version
/// per component, the granted capability policy, and rollback history.
#[derive(Clone, Default)]
pub struct RegistryState {
    drivers: HashMap<(ComponentTypeId, DriverVersion), DriverHandle>,
    active: HashMap<ComponentTypeId, DriverVersion>,
    /// Previously active versions per component, oldest first; the last entry is
    /// what [`DriverRegistry::rollback`] returns to.
    history: HashMap<ComponentTypeId, Vec<DriverVersion>>,
    policy: CapabilitySet,
}

impl RegistryState {
    /// The active driver for `component`, if one is activated.
    pub fn active(&self, component: ComponentTypeId) -> Option<DriverHandle> {
        let version = self.active.get(&component)?;
        self.drivers.get(&(component, *version)).cloned()
    }

    pub fn active_version(&self, component: ComponentTypeId) -> Option<DriverVersion> {
        self.active.get(&component).copied()
    }

    pub fn get(&self, component: ComponentTypeId, version: DriverVersion) -> Option<DriverHandle> {
        self.drivers.get(&(component, version)).cloned()
    }

    /// Every registered version for `component`, oldest first.
    pub fn versions(&self, component: ComponentTypeId) -> Vec<DriverVersion> {
        let mut v: Vec<DriverVersion> = self
            .drivers
            .keys()
            .filter(|(c, _)| *c == component)
            .map(|(_, ver)| *ver)
            .collect();
        v.sort();
        v
    }

    /// The active drivers, ordered by `(id, version)` so frame execution is
    /// deterministic regardless of hash-map iteration order.
    pub fn active_drivers(&self) -> Vec<DriverHandle> {
        let mut out: Vec<DriverHandle> = self
            .active
            .iter()
            .filter_map(|(c, v)| self.drivers.get(&(*c, *v)).cloned())
            .collect();
        out.sort_by_key(|d| (d.id(), d.version()));
        out
    }

    /// The capabilities the registry will grant to activated drivers.
    pub fn policy(&self) -> &CapabilitySet {
        &self.policy
    }
}

/// Thread-safe, versioned driver registry. See the module docs for the
/// concurrency model.
pub struct DriverRegistry {
    state: ArcSwap<RegistryState>,
    writer: Mutex<()>,
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverRegistry {
    /// A registry that grants no capabilities: only drivers that request none
    /// can be activated.
    pub fn new() -> Self {
        Self::with_policy(CapabilitySet::empty())
    }

    /// A registry that grants `policy` to activated drivers.
    pub fn with_policy(policy: CapabilitySet) -> Self {
        DriverRegistry {
            state: ArcSwap::from_pointee(RegistryState {
                policy,
                ..Default::default()
            }),
            writer: Mutex::new(()),
        }
    }

    /// Take a consistent snapshot. Hold it for the duration of a frame.
    pub fn snapshot(&self) -> Arc<RegistryState> {
        self.state.load_full()
    }

    /// Register a driver version. Does not activate it.
    pub fn register(&self, driver: DriverHandle) -> Result<(), RegistryError> {
        let _guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = (*self.state.load_full()).clone();
        let key = (driver.component(), driver.version());
        if next.drivers.contains_key(&key) {
            return Err(RegistryError::Duplicate {
                id: driver.id(),
                version: driver.version(),
            });
        }
        next.drivers.insert(key, driver);
        self.state.store(Arc::new(next));
        Ok(())
    }

    /// Make `version` the active driver for `component`, recording the version
    /// it replaces for rollback. Fails if the version is unknown or requests a
    /// capability the policy does not grant. Takes effect for the next snapshot.
    pub fn activate(
        &self,
        component: ComponentTypeId,
        version: DriverVersion,
    ) -> Result<(), RegistryError> {
        self.set_active(component, version, true)
    }

    /// Revert `component` to the version active before the current one.
    ///
    /// Returns the version now active. Versions that were rolled back from are
    /// removed from the history, so repeated calls walk further back.
    pub fn rollback(&self, component: ComponentTypeId) -> Result<DriverVersion, RegistryError> {
        let _guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = (*self.state.load_full()).clone();
        // Walk back through history, skipping versions that are no longer
        // registered (should not happen, but a stale entry must not block revert).
        loop {
            let candidate = next
                .history
                .get_mut(&component)
                .and_then(|h| h.pop())
                .ok_or(RegistryError::NoPreviousVersion)?;
            if next.drivers.contains_key(&(component, candidate)) {
                next.active.insert(component, candidate);
                self.state.store(Arc::new(next));
                return Ok(candidate);
            }
        }
    }

    /// Convenience lookup that reads a fresh snapshot.
    pub fn active(&self, component: ComponentTypeId) -> Option<DriverHandle> {
        self.snapshot().active(component)
    }

    pub fn get(&self, component: ComponentTypeId, version: DriverVersion) -> Option<DriverHandle> {
        self.snapshot().get(component, version)
    }

    fn set_active(
        &self,
        component: ComponentTypeId,
        version: DriverVersion,
        record_history: bool,
    ) -> Result<(), RegistryError> {
        let _guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = (*self.state.load_full()).clone();
        let driver = next
            .drivers
            .get(&(component, version))
            .cloned()
            .ok_or(RegistryError::UnknownVersion { version })?;

        // Layer 2: the driver may only run with capabilities the policy grants.
        let requested = driver.requested_capabilities();
        if let Some(missing) = requested.iter().find(|c| !next.policy.contains(*c)) {
            return Err(RegistryError::CapabilityDenied(missing));
        }

        if record_history {
            if let Some(previous) = next.active.get(&component).copied() {
                if previous != version {
                    next.history.entry(component).or_default().push(previous);
                }
            }
        }
        next.active.insert(component, version);
        self.state.store(Arc::new(next));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{DriverContext, DriverError};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tpt_nova_ecs::{Component, World};

    #[derive(Debug, Default)]
    struct Gravity;
    impl Component for Gravity {}

    /// Configurable test driver.
    struct Versioned {
        version: DriverVersion,
        caps: CapabilitySet,
        migrates: bool,
    }

    impl Driver for Versioned {
        fn id(&self) -> DriverId {
            DriverId::new("gravity-tweak").unwrap()
        }
        fn version(&self) -> DriverVersion {
            self.version
        }
        fn component(&self) -> ComponentTypeId {
            ComponentTypeId::of::<Gravity>()
        }
        fn requested_capabilities(&self) -> CapabilitySet {
            self.caps.clone()
        }
        fn update(&self, _ctx: &DriverContext, _world: &mut World) -> Result<(), DriverError> {
            Ok(())
        }
        fn migrate_from(
            &self,
            from: DriverVersion,
            data: &serde_json::Value,
        ) -> Option<Result<serde_json::Value, DriverError>> {
            if !self.migrates {
                return None;
            }
            let mut out = data.clone();
            out["migrated_from"] = serde_json::json!(from.to_string());
            Some(Ok(out))
        }
    }

    fn driver(major: u32) -> DriverHandle {
        Arc::new(Versioned {
            version: DriverVersion::new(major, 0, 0),
            caps: CapabilitySet::empty(),
            migrates: false,
        })
    }

    fn custom(major: u32, caps: CapabilitySet, migrates: bool) -> DriverHandle {
        Arc::new(Versioned {
            version: DriverVersion::new(major, 0, 0),
            caps,
            migrates,
        })
    }

    #[test]
    fn register_then_activate_selects_the_driver() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        assert!(reg.active(c).is_none());
        reg.register(driver(1)).unwrap();
        assert!(
            reg.active(c).is_none(),
            "registration alone does not activate"
        );
        reg.activate(c, DriverVersion::new(1, 0, 0)).unwrap();
        assert_eq!(
            reg.active(c).unwrap().version(),
            DriverVersion::new(1, 0, 0)
        );
    }

    #[test]
    fn duplicate_and_unknown_versions_are_rejected() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(1)).unwrap();
        assert!(matches!(
            reg.register(driver(1)),
            Err(RegistryError::Duplicate { .. })
        ));
        assert_eq!(
            reg.activate(c, DriverVersion::new(9, 0, 0)),
            Err(RegistryError::UnknownVersion {
                version: DriverVersion::new(9, 0, 0)
            })
        );
    }

    #[test]
    fn hot_swap_takes_effect_next_frame_and_keeps_in_flight_frames() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(1)).unwrap();
        reg.register(driver(2)).unwrap();
        reg.activate(c, DriverVersion::new(1, 0, 0)).unwrap();

        // A frame that began before the swap keeps its snapshot.
        let frame_n = reg.snapshot();

        reg.activate(c, DriverVersion::new(2, 0, 0)).unwrap();

        assert_eq!(frame_n.active_version(c), Some(DriverVersion::new(1, 0, 0)));
        // The next frame sees the new driver with no gap in between.
        assert_eq!(
            reg.snapshot().active_version(c),
            Some(DriverVersion::new(2, 0, 0))
        );
    }

    #[test]
    fn versions_are_listed_oldest_first_and_active_drivers_are_ordered() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(3)).unwrap();
        reg.register(driver(1)).unwrap();
        reg.activate(c, DriverVersion::new(3, 0, 0)).unwrap();
        let snap = reg.snapshot();
        assert_eq!(
            snap.versions(c),
            vec![DriverVersion::new(1, 0, 0), DriverVersion::new(3, 0, 0)]
        );
        assert_eq!(snap.active_drivers().len(), 1);
    }

    #[test]
    fn concurrent_readers_never_observe_a_missing_driver_during_swaps() {
        let reg = Arc::new(DriverRegistry::new());
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(1)).unwrap();
        reg.register(driver(2)).unwrap();
        reg.activate(c, DriverVersion::new(1, 0, 0)).unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let reg = Arc::clone(&reg);
                let done = Arc::clone(&done);
                std::thread::spawn(move || {
                    let mut reads = 0u64;
                    while !done.load(Ordering::Relaxed) {
                        let d = reg.active(c).expect("active driver must never disappear");
                        assert!(d.version().major == 1 || d.version().major == 2);
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();

        for i in 0..2000u32 {
            let v = if i % 2 == 0 { 2 } else { 1 };
            reg.activate(c, DriverVersion::new(v, 0, 0)).unwrap();
        }
        done.store(true, Ordering::Relaxed);
        let total: u64 = readers.into_iter().map(|h| h.join().unwrap()).sum();
        assert!(total > 0);
    }

    #[test]
    fn activation_is_denied_for_ungranted_capabilities() {
        let c = ComponentTypeId::of::<Gravity>();
        let wants_physics = CapabilitySet::empty().with(Capability::WritePhysics);

        let strict = DriverRegistry::new();
        strict
            .register(custom(1, wants_physics.clone(), false))
            .unwrap();
        assert_eq!(
            strict.activate(c, DriverVersion::new(1, 0, 0)),
            Err(RegistryError::CapabilityDenied(Capability::WritePhysics))
        );
        assert!(strict.active(c).is_none());

        let granted = DriverRegistry::with_policy(wants_physics.clone());
        granted.register(custom(1, wants_physics, false)).unwrap();
        granted.activate(c, DriverVersion::new(1, 0, 0)).unwrap();
        assert!(granted.active(c).is_some());
    }

    #[test]
    fn rollback_reverts_to_the_previously_active_version() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(1)).unwrap();
        reg.register(driver(2)).unwrap();
        reg.register(driver(3)).unwrap();
        assert_eq!(reg.rollback(c), Err(RegistryError::NoPreviousVersion));

        reg.activate(c, DriverVersion::new(1, 0, 0)).unwrap();
        reg.activate(c, DriverVersion::new(2, 0, 0)).unwrap();
        reg.activate(c, DriverVersion::new(3, 0, 0)).unwrap();

        assert_eq!(reg.rollback(c).unwrap(), DriverVersion::new(2, 0, 0));
        assert_eq!(reg.rollback(c).unwrap(), DriverVersion::new(1, 0, 0));
        assert_eq!(reg.rollback(c), Err(RegistryError::NoPreviousVersion));
        assert_eq!(
            reg.snapshot().active_version(c),
            Some(DriverVersion::new(1, 0, 0))
        );
    }

    #[test]
    fn rollback_is_visible_to_the_next_snapshot_only() {
        let reg = DriverRegistry::new();
        let c = ComponentTypeId::of::<Gravity>();
        reg.register(driver(1)).unwrap();
        reg.register(driver(2)).unwrap();
        reg.activate(c, DriverVersion::new(1, 0, 0)).unwrap();
        reg.activate(c, DriverVersion::new(2, 0, 0)).unwrap();
        let in_flight = reg.snapshot();
        reg.rollback(c).unwrap();
        assert_eq!(
            in_flight.active_version(c),
            Some(DriverVersion::new(2, 0, 0))
        );
        assert_eq!(
            reg.snapshot().active_version(c),
            Some(DriverVersion::new(1, 0, 0))
        );
    }
}
