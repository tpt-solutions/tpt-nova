//! TPT Nova scene/level editor v1.
//!
//! The editor is intentionally **logic-only and framework-agnostic**: every
//! module here operates on the ECS [`World`](nova_ecs::World) and emits or
//! consumes plain data (selection, dotted field paths, gizmo drags, curve
//! points). The chosen front-end (egui, per the roadmap decision) renders these
//! models; the immediate-mode [`nova_ui`] draw list is used for the in-engine
//! panels so tooling and runtime UI share one primitive set.
//!
//! Modules:
//! - [`hierarchy`] — flatten the entity parent/child graph for the tree panel.
//! - [`inspector`] — read/write component fields by dotted path.
//! - [`gizmo`] — translate/rotate/scale the selection from pointer drags.
//! - [`vibe`] — the Bézier "Vibe GUI" that drives a physics parameter live.

pub mod gizmo;
pub mod hierarchy;
pub mod inspector;
pub mod vibe;

use nova_ecs::{Entity, World};
use nova_ui::{Rect, Ui, UiInput};

pub use gizmo::{apply_gizmo, GizmoMode, GizmoSnap};
pub use hierarchy::{build_hierarchy, HierarchyItem};
pub use inspector::{inspect_entity, set_field, ComponentInspection, Field};
pub use vibe::{BezierCurve, CurveEditor, GravityCurveBinding};

/// Aggregate editor state carried across frames.
#[derive(Debug, Clone, Default)]
pub struct EditorState {
    pub selected: Option<Entity>,
    pub gizmo_mode: GizmoMode,
    pub snap: GizmoSnap,
}

impl EditorState {
    pub fn new() -> Self {
        EditorState::default()
    }

    pub fn select(&mut self, entity: Entity) {
        self.selected = Some(entity);
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Cycle Move -> Rotate -> Scale -> Move.
    pub fn cycle_gizmo(&mut self) {
        self.gizmo_mode = match self.gizmo_mode {
            GizmoMode::Move => GizmoMode::Rotate,
            GizmoMode::Rotate => GizmoMode::Scale,
            GizmoMode::Scale => GizmoMode::Move,
        };
    }
}

/// Build a hierarchy panel draw list showing every entity, indented by depth,
/// with the selected row highlighted. Clicking a row updates `state.selected`.
///
/// Returns the immediate-mode draw list for the panel.
pub fn hierarchy_panel(
    world: &World,
    state: &mut EditorState,
    input: UiInput,
    area: Rect,
) -> nova_ui::DrawList {
    let items = build_hierarchy(world);
    let mut ui = Ui::new(input);
    ui.begin_panel(area, Some("Hierarchy"));
    for item in items {
        let indent = "  ".repeat(item.depth as usize);
        let marker = if item.has_children { "> " } else { "- " };
        let label = format!("{indent}{marker}{}", item.entity);
        let resp = ui.button(&label);
        if resp.clicked {
            state.select(item.entity);
        }
    }
    ui.end_panel();
    ui.finish()
}

/// Build an inspector panel draw list for the current selection.
pub fn inspector_panel(
    world: &World,
    state: &EditorState,
    input: UiInput,
    area: Rect,
) -> nova_ui::DrawList {
    let mut ui = Ui::new(input);
    ui.begin_panel(area, Some("Inspector"));
    match state.selected {
        None => {
            ui.label("(no selection)");
        }
        Some(e) => {
            ui.label(&format!("{e}"));
            for comp in inspect_entity(world, e) {
                ui.label(&comp.component);
                for field in comp.fields {
                    ui.label(&format!("  {} = {:.3}", field.path, field.value));
                }
            }
        }
    }
    ui.end_panel();
    ui.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use nova_ecs::transform::Transform;

    #[test]
    fn gizmo_mode_cycles() {
        let mut s = EditorState::new();
        assert_eq!(s.gizmo_mode, GizmoMode::Move);
        s.cycle_gizmo();
        assert_eq!(s.gizmo_mode, GizmoMode::Rotate);
        s.cycle_gizmo();
        assert_eq!(s.gizmo_mode, GizmoMode::Scale);
        s.cycle_gizmo();
        assert_eq!(s.gizmo_mode, GizmoMode::Move);
    }

    #[test]
    fn hierarchy_panel_selects_on_click() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());

        let mut state = EditorState::new();
        let area = Rect::from_min_size(Vec2::new(0.0, 0.0), Vec2::new(240.0, 400.0));

        // First pass with no click to discover the row's rect.
        let _ = hierarchy_panel(&world, &mut state, UiInput::default(), area);
        // The single entity's button sits just below the title bar + padding.
        // Click roughly there.
        let click = UiInput {
            pointer: Vec2::new(30.0, 45.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let _ = hierarchy_panel(&world, &mut state, click, area);
        assert_eq!(state.selected, Some(e));
    }

    #[test]
    fn inspector_panel_renders_for_selection() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        let mut state = EditorState::new();
        state.select(e);
        let area = Rect::from_min_size(Vec2::ZERO, Vec2::new(240.0, 400.0));
        let draw = inspector_panel(&world, &state, UiInput::default(), area);
        assert!(!draw.is_empty());
    }
}
