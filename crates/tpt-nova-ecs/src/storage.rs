//! Per-component storage backed by a `HashMap<Entity, T>`.
//!
//! This is a straightforward, correct sparse storage: it keeps data next to an
//! entity key and supports O(1) insert/get/remove. It is not cache-optimized
//! like an archetype store, but it keeps the API simple and predictable, which
//! matters more than raw throughput at Phase 1 scale.

use std::any::Any;
use std::collections::HashMap;

use crate::entity::Entity;

/// Sparse entity-keyed storage for one component type.
#[derive(Debug, Clone, Default)]
pub struct MapStorage<T> {
    data: HashMap<Entity, T>,
}

impl<T> MapStorage<T> {
    pub fn new() -> Self {
        MapStorage {
            data: HashMap::new(),
        }
    }

    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        self.data.insert(entity, value)
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(&entity)
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data.get_mut(&entity)
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.data.remove(&entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.data.contains_key(&entity)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn entities(&self) -> Vec<Entity> {
        self.data.keys().copied().collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.data.iter().map(|(e, v)| (*e, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        self.data.iter_mut().map(|(e, v)| (*e, v))
    }
}

/// Type-erased storage capability so the [`World`](crate::world::World) can
/// manage many concrete `MapStorage<T>` values behind one boxed trait object
/// while still allowing typed downcasts and entity erasure.
pub trait Storage: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn erase(&mut self, entity: Entity);
    fn entities(&self) -> Vec<Entity>;
}

impl<T: 'static + Send + Sync> Storage for MapStorage<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn erase(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn entities(&self) -> Vec<Entity> {
        MapStorage::entities(self)
    }
}
