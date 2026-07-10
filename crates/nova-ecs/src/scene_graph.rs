//! Scene-graph hierarchy components and world-transform propagation.

use crate::component::Component;
use crate::entity::Entity;
use crate::world::World;
use serde::{Deserialize, Serialize};

/// A parent link: this entity is a child of `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent(pub Entity);

/// The set of children owned by this entity.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Children(pub Vec<Entity>);

impl Component for Parent {}
impl Component for Children {}

/// Recompute every entity's [`GlobalTransform`] from its local [`Transform`] and
/// its parent chain.
///
/// Roots (entities with a `Transform` but no `Parent`) start from the identity
/// matrix; children multiply their local matrix onto their parent's world
/// matrix via an explicit depth-first walk so parents are always resolved
/// before children.
pub fn propagate_transforms(world: &mut World) {
    // Snapshot parent/child relationships to avoid borrow conflicts.
    let mut parent_of: Vec<(Entity, Entity)> = Vec::new();
    let mut children_of: Vec<(Entity, Vec<Entity>)> = Vec::new();

    for (e, p) in world.query_1::<Parent>() {
        parent_of.push((e, p.0));
    }
    for (e, c) in world.query_1::<Children>() {
        children_of.push((e, c.0.clone()));
    }

    // Map child -> parent.
    let parent: std::collections::HashMap<Entity, Entity> = parent_of.into_iter().collect();

    // Collect all entities that have a Transform.
    let nodes: Vec<Entity> = world
        .query_1::<crate::transform::Transform>()
        .into_iter()
        .map(|(e, _)| e)
        .collect();

    // Roots = nodes without a parent.
    let roots: Vec<Entity> = nodes
        .iter()
        .copied()
        .filter(|e| !parent.contains_key(e))
        .collect();

    let mut child_map: std::collections::HashMap<Entity, Vec<Entity>> =
        std::collections::HashMap::new();
    for (e, kids) in children_of {
        child_map.insert(e, kids);
    }

    // Ensure every Transform-bearing entity has a GlobalTransform.
    for e in &nodes {
        if !world.has_component::<crate::transform::GlobalTransform>(*e) {
            world.add_component(*e, crate::transform::GlobalTransform::identity());
        }
    }

    fn visit(
        world: &mut World,
        entity: Entity,
        parent_global: crate::math::Mat4,
        parent: &std::collections::HashMap<Entity, Entity>,
        child_map: &std::collections::HashMap<Entity, Vec<Entity>>,
    ) {
        let local = match world.get_component::<crate::transform::Transform>(entity) {
            Some(t) => t.matrix(),
            None => return,
        };
        let global = parent_global * local;
        if let Some(g) = world.get_component_mut::<crate::transform::GlobalTransform>(entity) {
            g.0 = global;
        }
        if let Some(kids) = child_map.get(&entity) {
            for &child in kids {
                // Only descend if the child actually points back at us (guards
                // against stale links).
                if parent.get(&child) == Some(&entity) {
                    visit(world, child, global, parent, child_map);
                }
            }
        }
    }

    for root in roots {
        visit(
            world,
            root,
            crate::math::Mat4::IDENTITY,
            &parent,
            &child_map,
        );
    }
}
