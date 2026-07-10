//! Component trait and stable type identifiers.

use std::any::TypeId;

/// A piece of data attached to an [`Entity`](crate::Entity).
///
/// Components must be `'static`, `Send`, and `Sync` so they can be stored in a
/// type-erased map and accessed from multiple systems.
pub trait Component: 'static + Send + Sync {}

/// A `TypeId` wrapper for component types, used as the key into the world's
/// storage map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentTypeId(TypeId);

impl ComponentTypeId {
    pub fn of<T: Component>() -> Self {
        ComponentTypeId(TypeId::of::<T>())
    }
}
