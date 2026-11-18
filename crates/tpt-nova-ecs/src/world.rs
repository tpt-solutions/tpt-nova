//! The [`World`]: entity allocation, component storage, resources, and queries.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::component::{Component, ComponentTypeId};
use crate::entity::Entity;
use crate::storage::{MapStorage, Storage};

#[derive(Debug, Clone)]
struct EntityMeta {
    generation: u32,
    alive: bool,
}

/// The central container for all ECS state.
pub struct World {
    entities: Vec<EntityMeta>,
    free: Vec<u32>,
    next_id: u32,
    storages: HashMap<ComponentTypeId, Box<dyn Storage>>,
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    pub fn new() -> Self {
        World {
            entities: Vec::new(),
            free: Vec::new(),
            next_id: 0,
            storages: HashMap::new(),
            resources: HashMap::new(),
        }
    }

    // ---- Entities --------------------------------------------------------

    /// Create a new, alive entity.
    pub fn spawn(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let gen = self.entities[index as usize].generation;
            self.entities[index as usize].alive = true;
            Entity::new(index, gen)
        } else {
            let index = self.next_id;
            self.next_id += 1;
            self.entities.push(EntityMeta {
                generation: 0,
                alive: true,
            });
            Entity::new(index, 0)
        }
    }

    /// Destroy an entity and remove all of its components.
    pub fn despawn(&mut self, entity: Entity) {
        if !self.is_alive(entity) {
            return;
        }
        for storage in self.storages.values_mut() {
            storage.erase(entity);
        }
        let meta = &mut self.entities[entity.index as usize];
        meta.alive = false;
        meta.generation += 1;
        self.free.push(entity.index);
    }

    fn is_alive(&self, entity: Entity) -> bool {
        (entity.index as usize) < self.entities.len()
            && self.entities[entity.index as usize].alive
            && self.entities[entity.index as usize].generation == entity.generation
    }

    pub fn entity_count(&self) -> usize {
        self.entities.iter().filter(|m| m.alive).count()
    }

    // ---- Component storage ----------------------------------------------

    fn storage_mut<T: Component>(&mut self) -> &mut MapStorage<T> {
        let id = ComponentTypeId::of::<T>();
        self.storages
            .entry(id)
            .or_insert_with(|| Box::new(MapStorage::<T>::new()) as Box<dyn Storage>)
            .as_any_mut()
            .downcast_mut::<MapStorage<T>>()
            .expect("storage type mismatch")
    }

    fn storage_ref<T: Component>(&self) -> Option<&MapStorage<T>> {
        let id = ComponentTypeId::of::<T>();
        self.storages
            .get(&id)
            .and_then(|b| b.as_any().downcast_ref::<MapStorage<T>>())
    }

    pub fn add_component<T: Component>(&mut self, entity: Entity, value: T) -> Option<T> {
        if !self.is_alive(entity) {
            return Some(value);
        }
        self.storage_mut::<T>().insert(entity, value)
    }

    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.storage_ref::<T>().and_then(|s| s.get(entity))
    }

    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        self.storage_mut::<T>().get_mut(entity)
    }

    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
        self.storage_mut::<T>().remove(entity)
    }

    pub fn has_component<T: Component>(&self, entity: Entity) -> bool {
        self.storage_ref::<T>()
            .map(|s| s.contains(entity))
            .unwrap_or(false)
    }

    // ---- Resources -------------------------------------------------------

    pub fn add_resource<R: Any + Send + Sync>(&mut self, resource: R) {
        self.resources.insert(TypeId::of::<R>(), Box::new(resource));
    }

    pub fn resource<R: Any + Send + Sync>(&self) -> Option<&R> {
        self.resources
            .get(&TypeId::of::<R>())
            .and_then(|b| b.downcast_ref::<R>())
    }

    pub fn resource_mut<R: Any + Send + Sync>(&mut self) -> Option<&mut R> {
        self.resources
            .get_mut(&TypeId::of::<R>())
            .and_then(|b| b.downcast_mut::<R>())
    }

    /// Remove and return a resource, taking ownership. Useful when a system
    /// needs to mutate the world while also holding the resource (avoiding a
    /// double mutable borrow); re-insert it with [`World::add_resource`].
    pub fn remove_resource<R: Any + Send + Sync>(&mut self) -> Option<R> {
        self.resources
            .remove(&TypeId::of::<R>())
            .and_then(|b| b.downcast::<R>().ok())
            .map(|b| *b)
    }

    pub fn has_resource<R: Any + Send + Sync>(&self) -> bool {
        self.resources.contains_key(&TypeId::of::<R>())
    }

    // ---- Queries ---------------------------------------------------------

    /// Every entity that currently has at least one component.
    pub fn entities(&self) -> Vec<Entity> {
        let mut seen: HashMap<u32, u32> = HashMap::new();
        for storage in self.storages.values() {
            for e in storage.entities() {
                seen.insert(e.index, e.generation);
            }
        }
        seen.into_iter()
            .map(|(idx, gen)| Entity::new(idx, gen))
            .collect()
    }

    pub fn query_1<A: Component>(&self) -> Vec<(Entity, &A)> {
        match self.storage_ref::<A>() {
            Some(s) => s.iter().collect(),
            None => Vec::new(),
        }
    }

    pub fn query_2<A: Component, B: Component>(&self) -> Vec<(Entity, &A, &B)> {
        let sa = match self.storage_ref::<A>() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let sb = match self.storage_ref::<B>() {
            Some(s) => s,
            None => return Vec::new(),
        };
        if sa.len() <= sb.len() {
            sa.iter()
                .filter_map(|(e, va)| sb.get(e).map(|vb| (e, va, vb)))
                .collect()
        } else {
            sb.iter()
                .filter_map(|(e, vb)| sa.get(e).map(|va| (e, va, vb)))
                .collect()
        }
    }

    pub fn query_3<A: Component, B: Component, C: Component>(&self) -> Vec<(Entity, &A, &B, &C)> {
        let sa = match self.storage_ref::<A>() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let sb = match self.storage_ref::<B>() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let sc = match self.storage_ref::<C>() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let smallest = [sa.len(), sb.len(), sc.len()]
            .into_iter()
            .enumerate()
            .min_by_key(|&(_, l)| l)
            .map(|(i, _)| i)
            .unwrap();
        match smallest {
            0 => sa
                .iter()
                .filter_map(|(e, _)| {
                    let va = sa.get(e)?;
                    let vb = sb.get(e)?;
                    let vc = sc.get(e)?;
                    Some((e, va, vb, vc))
                })
                .collect(),
            1 => sb
                .iter()
                .filter_map(|(e, _)| {
                    let va = sa.get(e)?;
                    let vb = sb.get(e)?;
                    let vc = sc.get(e)?;
                    Some((e, va, vb, vc))
                })
                .collect(),
            _ => sc
                .iter()
                .filter_map(|(e, _)| {
                    let va = sa.get(e)?;
                    let vb = sb.get(e)?;
                    let vc = sc.get(e)?;
                    Some((e, va, vb, vc))
                })
                .collect(),
        }
    }
}
