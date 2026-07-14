//! TPT Nova scene/level editor v1.
//!
//! The editor is intentionally **logic-only and framework-agnostic**: every
//! module here operates on the ECS [`World`](nova_ecs::World) and emits or
//! consumes plain data (selection, dotted field paths, gizmo drags, curve
//! points). The front-end is the bespoke immediate-mode [`nova_ui`] draw list
//! (not egui/eframe — an earlier roadmap decision named egui, but the shipped
//! implementation is a hand-rolled `Ui`/`DrawList` stack rendered by
//! `nova-render`'s `UiOverlay` pass), so tooling and runtime UI share one
//! primitive set.
//!
//! Modules:
//! - [`hierarchy`] — flatten the entity parent/child graph for the tree panel.
//! - [`inspector`] — read/write component fields by dotted path.
//! - [`gizmo`] — translate/rotate/scale the selection from pointer drags.
//! - [`vibe`] — the Bézier "Vibe GUI" that drives a physics parameter live.

pub mod gizmo;
pub mod gizmo3d;
pub mod hierarchy;
pub mod inspector;
pub mod vibe;

use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};
use nova_ui::{DragState, Rect, Ui, UiInput};
use std::collections::{HashMap, VecDeque};

pub use gizmo::{apply_gizmo, GizmoMode, GizmoSnap};
pub use gizmo3d::{apply_gizmo_3d, drag_plane_point, ray_plane, GizmoMode3D, Ray};
pub use hierarchy::{build_hierarchy, HierarchyItem};
pub use inspector::{inspect_entity, set_field, ComponentInspection, Field};
pub use vibe::{normalized_to_screen, BezierCurve, CurveEditor, GravityCurveBinding};

use std::collections::HashSet;

/// A recorded AI/external action for the editor's explainability panel.
///
/// `nova-agent-api` (or the in-editor "Highlight & Fix" flow) can attach an
/// optional `rationale` describing *why* a batch of [`nova_agent_api::AgentCommand`]s
/// was applied; the panel shows the last N actions so a human can audit what the
/// agent did and when.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentAction {
    /// A short human-readable summary (e.g. "move e0#1 +1.0 on x").
    pub summary: String,
    /// Optional free-text rationale supplied by the agent.
    pub rationale: Option<String>,
    /// The world tick at which the action was applied.
    pub tick: u64,
}

/// A bounded ring buffer of recent telemetry snapshots (stored as JSON strings),
/// enabling a mini "time-travel" debugger in the editor: scrub backward through
/// the last N world states without re-running the simulation.
#[derive(Debug, Clone, Default)]
pub struct TelemetryRing {
    cap: usize,
    frames: VecDeque<String>,
}

impl TelemetryRing {
    pub fn new(cap: usize) -> Self {
        TelemetryRing {
            cap: cap.max(1),
            frames: VecDeque::new(),
        }
    }

    /// Store the newest frame, dropping the oldest when over capacity.
    pub fn push(&mut self, frame_json: String) {
        self.frames.push_back(frame_json);
        while self.frames.len() > self.cap {
            self.frames.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The frame at `idx` where `0` is the oldest retained snapshot and
    /// `len() - 1` is the most recent.
    pub fn get(&self, idx: usize) -> Option<&String> {
        self.frames.get(idx)
    }

    /// The most recently stored frame, if any.
    pub fn latest(&self) -> Option<&String> {
        self.frames.back()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub name: String,
    /// Free-form kind label, e.g. "mesh", "splat", "scene".
    pub kind: String,
}

/// A single recorded edit, for undo/redo.
#[derive(Debug, Clone)]
enum EditRecord {
    /// A single scalar component-field edit (dotted path, see [`set_field`]).
    Field {
        entity: Entity,
        path: String,
        before: f32,
        after: f32,
    },
    /// A whole-`Transform` edit (e.g. a gizmo drag), stored as before/after
    /// snapshots so arbitrary rotation/scale/translation changes round-trip
    /// exactly on undo instead of only restoring a single scalar axis.
    Transform {
        entity: Entity,
        before: Transform,
        after: Transform,
    },
}

/// A bounded undo/redo stack of component-field edits.
#[derive(Debug, Clone)]
pub struct EditHistory {
    undo_stack: Vec<EditRecord>,
    redo_stack: Vec<EditRecord>,
    limit: usize,
}

impl Default for EditHistory {
    fn default() -> Self {
        EditHistory {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            limit: 200,
        }
    }
}

/// True if two transforms differ by more than an epsilon on any of
/// translation/rotation/scale. Used to skip no-op gizmo drags when recording
/// undo history.
fn transform_changed(a: &Transform, b: &Transform) -> bool {
    let eps = 1e-6;
    (a.translation - b.translation).length() > eps
        || (a.rotation - b.rotation).length() > eps
        || (a.scale - b.scale).length() > eps
}

impl EditHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a scalar field edit, pushing it onto the undo stack and clearing
    /// redo.
    pub fn record(&mut self, entity: Entity, path: impl Into<String>, before: f32, after: f32) {
        if (before - after).abs() < 1e-6 {
            return;
        }
        self.undo_stack.push(EditRecord::Field {
            entity,
            path: path.into(),
            before,
            after,
        });
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Record a whole-`Transform` edit (e.g. a gizmo drag), storing before/after
    /// snapshots so undo restores the exact previous pose.
    pub fn record_transform(&mut self, entity: Entity, before: Transform, after: Transform) {
        if !transform_changed(&before, &after) {
            return;
        }
        self.undo_stack.push(EditRecord::Transform {
            entity,
            before,
            after,
        });
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Undo the most recent edit, applying `before` back to the world.
    pub fn undo(&mut self, world: &mut World) -> bool {
        let rec = match self.undo_stack.pop() {
            Some(r) => r,
            None => return false,
        };
        match &rec {
            EditRecord::Field {
                entity,
                path,
                before,
                ..
            } => {
                let _ = set_field(world, *entity, path, *before);
            }
            EditRecord::Transform { entity, before, .. } => {
                if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                    *t = *before;
                }
            }
        }
        self.redo_stack.push(rec);
        true
    }

    /// Redo the most recently undone edit.
    pub fn redo(&mut self, world: &mut World) -> bool {
        let rec = match self.redo_stack.pop() {
            Some(r) => r,
            None => return false,
        };
        match &rec {
            EditRecord::Field {
                entity,
                path,
                after,
                ..
            } => {
                let _ = set_field(world, *entity, path, *after);
            }
            EditRecord::Transform { entity, after, .. } => {
                if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                    *t = *after;
                }
            }
        }
        self.undo_stack.push(rec);
        true
    }
}

/// Aggregate editor state carried across frames.
#[derive(Debug, Clone)]
pub struct EditorState {
    /// Primary selection (the gizmo target / inspector focus).
    pub selected: Option<Entity>,
    /// Full multi-selection set (includes `selected` when set).
    pub selection: HashSet<Entity>,
    pub gizmo_mode: GizmoMode,
    pub snap: GizmoSnap,
    /// Play-in-editor toggle: when true the simulation steps; when false the
    /// editor is in pure edit mode (selection/gizmos frozen in time).
    pub playing: bool,
    /// Assets registered with the editor, shown in the asset browser.
    pub assets: Vec<AssetEntry>,
    /// The asset currently selected in the browser (e.g. to drag into the scene).
    pub selected_asset: Option<String>,
    /// Undo/redo history of component-field edits.
    pub history: EditHistory,
    /// Per-field drag state for the inspector's `drag_float` widgets, keyed by
    /// dotted component path so a drag survives across the immediate-mode frame
    /// rebuilds.
    pub field_drag: HashMap<String, DragState>,
    /// The "Vibe GUI" Bézier curve editor (drives a physics parameter live).
    pub vibe: CurveEditor,
    /// Binding mapping the curve to a world gravity magnitude.
    pub vibe_binding: GravityCurveBinding,
    /// Which text field currently holds keyboard focus (pointer-only UI), if any.
    /// The focused field id (e.g. "fix_instruction") receives typed keystrokes
    /// via [`UiInput::text_entered`]; clicking a field toggles its focus.
    pub text_focus: Option<String>,
    /// Log of recent AI/external actions for the explainability panel.
    pub action_log: Vec<AgentAction>,
    /// Ring buffer of recent telemetry snapshots for the time-travel scrubber.
    pub telemetry_ring: TelemetryRing,
}

impl Default for EditorState {
    fn default() -> Self {
        EditorState {
            selected: None,
            selection: HashSet::new(),
            gizmo_mode: GizmoMode::Move,
            snap: GizmoSnap::default(),
            playing: false,
            assets: Vec::new(),
            selected_asset: None,
            history: EditHistory::default(),
            field_drag: HashMap::new(),
            vibe: CurveEditor::default(),
            vibe_binding: GravityCurveBinding::default(),
            text_focus: None,
            action_log: Vec::new(),
            telemetry_ring: TelemetryRing::new(300),
        }
    }
}

impl EditorState {
    pub fn new() -> Self {
        EditorState::default()
    }

    /// Select a single entity, replacing the current selection.
    pub fn select(&mut self, entity: Entity) {
        self.selected = Some(entity);
        self.selection.clear();
        self.selection.insert(entity);
    }

    /// Toggle `entity` in the selection (shift-click behavior). Keeps a primary
    /// `selected` of the most recently toggled-on entity.
    pub fn toggle_select(&mut self, entity: Entity) {
        if self.selection.contains(&entity) {
            self.selection.remove(&entity);
            if self.selected == Some(entity) {
                self.selected = self.selection.iter().next().copied();
            }
        } else {
            self.selection.insert(entity);
            self.selected = Some(entity);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.selection.clear();
    }

    /// Number of entities currently selected.
    pub fn selection_size(&self) -> usize {
        self.selection.len()
    }

    /// Cycle Move -> Rotate -> Scale -> Move.
    pub fn cycle_gizmo(&mut self) {
        self.gizmo_mode = match self.gizmo_mode {
            GizmoMode::Move => GizmoMode::Rotate,
            GizmoMode::Rotate => GizmoMode::Scale,
            GizmoMode::Scale => GizmoMode::Move,
        };
    }

    /// Toggle play-in-editor.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
    }

    /// Register an asset in the browser (de-duplicated by name).
    pub fn add_asset(&mut self, name: impl Into<String>, kind: impl Into<String>) {
        let name = name.into();
        if !self.assets.iter().any(|a| a.name == name) {
            self.assets.push(AssetEntry {
                name,
                kind: kind.into(),
            });
        }
    }

    /// Remove an asset from the browser by name.
    pub fn remove_asset(&mut self, name: &str) -> bool {
        let before = self.assets.len();
        self.assets.retain(|a| a.name != name);
        self.assets.len() != before
    }

    /// Record an AI/external action for the explainability panel. Keeps only the
    /// most recent 200 actions.
    pub fn record_action(&mut self, summary: impl Into<String>, rationale: Option<String>, tick: u64) {
        self.action_log.push(AgentAction {
            summary: summary.into(),
            rationale,
            tick,
        });
        if self.action_log.len() > 200 {
            self.action_log.remove(0);
        }
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
    shift: bool,
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
            // Shift-click toggles membership in the multi-selection; a plain
            // click replaces the selection with just this entity.
            if shift {
                state.toggle_select(item.entity);
            } else {
                state.select(item.entity);
            }
        }
    }
    ui.end_panel();
    ui.finish()
}

/// Suggest a sensible slider range for an inspector field path so the editable
/// widget stays within a reasonable band regardless of the component.
fn field_range(path: &str) -> (f32, f32) {
    if path.contains("rotation") || path.contains("angvel") {
        (-std::f32::consts::PI, std::f32::consts::PI)
    } else if path.contains("scale") {
        (0.01, 10.0)
    } else if path.contains("damping") || path.contains("gravity_scale") {
        (0.0, 10.0)
    } else if path.contains("linvel") {
        (-20.0, 20.0)
    } else {
        (-10.0, 10.0)
    }
}

/// Build an inspector panel draw list for the current selection.
///
/// Unlike the read-only hierarchy panel, this panel is **editable**: each field
/// is rendered as a slider and any change is written straight back into the
/// ECS component via [`set_field`], and recorded in `state.history` for
/// undo/redo. Pass `&mut World` so edits land in the live world.
pub fn inspector_panel(
    world: &mut World,
    state: &mut EditorState,
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
            let extra = state.selection_size().saturating_sub(1);
            if extra > 0 {
                ui.label(&format!("(+{extra} more selected)"));
            }
            for comp in inspect_entity(world, e) {
                ui.label(&comp.component);
                for field in comp.fields {
                    let mut val = field.value;
                    let (min, max) = field_range(&field.path);
                    // Editable widget: a drag-float whose speed maps a 200px drag
                    // to the full suggested range. The drag state lives in
                    // `state.field_drag` so the in-progress drag is stable across
                    // immediate-mode rebuilds.
                    let drag = state.field_drag.entry(field.path.clone()).or_default();
                    let speed = ((max - min) / 200.0).max(1e-4);
                    let changed =
                        ui.drag_float(&format!("  {}", field.path), &mut val, speed, drag);
                    if changed {
                        let before = field.value;
                        if set_field(world, e, &field.path, val) {
                            state.history.record(e, field.path.clone(), before, val);
                        }
                    }
                }
            }
        }
    }
    ui.end_panel();
    ui.finish()
}

/// Build a prompt-ready text block describing `entity` for an AI agent (or a
/// human clipboard), packaging its component state plus optional nearby context
/// (e.g. RAG-retrieved docs). Dependency-free: it reads component fields via the
/// [`inspect_entity`] registry so it stays correct as new components are added.
///
/// The result is plain text meant to be pasted into a chat or written to a file
/// by the host (the bespoke UI has no clipboard access of its own).
pub fn explain_entity(world: &World, entity: Entity, context: Option<&str>) -> String {
    let mut out = format!("Explain entity {entity}\n");
    out.push_str("Components:\n");
    let inspections = inspect_entity(world, entity);
    if inspections.is_empty() {
        out.push_str("  (none)\n");
    }
    for comp in &inspections {
        out.push_str(&format!("  {}:\n", comp.component));
        for f in &comp.fields {
            out.push_str(&format!("    {} = {:.4}\n", f.path, f.value));
        }
    }
    if let Some(ctx) = context {
        if !ctx.trim().is_empty() {
            out.push_str("\nRelevant context:\n");
            out.push_str(ctx);
            out.push('\n');
        }
    }
    out
}
/// "Play" toggle button at the top that reflects `state.playing`. Clicking an
/// asset row selects it (recorded in `state.selected_asset`); this is the
/// hook the editor uses to spawn/drag an asset into the viewport.
pub fn asset_browser_panel(
    state: &mut EditorState,
    input: UiInput,
    area: Rect,
) -> nova_ui::DrawList {
    let mut ui = Ui::new(input);
    ui.begin_panel(area, Some("Assets"));
    let mut playing = state.playing;
    let _ = ui.checkbox(if playing { "Playing" } else { "Paused" }, &mut playing);
    if playing != state.playing {
        state.toggle_play();
    }
    ui.label("Click an asset to select:");
    for asset in &state.assets {
        let resp = ui.button(&format!("{} ({})", asset.name, asset.kind));
        if resp.clicked {
            state.selected_asset = Some(asset.name.clone());
        }
    }
    if state.assets.is_empty() {
        ui.label("(no assets registered)");
    }
    ui.end_panel();
    ui.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use nova_ecs::transform::Transform;
    use nova_ecs::Vec3;

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
        let _ = hierarchy_panel(&world, &mut state, UiInput::default(), area, false);
        // The single entity's button sits just below the title bar + padding.
        // Click roughly there.
        let click = UiInput {
            pointer: Vec2::new(30.0, 45.0),
            pointer_down: true,
            pointer_pressed: true,
            ..Default::default()
        };
        let _ = hierarchy_panel(&world, &mut state, click, area, false);
        assert_eq!(state.selected, Some(e));
    }

    #[test]
    fn hierarchy_panel_shift_click_multi_selects() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Transform::default());
        world.add_component(b, Transform::default());

        let mut state = EditorState::new();
        let area = Rect::from_min_size(Vec2::new(0.0, 0.0), Vec2::new(240.0, 400.0));
        // Select `a` first with a plain click.
        let click_a = UiInput {
            pointer: Vec2::new(30.0, 45.0),
            pointer_down: true,
            pointer_pressed: true,
            ..Default::default()
        };
        let _ = hierarchy_panel(&world, &mut state, click_a, area, false);
        assert_eq!(state.selected, Some(a));

        // Shift-click `b` to add it to the selection.
        let click_b = UiInput {
            pointer: Vec2::new(30.0, 71.0),
            pointer_down: true,
            pointer_pressed: true,
            ..Default::default()
        };
        let _ = hierarchy_panel(&world, &mut state, click_b, area, true);
        assert_eq!(state.selection_size(), 2);
        assert!(state.selection.contains(&b));
    }

    #[test]
    fn inspector_panel_renders_for_selection() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        let mut state = EditorState::new();
        state.select(e);
        let area = Rect::from_min_size(Vec2::ZERO, Vec2::new(240.0, 400.0));
        let draw = inspector_panel(&mut world, &mut state, UiInput::default(), area);
        assert!(!draw.is_empty());
    }

    #[test]
    fn inspector_slider_writes_field_and_records_history() {
        // Editable (drag-float) edits must round-trip into the world *and* be
        // undoable.
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
        let mut state = EditorState::new();
        state.select(e);

        let area = Rect::from_min_size(Vec2::new(40.0, 40.0), Vec2::new(240.0, 400.0));
        // Press inside the first field row (Transform.translation.x), then drag
        // 150px to the right while held. The row starts below the title bar
        // (22px), the entity label, and the "Transform" component label — roughly
        // 134px down. translation range is (-10, 10) -> drag speed 0.1/px, so the
        // value should jump by ~15.
        let press = UiInput {
            pointer: Vec2::new(60.0, 134.0),
            pointer_down: true,
            pointer_pressed: true,
            ..Default::default()
        };
        let _ = inspector_panel(&mut world, &mut state, press, area);
        let drag = UiInput {
            pointer: Vec2::new(210.0, 134.0),
            pointer_down: true,
            pointer_pressed: false,
            ..Default::default()
        };
        let _ = inspector_panel(&mut world, &mut state, drag, area);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!(
            t.translation.x > 5.0,
            "dragging translation.x to the right must raise x, got {}",
            t.translation.x
        );
        assert!(state.history.can_undo());

        // Undo restores the original value.
        state.history.undo(&mut world);
        let t2 = world.get_component::<Transform>(e).unwrap();
        assert!((t2.translation.x - 1.0).abs() < 1e-3);
        assert!(state.history.can_redo());
    }

    #[test]
    fn multi_select_toggles_and_reports_size() {
        let mut s = EditorState::new();
        let a = Entity {
            index: 0,
            generation: 0,
        };
        let b = Entity {
            index: 1,
            generation: 0,
        };
        s.select(a);
        assert_eq!(s.selection_size(), 1);
        s.toggle_select(b);
        assert_eq!(s.selection_size(), 2);
        assert_eq!(s.selected, Some(b));
        s.toggle_select(b);
        assert_eq!(s.selection_size(), 1);
        assert_eq!(s.selected, Some(a));
    }

    #[test]
    fn play_toggle_and_asset_browser() {
        let mut s = EditorState::new();
        assert!(!s.playing);
        s.toggle_play();
        assert!(s.playing);
        s.add_asset("cube.glb", "mesh");
        s.add_asset("cube.glb", "mesh"); // de-dup
        s.add_asset("park.splat", "splat");
        assert_eq!(s.assets.len(), 2);
        assert!(s.remove_asset("cube.glb"));
        assert_eq!(s.assets.len(), 1);

        let mut state = EditorState::new();
        state.add_asset("cube.glb", "mesh");
        let area = Rect::from_min_size(Vec2::new(40.0, 40.0), Vec2::new(240.0, 400.0));
        let draw = asset_browser_panel(&mut state, UiInput::default(), area);
        assert!(!draw.is_empty());
    }

    #[test]
    fn transform_undo_restores_pose() {
        // A gizmo drag records a whole-Transform edit; undo must restore the
        // exact pre-drag pose, including arbitrary rotation/scale.
        let mut world = World::new();
        let e = world.spawn();
        let before = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
        world.add_component(e, before);

        let mut state = EditorState::new();
        let after = Transform::new(
            Vec3::new(4.0, 5.0, 6.0),
            nova_ecs::Quat::from_rotation_y(0.7),
            Vec3::new(2.0, 2.0, 2.0),
        );
        state.history.record_transform(e, before, after);
        assert!(state.history.can_undo());

        // Apply the "after" pose, then undo back to "before".
        *world.get_component_mut::<Transform>(e).unwrap() = after;
        state.history.undo(&mut world);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!((t.translation - before.translation).length() < 1e-5);
        assert!((t.rotation - before.rotation).length() < 1e-5);
        assert!((t.scale - before.scale).length() < 1e-5);

        // And redo re-applies "after".
        state.history.redo(&mut world);
        let t2 = world.get_component::<Transform>(e).unwrap();
        assert!((t2.translation - after.translation).length() < 1e-5);
    }

    #[test]
    fn transform_noop_drag_is_not_recorded() {
        let mut state = EditorState::new();
        let before = Transform::from_translation(Vec3::new(1.0, 0.0, 0.0));
        state.history.record_transform(
            Entity {
                index: 0,
                generation: 0,
            },
            before,
            before,
        );
        assert!(!state.history.can_undo());
    }

    #[test]
    fn telemetry_ring_keeps_most_recent_within_cap() {
        let mut ring = TelemetryRing::new(3);
        assert!(ring.is_empty());
        ring.push("a".to_string());
        ring.push("b".to_string());
        ring.push("c".to_string());
        ring.push("d".to_string());
        assert_eq!(ring.len(), 3);
        // Oldest ("a") evicted; newest is "d".
        assert_eq!(ring.get(0), Some(&"b".to_string()));
        assert_eq!(ring.latest(), Some(&"d".to_string()));
    }

    #[test]
    fn record_action_appends_and_caps_log() {
        let mut s = EditorState::new();
        assert!(s.action_log.is_empty());
        s.record_action("move x", Some("user dragged".into()), 10);
        assert_eq!(s.action_log.len(), 1);
        assert_eq!(s.action_log[0].summary, "move x");
        assert_eq!(s.action_log[0].tick, 10);
        assert_eq!(s.action_log[0].rationale.as_deref(), Some("user dragged"));

        for i in 0..250 {
            s.record_action(format!("act{i}"), None, i as u64);
        }
        assert_eq!(s.action_log.len(), 200, "log capped at 200");
    }

    #[test]
    fn explain_entity_packages_components_and_context() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let text = explain_entity(&world, e, Some("see docs/foo.md"));
        assert!(text.contains(&format!("Explain entity {e}")));
        assert!(text.contains("Transform.translation.x = 1.0000"));
        assert!(text.contains("see docs/foo.md"));
    }
}
