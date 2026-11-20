//! The `Driver` contract: identity, versioning, declared capabilities, and the
//! update entry point a hot-swappable behavior implements.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use tpt_nova_ecs::{ComponentTypeId, World};

/// Stable name of a driver, e.g. `"gravity-tweak"`. Ids group versions: two
/// drivers with the same id but different versions are the same behavior at
/// different revisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DriverId(String);

impl DriverId {
    /// Build an id. Empty names are rejected so every driver is addressable.
    pub fn new(name: impl Into<String>) -> Result<Self, DriverError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DriverError::EmptyId);
        }
        Ok(DriverId(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Semantic version of a driver revision. Ordered lexicographically by
/// `(major, minor, patch)`, so the newest version is the maximum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct DriverVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl DriverVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        DriverVersion {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for DriverVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A permission a driver may exercise. Layer 1 only defines and checks the set;
/// enforcing it against a real sandbox is Layer 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    ReadTransform,
    WriteTransform,
    ReadNeighbors,
    WritePhysics,
    EmitEvents,
}

/// The set of capabilities a driver requests or is granted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn with(mut self, cap: Capability) -> Self {
        self.0.insert(cap);
        self
    }

    pub fn contains(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    /// True when every capability in `self` is also in `granted`.
    pub fn is_subset_of(&self, granted: &CapabilitySet) -> bool {
        self.0.is_subset(&granted.0)
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        CapabilitySet(iter.into_iter().collect())
    }
}

/// Per-update data handed to a driver. Carries the deterministic tick, the fixed
/// timestep, and the capabilities granted for this call.
#[derive(Debug, Clone)]
pub struct DriverContext {
    pub tick: u64,
    pub dt: f32,
    granted: CapabilitySet,
}

impl DriverContext {
    pub fn new(tick: u64, dt: f32, granted: CapabilitySet) -> Self {
        DriverContext { tick, dt, granted }
    }

    pub fn granted(&self) -> &CapabilitySet {
        &self.granted
    }

    /// Fail with [`DriverError::CapabilityDenied`] unless `cap` was granted.
    pub fn require(&self, cap: Capability) -> Result<(), DriverError> {
        if self.granted.contains(cap) {
            Ok(())
        } else {
            Err(DriverError::CapabilityDenied(cap))
        }
    }
}

/// A hot-swappable behavior bound to one component type.
///
/// Implementations must be stateless with respect to the swap: anything a
/// driver needs to persist belongs in the ECS, so replacing the driver does not
/// lose state.
pub trait Driver: Send + Sync {
    fn id(&self) -> DriverId;

    fn version(&self) -> DriverVersion;

    /// The component type this driver operates on. Registry lookups key on it.
    fn component(&self) -> ComponentTypeId;

    /// Capabilities this driver needs. Checked at activation in later layers.
    fn requested_capabilities(&self) -> CapabilitySet {
        CapabilitySet::empty()
    }

    /// Run one update against the world for the current tick.
    fn update(&self, ctx: &DriverContext, world: &mut World) -> Result<(), DriverError>;

    /// Upgrade serialized component data written by `from` into this version's
    /// schema. Return `None` (the default) when this driver has no migration
    /// from `from`; the migration composer then refuses the path.
    fn migrate_from(
        &self,
        from: DriverVersion,
        data: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, DriverError>> {
        let _ = (from, data);
        None
    }
}

/// Errors from driver construction and execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriverError {
    #[error("driver id must not be empty")]
    EmptyId,
    #[error("capability {0:?} was not granted to this driver")]
    CapabilityDenied(Capability),
    #[error("driver failed: {0}")]
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_newest_last() {
        assert!(DriverVersion::new(1, 2, 3) < DriverVersion::new(1, 10, 0));
        assert!(DriverVersion::new(2, 0, 0) > DriverVersion::new(1, 99, 99));
        assert_eq!(DriverVersion::new(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn empty_driver_ids_are_rejected() {
        assert_eq!(DriverId::new("   "), Err(DriverError::EmptyId));
        assert_eq!(DriverId::new("gravity").unwrap().as_str(), "gravity");
    }

    #[test]
    fn capability_subset_checks() {
        let want = CapabilitySet::empty().with(Capability::ReadTransform);
        let granted: CapabilitySet = [Capability::ReadTransform, Capability::EmitEvents]
            .into_iter()
            .collect();
        assert!(want.is_subset_of(&granted));
        assert!(!granted.is_subset_of(&want));
        assert!(CapabilitySet::empty().is_subset_of(&want));
    }

    #[test]
    fn context_denies_ungranted_capabilities() {
        let granted = CapabilitySet::empty().with(Capability::ReadTransform);
        let ctx = DriverContext::new(0, 1.0 / 60.0, granted);
        assert!(ctx.require(Capability::ReadTransform).is_ok());
        assert_eq!(
            ctx.require(Capability::WritePhysics),
            Err(DriverError::CapabilityDenied(Capability::WritePhysics))
        );
    }
}
