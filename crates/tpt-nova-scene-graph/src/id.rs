//! Stable node handles.

use serde::{Deserialize, Serialize};

/// A handle to a node inside a [`Scene`](crate::Scene).
///
/// Ids are indices into the scene's node arena. Slots are never reused, so a
/// stale id resolves to `None` instead of aliasing a newer node. This keeps ids
/// `Copy + Eq + Hash` and cheap to store in UI state, telemetry, and agent
/// commands without a generation counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "node#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_displayable_and_ordered() {
        assert_eq!(NodeId(7).to_string(), "node#7");
        assert!(NodeId(1) < NodeId(2));
    }
}
