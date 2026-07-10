//! Scene hierarchy: flatten the ECS parent/child graph into a displayable tree.

use nova_ecs::scene_graph::{Children, Parent};
use nova_ecs::{Entity, World};

/// One row in the hierarchy panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HierarchyItem {
    pub entity: Entity,
    /// Indentation depth (0 = root).
    pub depth: u32,
    pub has_children: bool,
}

/// Build a depth-first, indentation-annotated list of all entities, roots first.
///
/// An entity is a root if it has no `Parent`. Children are ordered by their
/// parent's `Children` list when present, otherwise by entity index for
/// determinism. Entities unreachable from any root (e.g. orphaned children with
/// a dangling parent) are appended at the end so nothing is hidden from the
/// editor.
pub fn build_hierarchy(world: &World) -> Vec<HierarchyItem> {
    let all: Vec<Entity> = {
        let mut v = world.entities();
        v.sort();
        v
    };

    let child_list = |e: Entity| -> Vec<Entity> {
        if let Some(c) = world.get_component::<Children>(e) {
            c.0.clone()
        } else {
            // Fall back to scanning Parent links.
            let mut kids: Vec<Entity> = all
                .iter()
                .copied()
                .filter(|&c| world.get_component::<Parent>(c).map(|p| p.0) == Some(e))
                .collect();
            kids.sort();
            kids
        }
    };

    let mut out = Vec::new();
    let mut visited = std::collections::HashSet::new();

    fn visit(
        e: Entity,
        depth: u32,
        out: &mut Vec<HierarchyItem>,
        visited: &mut std::collections::HashSet<Entity>,
        child_list: &dyn Fn(Entity) -> Vec<Entity>,
    ) {
        if !visited.insert(e) {
            return; // guard against cycles
        }
        let kids = child_list(e);
        out.push(HierarchyItem {
            entity: e,
            depth,
            has_children: !kids.is_empty(),
        });
        for k in kids {
            visit(k, depth + 1, out, visited, child_list);
        }
    }

    for &e in &all {
        let is_root = world.get_component::<Parent>(e).is_none();
        if is_root {
            visit(e, 0, &mut out, &mut visited, &child_list);
        }
    }
    // Append anything not yet visited (orphans / dangling parents).
    for &e in &all {
        if !visited.contains(&e) {
            visit(e, 0, &mut out, &mut visited, &child_list);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::Transform;

    #[test]
    fn flattens_parent_child_tree_depth_first() {
        let mut world = World::new();
        let root = world.spawn();
        world.add_component(root, Transform::default());
        let child = world.spawn();
        world.add_component(child, Transform::default());
        let grandchild = world.spawn();
        world.add_component(grandchild, Transform::default());

        world.add_component(child, Parent(root));
        world.add_component(root, Children(vec![child]));
        world.add_component(grandchild, Parent(child));
        world.add_component(child, Children(vec![grandchild]));

        let items = build_hierarchy(&world);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].entity, root);
        assert_eq!(items[0].depth, 0);
        assert!(items[0].has_children);
        assert_eq!(items[1].entity, child);
        assert_eq!(items[1].depth, 1);
        assert_eq!(items[2].entity, grandchild);
        assert_eq!(items[2].depth, 2);
        assert!(!items[2].has_children);
    }
}
