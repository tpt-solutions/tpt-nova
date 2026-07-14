//! Headless `nova-editor` demo: build editor state, manage selection and
//! assets, record + undo a `Transform` edit, and render the hierarchy /
//! inspector panels to draw lists — all without a window.
//!
//! Run with: `cargo run -p nova-editor --example editor_state`

use glam::Vec2;
use nova_ecs::transform::Transform;
use nova_ecs::{Quat, Vec3, World};
use nova_editor::{
    asset_browser_panel, hierarchy_panel, inspector_panel, AssetEntry, EditorState, GizmoMode,
};
use nova_ui::{Rect, UiInput};

fn main() {
    // --- A tiny world with two entities ------------------------------------
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    let b = world.spawn();
    world.add_component(b, Transform::default());

    let mut state = EditorState::new();

    // --- Selection + gizmo mode --------------------------------------------
    state.select(a);
    assert_eq!(state.selected, Some(a));
    assert_eq!(state.selection_size(), 1);

    state.toggle_select(b);
    assert_eq!(state.selection_size(), 2);
    state.cycle_gizmo();
    assert_eq!(state.gizmo_mode, GizmoMode::Rotate);

    // --- Assets ------------------------------------------------------------
    state.add_asset("cube.glb", "mesh");
    state.add_asset("cube.glb", "mesh"); // de-duped
    state.add_asset("park.splat", "splat");
    assert_eq!(state.assets.len(), 2);
    let _asset = AssetEntry {
        name: "cube.glb".into(),
        kind: "mesh".into(),
    };
    assert!(state.remove_asset("cube.glb"));
    assert_eq!(state.assets.len(), 1);

    // --- Record + undo a whole-Transform edit ------------------------------
    let before = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
    *world.get_component_mut::<Transform>(a).unwrap() = before;
    let after = Transform::new(
        Vec3::new(4.0, 5.0, 6.0),
        Quat::from_rotation_y(0.7),
        Vec3::new(2.0, 2.0, 2.0),
    );
    state.history.record_transform(a, before, after);
    assert!(state.history.can_undo());

    *world.get_component_mut::<Transform>(a).unwrap() = after;
    state.history.undo(&mut world);
    let restored = world.get_component::<Transform>(a).unwrap();
    assert!((restored.translation - before.translation).length() < 1e-5);
    assert!(state.history.can_redo());

    // --- Render panels headlessly and assert they emit draw primitives -----
    let area = Rect::from_min_size(Vec2::ZERO, Vec2::new(240.0, 400.0));

    let hier = hierarchy_panel(&world, &mut state, UiInput::default(), area, false);
    assert!(!hier.is_empty(), "hierarchy panel must draw");

    // Re-select `a` so the inspector has something to show.
    state.select(a);
    let insp = inspector_panel(&mut world, &mut state, UiInput::default(), area);
    assert!(!insp.is_empty(), "inspector panel must draw");

    let mut state2 = EditorState::new();
    state2.add_asset("cube.glb", "mesh");
    let assets = asset_browser_panel(&mut state2, UiInput::default(), area);
    assert!(!assets.is_empty(), "asset browser must draw");

    println!(
        "editor_state: {} entities, gizmo={:?}, history undoable={}",
        world.entity_count(),
        state.gizmo_mode,
        state.history.can_undo()
    );
    println!("editor_state: OK — panels produced non-empty draw lists");
}
