//! Entity identifiers with generational indexing.
//!
//! An [`Entity`] is a stable handle: a packed index plus a generation counter.
//! Reusing a slot bumps the generation so stale handles never resurrect a
//! different entity.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A unique entity handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

impl Entity {
    pub const INVALID: Entity = Entity {
        index: u32::MAX,
        generation: u32::MAX,
    };

    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Entity { index, generation }
    }

    pub fn is_valid(&self) -> bool {
        *self != Entity::INVALID
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}#{}", self.index, self.generation)
    }
}
