//! TPT Nova application shell.
//!
//! Boots a winit window, builds the ECS world (a cube + camera), and runs a
//! deterministic fixed-timestep loop. Input is mapped to actions; an external
//! AI agent can hot-apply changes by writing `nova-control.json`, which the
//! engine polls each tick. Telemetry is dumped to `nova-telemetry.json` on an
//! interval so the agent can observe and self-correct.
//!
//! On top of the 3D scene the engine now renders a live **editor UI** built from
//! [`nova_ui`] draw lists and composited by [`nova_render`]'s `UiOverlay` pass:
//! a hierarchy panel, a component inspector, an asset browser, a toolbar, and a
//! 3D viewport where pointer drags drive the gizmo and a marquee drives the
//! "Highlight & Fix" overlay. This is what makes the engine usable by a human,
//! not just a set of logic layers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{EulerRot, Vec2, Vec3};
use nova_ecs::scheduler::Scheduler;
use nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind, Transform};
use nova_ecs::{Entity, Mat4, Quat, World};
use nova_editor::{
    asset_browser_panel, hierarchy_panel, inspector_panel, normalized_to_screen, EditorState,
};
use nova_input::{default_action_map, ActionMap, InputState};
use nova_neural_materials::prompt::{FeedSource, MaterialPrompt};
use nova_neural_materials::{MaterialBindings, NeuralMaterialRegistry};
use nova_overlay::{project_to_screen, SelectionTool};
use nova_render::Renderer;
use nova_scene;
use nova_telemetry::{FileSink, TelemetryEmitter};
use nova_ui::{Color, DrawCommand, DrawList, Rect, Ui, UiInput};
use serde::Deserialize;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent as WinitWindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes};

const FIXED_DT: f32 = 1.0 / 60.0;
const TELEMETRY_INTERVAL: u64 = 30; // emit every 30 ticks (~0.5s)
const AUTOSAVE_INTERVAL: u64 = 300; // autosave every 300 ticks (~5s)
const CONTROL_PATH: &str = "nova-control.json";

/// Which pointer tool is active in the 3D viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportTool {
    /// Drag the selection with the translate/rotate/scale gizmo.
    Gizmo,
    /// Drag a rectangular "Highlight & Fix" marquee to build an AI fix request.
    Highlight,
}

/// Tracks the simulation tick for systems and telemetry.
#[derive(Debug, Clone)]
pub struct TickResource {
    pub tick: u64,
}

/// A control file written by an external AI agent.
#[derive(Debug, Deserialize, Default)]
struct ControlFile {
    #[serde(default)]
    set_rotation: Option<RotationXYZ>,
}

#[derive(Debug, Deserialize, Default)]
struct RotationXYZ {
    x: f32,
    y: f32,
    z: f32,
}

/// Panel layout (in window pixels) for the editor chrome. Pure so it can be
/// unit-tested without a window.
pub(crate) fn editor_layout(size: (u32, u32)) -> EditorLayout {
    let w = size.0 as f32;
    let h = size.1 as f32;
    let center_w = (w - 600.0).max(1.0);
    EditorLayout {
        toolbar: Rect::from_min_size(Vec2::ZERO, Vec2::new(w, 30.0)),
        hierarchy: Rect::from_min_size(Vec2::new(8.0, 38.0), Vec2::new(260.0, h - 196.0)),
        inspector: Rect::from_min_size(Vec2::new(w - 300.0, 38.0), Vec2::new(284.0, h - 46.0)),
        assets: Rect::from_min_size(Vec2::new(8.0, h - 150.0), Vec2::new(260.0, 140.0)),
        // The "Highlight & Fix" instruction field, shown across the top-center
        // while the Highlight tool is active.
        instruction: Rect::from_min_size(Vec2::new(280.0, 34.0), Vec2::new(center_w, 26.0)),
        // The "Vibe GUI" Bézier curve editor, along the bottom-center strip
        // (above the agent/telemetry panel).
        vibe: Rect::from_min_size(Vec2::new(280.0, h - 270.0), Vec2::new(center_w, 100.0)),
        // Agent explainability log + telemetry time-travel scrubber, along the
        // bottom-center strip beside the asset browser.
        agent_panel: Rect::from_min_size(Vec2::new(280.0, h - 150.0), Vec2::new(center_w, 140.0)),
        // The interactive 3D region: the full window minus the side/bottom panels
        // and the vibe strip at the bottom.
        viewport: Rect::from_min_size(
            Vec2::new(280.0, 62.0),
            Vec2::new(center_w, (h - 150.0 - 62.0).max(1.0)),
        ),
    }
}

/// Editor chrome rectangles in window pixel space.
pub(crate) struct EditorLayout {
    toolbar: Rect,
    hierarchy: Rect,
    inspector: Rect,
    assets: Rect,
    instruction: Rect,
    vibe: Rect,
    agent_panel: Rect,
    viewport: Rect,
}

impl EditorLayout {
    /// True if a window-space pointer is over any editor panel (and therefore
    /// should not be treated as a 3D viewport interaction). Anything outside the
    /// central viewport rectangle (the gutters between panels) also counts as
    /// non-interactive so clicks there don't fall through to the 3D scene.
    fn over_panel(&self, p: Vec2) -> bool {
        !self.viewport.contains(p)
            || self.toolbar.contains(p)
            || self.hierarchy.contains(p)
            || self.inspector.contains(p)
            || self.assets.contains(p)
            || self.instruction.contains(p)
            || self.vibe.contains(p)
            || self.agent_panel.contains(p)
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    world: World,
    scheduler: Scheduler,
    emitter: TelemetryEmitter<FileSink>,
    cube: Entity,
    #[allow(dead_code)]
    camera: Entity,
    last_time: Instant,
    accumulator: f32,
    control_mtime: Option<u64>,
    #[allow(dead_code)]
    telemetry_path: PathBuf,
    control_path: String,

    // ---- Editor state ----------------------------------------------------
    editor_enabled: bool,
    editor: EditorState,
    tool: ViewportTool,
    /// 3D gizmo mode (Move/Rotate/Scale), independent of the 2D editor's mode.
    gizmo_mode: nova_editor::GizmoMode3D,
    overlay: SelectionTool,
    /// Live window-space pointer (pixels, y-down).
    pointer: Vec2,
    pointer_down: bool,
    /// A press edge that is true for exactly the frame after a mouse-down.
    pointer_pressed: bool,
    /// Window size in pixels, kept in sync with `Resized`.
    viewport_size: (u32, u32),
    /// Current keyboard modifier state (tracked via `ModifiersChanged`).
    modifiers: ModifiersState,
    /// Active gizmo drag: (start pointer, entity, transform at drag start). The
    /// pre-drag transform is kept so the drag can be recorded into the undo
    /// history on release.
    gizmo_drag: Option<(Vec2, Entity, Transform)>,
    /// The most recent "Highlight & Fix" request, kept for display.
    last_fix: Option<nova_overlay::AiFixRequest>,
    /// The dolly distance of the editor camera from the origin, driven by the
    /// mouse wheel (viewport zoom).
    camera_distance: f32,
    /// The "Highlight & Fix" natural-language instruction typed by the user
    /// (replaces the previously-hardcoded literal). Edited via the in-viewport
    /// text field; fed to the fix request on release.
    fix_instruction: String,
    /// "Propose" mode: when on, a built Highlight & Fix request is shown as a
    /// translucent ghost preview with Accept/Reject instead of being applied
    /// immediately. Lets a human vet an agent action before it mutates the world.
    propose_mode: bool,
    /// A fix request awaiting Accept/Reject while `propose_mode` is active.
    pending_fix: Option<nova_overlay::AiFixRequest>,
    /// The neural-material registry (live video-LLM feeds) and its ECS bindings.
    /// Demonstrates the PBR binding contract: a material id is bound to the cube
    /// entity so each frame's latest decoded frame can drive its surface.
    neural: NeuralMaterialRegistry,
    bindings: MaterialBindings,
    /// Optional autosave path (enabled via the `NOVA_AUTOSAVE` env var). When set,
    /// the world is periodically serialized with `nova-scene` and, if
    /// `NOVA_RESTORE` is also set, reloaded on the next launch.
    autosave_path: Option<PathBuf>,
    /// Selected index into `editor.telemetry_ring` for the time-travel scrubber.
    scrub_index: Option<usize>,
}

impl App {
    /// Build the app with explicit telemetry/control paths (used by tests to
    /// keep the AI code-injection loop hermetic).
    fn new_with_paths(seed: u64, telemetry_path: PathBuf, control_path: String) -> Self {
        let mut world = World::new();

        // Cube entity.
        let cube = world.spawn();
        world.add_component(cube, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
        world.add_component(
            cube,
            Mesh {
                kind: MeshKind::Cube,
            },
        );
        world.add_component(cube, GlobalTransform::identity());

        // Camera entity, pulled back along +Z looking at the origin.
        let camera = world.spawn();
        world.add_component(
            camera,
            Transform::from_translation(Vec3::new(0.0, 0.0, 3.5)),
        );
        world.add_component(camera, Camera::default());
        world.add_component(camera, GlobalTransform::identity());

        // Resources.
        world.add_resource(InputState::default());
        world.add_resource(default_action_map());
        world.add_resource(nova_ecs::rng::RngResource::new(seed));
        world.add_resource(TickResource { tick: 0 });

        // Build the deterministic schedule.
        let mut scheduler = Scheduler::new();
        let cube_e = cube;
        scheduler.add_system(move |w: &mut World| movement_system(w, cube_e));
        scheduler.add_system(move |w: &mut World| nova_ecs::scene_graph::propagate_transforms(w));

        let mut editor = EditorState::new();
        editor.add_asset("cube.glb", "mesh");
        editor.add_asset("park.splat", "splat");

        // PBR neural-material binding demo: register a "video" feed (MockProvider
        // by default, swap for a real Video LLM via `NeuralMaterialRegistry::
        // set_provider`) and bind it to the cube so its latest decoded frame can
        // drive the cube's surface each tick. The actual GPU texture upload/bind
        // lives in `nova-neural-materials` (`NeuralTexture` + `MaterialBinding`);
        // here we resolve the bound frame every tick to prove the link is live.
        let mut neural = NeuralMaterialRegistry::default();
        let _ = neural.register(
            MaterialPrompt::new("video", "ambient billboard", FeedSource::CaptureDevice(0))
                .with_resolution(8, 8),
        );
        let mut bindings = MaterialBindings::new();
        bindings.bind("video", cube);

        let autosave_path = std::env::var("NOVA_AUTOSAVE")
            .ok()
            .map(PathBuf::from);

        let mut app = App {
            window: None,
            renderer: None,
            world,
            scheduler,
            emitter: TelemetryEmitter::new(
                FileSink::new(telemetry_path.clone()),
                TELEMETRY_INTERVAL,
            ),
            cube,
            camera,
            last_time: Instant::now(),
            accumulator: 0.0,
            control_mtime: None,
            telemetry_path,
            control_path,
            editor_enabled: true,
            editor,
            tool: ViewportTool::Gizmo,
            gizmo_mode: nova_editor::GizmoMode3D::Move,
            overlay: SelectionTool::new(),
            pointer: Vec2::ZERO,
            pointer_down: false,
            pointer_pressed: false,
            viewport_size: (1280, 720),
            modifiers: ModifiersState::empty(),
            gizmo_drag: None,
            last_fix: None,
            camera_distance: 3.5,
            fix_instruction: String::from("fix selection"),
            propose_mode: false,
            pending_fix: None,
            neural,
            bindings,
            autosave_path,
            scrub_index: None,
        };
        if std::env::var("NOVA_RESTORE").is_ok() {
            app.restore_session();
        }
        app
    }

    /// Reload the most recently autosaved world (opt-in via the `NOVA_RESTORE`
    /// env var) in place, re-binding the cube/camera and rebuilding the
    /// scheduler. No-op (with a warning) if no autosave exists or it is missing
    /// the cube/camera entities. Tests never enable `NOVA_RESTORE`, so the
    /// hermetic seeded world is preserved.
    fn restore_session(&mut self) {
        let Some(path) = self.autosave_path.clone() else {
            return;
        };
        if !path.exists() {
            return;
        }
        let Ok(world) = nova_scene::load_from_file(&path) else {
            log::warn!("session restore failed to load {}", path.display());
            return;
        };
        let seed = self
            .world
            .resource::<nova_ecs::rng::RngResource>()
            .map(|r| r.seed)
            .unwrap_or(0);
        let mut world = world;
        world.add_resource(InputState::default());
        world.add_resource(default_action_map());
        world.add_resource(nova_ecs::rng::RngResource::new(seed));
        world.add_resource(TickResource { tick: 0 });

        let cube = world.query_1::<Mesh>().into_iter().next().map(|(e, _)| e);
        let camera = world.query_1::<Camera>().into_iter().next().map(|(e, _)| e);
        let (cube, camera) = match (cube, camera) {
            (Some(c), Some(cam)) => (c, cam),
            _ => {
                log::warn!("autosave missing cube/camera; ignoring restore");
                return;
            }
        };
        let mut scheduler = Scheduler::new();
        scheduler.add_system(move |w: &mut World| movement_system(w, cube));
        scheduler
            .add_system(move |w: &mut World| nova_ecs::scene_graph::propagate_transforms(w));

        self.world = world;
        self.scheduler = scheduler;
        self.cube = cube;
        self.camera = camera;
        log::info!("restored session from {}", path.display());
    }

    fn new(seed: u64) -> Self {
        Self::new_with_paths(
            seed,
            PathBuf::from("nova-telemetry.json"),
            CONTROL_PATH.to_string(),
        )
    }

    /// Compute the camera view-projection (and its inverse + forward) for the
    /// current world, matching what [`nova_render`] uses so gizmo math lines up
    /// with the rendered scene. Returns `None` when there is no camera.
    fn camera_matrices(&self) -> Option<(Mat4, Mat4, Vec3)> {
        let aspect = self.viewport_size.0 as f32 / self.viewport_size.1.max(1) as f32;
        self.world
            .query_2::<Camera, GlobalTransform>()
            .into_iter()
            .next()
            .map(|(_e, cam, gt)| {
                let mut proj = *cam;
                proj.aspect = aspect;
                let view = gt.0.inverse();
                let vp = proj.perspective() * view;
                let forward = gt.0.transform_vector3(Vec3::NEG_Z).normalize();
                (vp, vp.inverse(), forward)
            })
    }

    /// Pick the entity whose projected center is nearest the window pointer
    /// (within `radius` pixels), used for click-to-select in the viewport.
    fn pick_entity(&self, vp: Mat4, radius: f32) -> Option<Entity> {
        let mut best: Option<(f32, Entity)> = None;
        for (e, _t, gt) in self
            .world
            .query_2::<Transform, GlobalTransform>()
            .into_iter()
        {
            if let Some((sx, sy)) = project_to_screen(gt.translation(), vp, self.viewport_size) {
                let d = ((sx as f32 - self.pointer.x).powi(2)
                    + (sy as f32 - self.pointer.y).powi(2))
                .sqrt();
                if d <= radius && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, e));
                }
            }
        }
        best.map(|(_, e)| e)
    }

    fn step(&mut self) {
        // External control override (hot-apply without restart).
        apply_control(
            &mut self.world,
            self.cube,
            &mut self.control_mtime,
            &self.control_path,
        );

        self.scheduler.run(&mut self.world);

        self.pump_neural_and_emit();
    }

    /// A "paused" simulation step for play-in-editor: still hot-applies external
    /// control and keeps transforms propagated (so gizmo edits show) but does not
    /// run gameplay systems.
    fn step_paused(&mut self) {
        apply_control(
            &mut self.world,
            self.cube,
            &mut self.control_mtime,
            &self.control_path,
        );
        nova_ecs::scene_graph::propagate_transforms(&mut self.world);
        self.pump_neural_and_emit();
    }

    /// Advance the neural-material feeds and emit telemetry (also capturing the
    /// emitted frame into the editor's time-travel ring buffer).
    fn pump_neural_and_emit(&mut self) {
        // Poll the live video-LLM feeds so the bound material's latest frame
        // stays current; a renderer would upload this onto the cube's PBR
        // material via `MaterialBinding::latest_frame` + `NeuralTexture`.
        self.neural.update();

        let tick = {
            let t = self.world.resource_mut::<TickResource>().unwrap();
            t.tick += 1;
            t.tick
        };
        let seed = self
            .world
            .resource::<nova_ecs::rng::RngResource>()
            .map(|r| r.seed)
            .unwrap_or(0);

        // Capture every emitted frame into the ring buffer for the scrubber.
        if self.emitter.maybe_emit(&self.world, tick, seed).unwrap_or(false) {
            if let Ok(json) = serde_json::to_string(
                &nova_telemetry::dump_world(&self.world, tick, seed),
            ) {
                self.editor.telemetry_ring.push(json);
            }
        }

        // Periodic autosave (opt-in via NOVA_AUTOSAVE env) of the in-memory world.
        if let Some(path) = &self.autosave_path {
            if tick % AUTOSAVE_INTERVAL == 0 {
                if let Err(e) = nova_scene::save_to_file(&self.world, path) {
                    log::warn!("autosave failed: {e}");
                }
            }
        }
    }

    fn render_frame(&mut self) {
        // Fixed-timestep accumulation.
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f32();
        self.last_time = now;

        let (accumulator, steps) = accumulate_fixed_steps(self.accumulator, elapsed, FIXED_DT);
        self.accumulator = accumulator;

        // While editing with the sim paused, keep transforms fresh but skip
        // gameplay. Otherwise run the full deterministic schedule.
        let simulate = !(self.editor_enabled && !self.editor.playing);
        for _ in 0..steps {
            if simulate {
                self.step();
            } else {
                self.step_paused();
            }
        }

        // Viewport zoom: the mouse wheel dollies the editor camera toward/away
        // from the origin. `scroll` is accumulated per-frame by `InputState` and
        // cleared by `end_frame` below, so read it before then.
        if let Some(input) = self.world.resource::<InputState>() {
            let scroll = input.scroll;
            if scroll != 0.0 {
                self.camera_distance = (self.camera_distance - scroll * 0.5).clamp(1.0, 50.0);
            }
        }
        if let Some(cam_t) = self.world.get_component_mut::<Transform>(self.camera) {
            cam_t.translation.z = self.camera_distance;
        }
        // Re-propagate so the gizmo's camera matrices reflect the new distance.
        nova_ecs::scene_graph::propagate_transforms(&mut self.world);

        // Continuous gizmo drag: apply the live delta every frame the button is
        // held over the selection.
        if let Some((start, entity, _before)) = self.gizmo_drag {
            if let Some((_vp, inv_vp, forward)) = self.camera_matrices() {
                let size = self.viewport_size;
                nova_editor::apply_gizmo_3d(
                    &mut self.world,
                    entity,
                    self.gizmo_mode,
                    inv_vp,
                    (size.0 as f32, size.1 as f32),
                    forward,
                    start,
                    self.pointer,
                    0.0,
                );
            }
        }

        // Build the editor draw list and composite it over the 3D scene.
        let draw = if self.editor_enabled {
            self.build_editor_ui()
        } else {
            DrawList::new()
        };

        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) = renderer.render_with_ui(&self.world, &draw) {
                log::error!("render error: {e}");
            }
        }

        // Per-frame input deltas + the one-frame press edge.
        if let Some(input) = self.world.resource_mut::<InputState>() {
            input.end_frame();
        }
        self.pointer_pressed = false;
    }

    /// Build the full editor `DrawList`: toolbar + panels + gizmo handles +
    /// highlight marquee. Also wires toolbar button clicks to editor actions.
    fn build_editor_ui(&mut self) -> DrawList {
        let layout = editor_layout(self.viewport_size);
        let input = UiInput {
            pointer: self.pointer,
            pointer_down: self.pointer_down,
            pointer_pressed: self.pointer_pressed,
            shift_held: self.modifiers.shift_key(),
            text_entered: self
                .world
                .resource::<InputState>()
                .map(|i| i.text_entered.clone())
                .unwrap_or_default(),
        };

        // Feed typed keystrokes into the focused text field (pointer-only UI
        // bridge). The Highlight & Fix instruction field is the focus target.
        if self.editor.text_focus.as_deref() == Some("fix_instruction") {
            self.fix_instruction.push_str(&input.text_entered);
        }

        let mut draw: DrawList = Vec::new();

        // ---- Toolbar (interactive) ---------------------------------------
        draw.extend(self.build_toolbar(layout.toolbar, input.clone()));

        // ---- Hierarchy / inspector / asset panels (self-handling) ---------
        draw.extend(hierarchy_panel(
            &self.world,
            &mut self.editor,
            input.clone(),
            layout.hierarchy,
            self.modifiers.shift_key(),
        ));
        draw.extend(inspector_panel(
            &mut self.world,
            &mut self.editor,
            input.clone(),
            layout.inspector,
        ));
        draw.extend(asset_browser_panel(&mut self.editor, input.clone(), layout.assets));

        // ---- Agent explainability log + telemetry scrubber ---------------
        draw.extend(self.draw_agent_panel(layout.agent_panel, input.clone()));

        // ---- Vibe GUI (Bézier curve editor) ------------------------------
        // Drive interaction each frame (immediate mode), then render the curve.
        self.editor.vibe.interact(
            layout.vibe,
            self.pointer,
            self.pointer_pressed,
            self.pointer_down,
        );
        draw.extend(self.draw_vibe_panel(layout.vibe));

        // ---- Gizmo handles for the current selection ----------------------
        if self.tool == ViewportTool::Gizmo {
            if let Some(sel) = self.editor.selected {
                if let Some((vp, _, _)) = self.camera_matrices() {
                    if let Some((sx, sy)) =
                        project_to_screen(self.selection_center(sel), vp, self.viewport_size)
                    {
                        draw.push(DrawCommand::Rect {
                            rect: Rect::from_min_size(
                                Vec2::new(sx as f32 - 6.0, sy as f32 - 6.0),
                                Vec2::new(12.0, 12.0),
                            ),
                            color: Color::rgb(0.2, 0.9, 0.3),
                            rounding: 2.0,
                        });
                    }
                }
            }
        }

        // ---- Highlight & Fix marquee -------------------------------------
        if self.tool == ViewportTool::Highlight {
            // The instruction text field (typed by the user, replaces the old
            // hardcoded literal).
            let mut ui = Ui::new(input);
            ui.begin_panel(layout.instruction, Some("Highlight & Fix"));
            let resp = ui.text_input("Instruction", &self.fix_instruction, self.fix_text_focused);
            if resp.clicked {
                self.fix_text_focused = !self.fix_text_focused;
            }
            ui.end_panel();
            draw.extend(ui.finish());

            if let Some(rect) = self.overlay.current_rect(self.viewport_size) {
                draw.push(DrawCommand::Rect {
                    rect: Rect::from_min_size(
                        Vec2::new(rect.x0 as f32, rect.y0 as f32),
                        Vec2::new(rect.width() as f32, rect.height() as f32),
                    ),
                    color: Color::rgba(0.2, 1.0, 0.4, 0.35),
                    rounding: 0.0,
                });
            }
        }

        draw
    }

    /// Render the Vibe GUI Bézier curve editor into a draw list for `area`.
    fn draw_vibe_panel(&self, area: Rect) -> DrawList {
        let mut draw: DrawList = Vec::new();
        draw.push(DrawCommand::Rect {
            rect: area,
            color: Color::rgba(0.12, 0.12, 0.14, 0.95),
            rounding: 3.0,
        });
        draw.push(DrawCommand::Text {
            pos: Vec2::new(area.min.x + 8.0, area.min.y + 4.0),
            text: "Vibe (gravity)".to_string(),
            color: Color::rgb(0.92, 0.92, 0.95),
            size: 14.0,
        });

        let plot = Rect::from_min_size(
            Vec2::new(area.min.x + 8.0, area.min.y + 24.0),
            Vec2::new(area.width() - 16.0, area.height() - 32.0),
        );
        draw.push(DrawCommand::Rect {
            rect: plot,
            color: Color::rgb(0.08, 0.08, 0.10),
            rounding: 2.0,
        });

        // The curve, drawn as a polyline of small squares.
        for p in self.editor.vibe.curve.polyline(40) {
            let s = normalized_to_screen(plot, p);
            draw.push(DrawCommand::Rect {
                rect: Rect::from_min_size(s, Vec2::new(2.0, 2.0)),
                color: Color::rgb(0.4, 0.9, 1.0),
                rounding: 0.0,
            });
        }
        // Draggable control-point handles.
        for r in self.editor.vibe.handle_rects(plot) {
            draw.push(DrawCommand::Rect {
                rect: r,
                color: Color::rgb(0.2, 0.9, 0.3),
                rounding: 2.0,
            });
        }

        let g = self.editor.vibe_binding.gravity(&self.editor.vibe.curve);
        draw.push(DrawCommand::Text {
            pos: Vec2::new(plot.min.x + 4.0, plot.max.y - 16.0),
            text: format!("gravity y = {:.2}", g.y),
            color: Color::rgb(0.9, 0.9, 0.6),
            size: 12.0,
        });
        draw
    }

    /// The world-space center of `entity` for gizmo handle placement.
    fn selection_center(&self, entity: Entity) -> Vec3 {
        self.world
            .get_component::<GlobalTransform>(entity)
            .map(|gt| gt.translation())
            .or_else(|| {
                self.world
                    .get_component::<Transform>(entity)
                    .map(|t| t.translation)
            })
            .unwrap_or(Vec3::ZERO)
    }

    /// Build the top toolbar and perform actions when its buttons are clicked.
    fn build_toolbar(&mut self, area: Rect, input: UiInput) -> DrawList {
        let mut ui = Ui::new(input);
        ui.begin_panel(area, Some("TPT Nova — Editor"));

        let mut enabled = self.editor_enabled;
        let _ = ui.checkbox("Editor", &mut enabled);
        if enabled != self.editor_enabled {
            self.editor_enabled = enabled;
        }

        let mode_label = match self.gizmo_mode {
            nova_editor::GizmoMode3D::Move => "Gizmo: Move",
            nova_editor::GizmoMode3D::Rotate => "Gizmo: Rotate",
            nova_editor::GizmoMode3D::Scale => "Gizmo: Scale",
        };
        let mode_resp = ui.button(mode_label);
        if mode_resp.clicked {
            self.gizmo_mode = match self.gizmo_mode {
                nova_editor::GizmoMode3D::Move => nova_editor::GizmoMode3D::Rotate,
                nova_editor::GizmoMode3D::Rotate => nova_editor::GizmoMode3D::Scale,
                nova_editor::GizmoMode3D::Scale => nova_editor::GizmoMode3D::Move,
            };
        }

        let tool_label = match self.tool {
            ViewportTool::Gizmo => "Tool: Gizmo",
            ViewportTool::Highlight => "Tool: Highlight",
        };
        let tool_resp = ui.button(tool_label);
        if tool_resp.clicked {
            self.tool = match self.tool {
                ViewportTool::Gizmo => ViewportTool::Highlight,
                ViewportTool::Highlight => ViewportTool::Gizmo,
            };
        }

        let mut playing = self.editor.playing;
        let play_label = if playing { "Pause" } else { "Play" };
        let play_resp = ui.button(play_label);
        if play_resp.clicked {
            playing = !playing;
            self.editor.playing = playing;
        }

        let undo_resp = ui.button("Undo");
        if undo_resp.clicked {
            self.editor.history.undo(&mut self.world);
        }
        let redo_resp = ui.button("Redo");
        if redo_resp.clicked {
            self.editor.history.redo(&mut self.world);
        }

        if let Some(fix) = &self.last_fix {
            ui.label(&format!("fix: {} ent", fix.entity_ids.len()));
        } else {
            ui.label("keys: E edit · G gizmo · H highlight · P play · Ctrl+Z undo");
        }

        ui.end_panel();
        ui.finish()
    }

    /// Begin a pointer interaction (mouse-down) given the current window state.
    fn on_pointer_press(&mut self) {
        if !self.editor_enabled {
            return;
        }
        let layout = editor_layout(self.viewport_size);
        if layout.over_panel(self.pointer) {
            return; // let the panel UI consume the click
        }

        // Shift-click in the viewport toggles the entity into the multi-selection
        // instead of replacing it (mirrors the hierarchy panel behavior).
        let shift = self.modifiers.shift_key();

        match self.tool {
            ViewportTool::Gizmo => {
                // Spawning: if an asset is selected in the browser, a viewport
                // click drops a new entity of that kind and selects it, so the
                // browser's `selected_asset` actually does something.
                if let Some(asset) = self.editor.selected_asset.clone() {
                    let e = self.world.spawn();
                    self.world
                        .add_component(e, Transform::from_translation(Vec3::ZERO));
                    self.world.add_component(
                        e,
                        Mesh {
                            kind: MeshKind::Cube,
                        },
                    );
                    self.world.add_component(e, GlobalTransform::identity());
                    self.editor.select(e);
                    log::info!("spawned asset '{asset}' as new entity");
                    return;
                }

                if let Some((vp, _, _)) = self.camera_matrices() {
                    let pick = self.pick_entity(vp, 40.0);
                    if shift {
                        // Shift-click is for multi-select, not a gizmo drag.
                        if let Some(e) = pick {
                            self.editor.toggle_select(e);
                        }
                        return;
                    }
                    if self.editor.selected.is_none() {
                        if let Some(e) = pick {
                            self.editor.select(e);
                        }
                    }
                    if let Some(sel) = self.editor.selected {
                        // Capture the pre-drag transform so the drag can be
                        // recorded into the undo history on release.
                        let before = self
                            .world
                            .get_component::<Transform>(sel)
                            .copied()
                            .unwrap_or_default();
                        self.gizmo_drag = Some((self.pointer, sel, before));
                    }
                }
            }
            ViewportTool::Highlight => {
                let (x, y) = (self.pointer.x as u32, self.pointer.y as u32);
                self.overlay.begin(x, y);
            }
        }
    }

    /// End a pointer interaction (mouse-up).
    fn on_pointer_release(&mut self) {
        // Record the completed gizmo drag into the undo history so 3D moves /
        // rotations / scales are undoable (previously the gizmo bypassed
        // `EditHistory::record`).
        if let Some((_start, entity, before)) = self.gizmo_drag.take() {
            if let Some(after) = self.world.get_component::<Transform>(entity).copied() {
                self.editor.history.record_transform(entity, before, after);
            }
        }
        if self.tool == ViewportTool::Highlight {
            let (x, y) = (self.pointer.x as u32, self.pointer.y as u32);
            self.overlay.drag(x, y);
            if let Some((vp, _, _)) = self.camera_matrices() {
                if let Ok(req) = self.overlay.build_request(
                    &self.world,
                    vp,
                    self.viewport_size,
                    &self.fix_instruction,
                ) {
                    log::info!("Highlight & Fix request:\n{}", req.prompt);
                    self.last_fix = Some(req);
                }
            }
        }
    }

    fn handle_key(&mut self, key: &Key, pressed: bool) {
        if !pressed {
            return;
        }
        // While the Highlight & Fix instruction field has focus, keystrokes edit
        // that string instead of driving editor shortcuts. Escape unfocuses.
        if self.fix_text_focused {
            match key {
                Key::Character(c) => self.fix_instruction.push_str(c.as_str()),
                Key::Named(NamedKey::Backspace) => {
                    self.fix_instruction.pop();
                }
                Key::Named(NamedKey::Space) => self.fix_instruction.push(' '),
                Key::Named(NamedKey::Escape) => self.fix_text_focused = false,
                _ => {}
            }
            return;
        }
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let ch = match key {
            Key::Character(c) => Some(c.to_string()),
            _ => None,
        };
        if ctrl {
            match ch.as_deref() {
                Some("z") if shift => {
                    self.editor.history.redo(&mut self.world);
                }
                Some("z") => {
                    self.editor.history.undo(&mut self.world);
                }
                Some("y") => {
                    self.editor.history.redo(&mut self.world);
                }
                _ => {}
            }
            return;
        }
        match ch.as_deref() {
            Some("e") => self.editor_enabled = !self.editor_enabled,
            Some("g") => {
                self.gizmo_mode = match self.gizmo_mode {
                    nova_editor::GizmoMode3D::Move => nova_editor::GizmoMode3D::Rotate,
                    nova_editor::GizmoMode3D::Rotate => nova_editor::GizmoMode3D::Scale,
                    nova_editor::GizmoMode3D::Scale => nova_editor::GizmoMode3D::Move,
                };
            }
            Some("h") => {
                self.tool = match self.tool {
                    ViewportTool::Gizmo => ViewportTool::Highlight,
                    ViewportTool::Highlight => ViewportTool::Gizmo,
                };
            }
            Some("p") => self.editor.toggle_play(),
            Some("Escape") => self.editor.clear_selection(),
            _ => {}
        }
    }
}

/// Given the current accumulator and the elapsed real time, return the number
/// of fixed simulation steps to run this frame plus the leftover accumulator.
///
/// A huge stall (e.g. a breakpoint hit or tab switch) is clamped to `0.25`s so a
/// single frame can never trigger an unbounded catch-up loop. Pulled out of
/// [`App::render_frame`] so the tick bookkeeping is unit-testable without a
/// window or GPU.
pub(crate) fn accumulate_fixed_steps(accumulator: f32, elapsed: f32, dt: f32) -> (f32, u32) {
    let mut acc = accumulator + elapsed.min(0.25);
    let mut steps = 0u32;
    while acc >= dt {
        acc -= dt;
        steps += 1;
    }
    (acc, steps)
}

fn movement_system(world: &mut World, cube: Entity) {
    let input = match world.resource::<InputState>() {
        Some(i) => i,
        None => return,
    };
    let actions = match world.resource::<ActionMap>() {
        Some(a) => a,
        None => return,
    };

    let speed = 1.5_f32 * FIXED_DT; // radians per fixed step
    let mut dy = 0.0_f32;
    let mut dx = 0.0_f32;
    if actions.is_active(input, "move_left") {
        dy += speed;
    }
    if actions.is_active(input, "move_right") {
        dy -= speed;
    }
    if actions.is_active(input, "spin_up") {
        dx += speed;
    }
    if actions.is_active(input, "spin_down") {
        dx -= speed;
    }

    if dx != 0.0 || dy != 0.0 {
        if let Some(t) = world.get_component_mut::<Transform>(cube) {
            let add = Quat::from_euler(EulerRot::XYZ, dx, dy, 0.0);
            t.rotation = add * t.rotation;
        }
    }
}

fn apply_control(world: &mut World, cube: Entity, last_mtime: &mut Option<u64>, path: &str) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d: Duration| d.as_millis() as u64);
    if mtime == *last_mtime {
        return;
    }
    *last_mtime = mtime;

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let ctrl: ControlFile = match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("bad control file: {e}");
            return;
        }
    };
    if let Some(rot) = ctrl.set_rotation {
        if let Some(t) = world.get_component_mut::<Transform>(cube) {
            t.rotation = Quat::from_euler(EulerRot::XYZ, rot.x, rot.y, rot.z);
            log::info!("hot-applied rotation x={} y={} z={}", rot.x, rot.y, rot.z);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("TPT Nova — Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let renderer = Renderer::new(Arc::clone(&window)).expect("init renderer");
        self.viewport_size = (
            window.inner_size().width.max(1),
            window.inner_size().height.max(1),
        );
        self.window = Some(Arc::clone(&window));
        self.renderer = Some(renderer);
        self.last_time = Instant::now();
        log::info!("TPT Nova resumed; window + renderer ready");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        match event {
            WinitWindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WinitWindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                self.viewport_size = (size.width.max(1), size.height.max(1));
            }
            WinitWindowEvent::RedrawRequested => {
                self.render_frame();
            }
            WinitWindowEvent::KeyboardInput {
                event: ref key_event,
                ..
            } => {
                if let Some(input) = self.world.resource_mut::<InputState>() {
                    input.apply_event(&event);
                }
                if key_event.state == ElementState::Pressed {
                    self.handle_key(&key_event.logical_key, true);
                }
            }
            WinitWindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WinitWindowEvent::CursorMoved { position, .. } => {
                self.pointer = Vec2::new(position.x as f32, position.y as f32);
                if let Some(input) = self.world.resource_mut::<InputState>() {
                    input.apply_event(&event);
                }
            }
            WinitWindowEvent::MouseInput { state, .. } => {
                let down = state == ElementState::Pressed;
                if down {
                    self.pointer_pressed = true;
                    self.pointer_down = true;
                    self.on_pointer_press();
                } else {
                    self.pointer_down = false;
                    self.on_pointer_release();
                }
                if let Some(input) = self.world.resource_mut::<InputState>() {
                    input.apply_event(&event);
                }
            }
            WinitWindowEvent::MouseWheel { .. } => {
                if let Some(input) = self.world.resource_mut::<InputState>() {
                    input.apply_event(&event);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let seed: u64 = std::env::var("NOVA_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x1234_ABCD);
    log::info!("TPT Nova starting (seed=0x{seed:016X})");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(seed);
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::scheduler::Scheduler;
    use nova_ecs::transform::{GlobalTransform, Mesh, Transform};
    use nova_ecs::{Quat, World};
    use nova_input::{default_action_map, ActionMap, InputState};
    use nova_ui::DrawList;

    // ---- App shell init (headless, no window/GPU) -------------------------

    #[test]
    fn app_new_builds_expected_world() {
        // `App::new` only builds the world/scheduler; it never opens a window or
        // initializes the renderer, so it is safe under CI with no display.
        let app = App::new(0xCAFE);

        assert_eq!(app.world.entity_count(), 2);
        assert!(app.world.has_component::<Transform>(app.cube));
        assert!(app.world.has_component::<Mesh>(app.cube));
        assert!(app.world.has_component::<Camera>(app.camera));
        assert!(app.world.has_component::<GlobalTransform>(app.cube));

        // Resources the systems rely on must be present.
        assert!(app.world.has_resource::<InputState>());
        assert!(app.world.has_resource::<ActionMap>());
        assert!(app.world.has_resource::<nova_ecs::rng::RngResource>());
        assert!(app.world.has_resource::<TickResource>());

        // The seed flows into the RNG resource.
        let seed = app
            .world
            .resource::<nova_ecs::rng::RngResource>()
            .unwrap()
            .seed;
        assert_eq!(seed, 0xCAFE);
    }

    #[test]
    fn fixed_timestep_accumulates_steps() {
        let dt = FIXED_DT;
        // Exactly one step fits; accumulator is fully consumed.
        let (acc, steps) = accumulate_fixed_steps(0.0, dt, dt);
        assert_eq!(steps, 1);
        assert!((acc).abs() < 1e-6);

        // Two steps fit into one frame of 2*dt.
        let (acc, steps) = accumulate_fixed_steps(0.0, dt * 2.0, dt);
        assert_eq!(steps, 2);
        assert!((acc).abs() < 1e-6);

        // A large stall is clamped to 0.25s, yielding a bounded step count.
        // At 60 Hz, 0.25 s is exactly 15 fixed steps.
        let (acc, steps) = accumulate_fixed_steps(0.0, 100.0, dt);
        assert_eq!(steps, 15);
        assert!(acc.abs() < dt);

        // A carry-over accumulator plus a small frame still advances correctly.
        let (acc, steps) = accumulate_fixed_steps(dt * 0.5, dt * 0.75, dt);
        assert_eq!(steps, 1);
        assert!((acc - dt * 0.25).abs() < 1e-6);
    }

    // ---- Gameplay logic ---------------------------------------------------

    #[test]
    fn movement_system_rotates_cube_from_active_action() {
        let mut world = World::new();
        let cube = world.spawn();
        world.add_component(cube, Transform::default());

        world.add_resource(InputState::default());
        world.add_resource(default_action_map());

        // Hold "move_left": expected to nudge the cube's rotation.
        world
            .resource_mut::<InputState>()
            .unwrap()
            .keys
            .insert(nova_input::KeyCode::KeyA);

        let before = world.get_component::<Transform>(cube).unwrap().rotation;
        movement_system(&mut world, cube);
        let after = world.get_component::<Transform>(cube).unwrap().rotation;

        assert_ne!(before, after);
        // A pure local spin rotated the cube away from its identity orientation.
        assert_eq!(before, Quat::IDENTITY);
        assert_ne!(after, Quat::IDENTITY);
        // The resulting orientation is still a valid unit quaternion.
        assert!((after.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn movement_system_is_noop_without_active_action() {
        let mut world = World::new();
        let cube = world.spawn();
        world.add_component(cube, Transform::from_rotation(Quat::from_rotation_y(0.5)));

        world.add_resource(InputState::default());
        world.add_resource(default_action_map());

        let before = world.get_component::<Transform>(cube).unwrap().rotation;
        movement_system(&mut world, cube);
        let after = world.get_component::<Transform>(cube).unwrap().rotation;
        assert_eq!(before, after);
    }

    #[test]
    fn scheduler_step_advances_tick_resource() {
        let mut world = World::new();
        world.add_resource(TickResource { tick: 0 });

        let mut scheduler = Scheduler::new();
        scheduler.add_system(|w: &mut World| {
            w.resource_mut::<TickResource>().unwrap().tick += 1;
        });

        for _ in 0..5 {
            scheduler.run(&mut world);
        }
        assert_eq!(world.resource::<TickResource>().unwrap().tick, 5);
    }

    // ---- AI code-injection loop (regression harness) ----------------------

    #[test]
    fn ai_control_loop_hot_applies_rotation_and_is_idempotent() {
        // End-to-end: an external agent writes `nova-control.json` -> the engine
        // reads telemetry -> mutates the control file -> the engine hot-applies
        // it next tick -> the world reflects the change (no restart).
        let dir = std::env::temp_dir();
        let control = dir.join("nova_control_loop_test.json");
        let telemetry = dir.join("nova_telemetry_loop_test.json");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&telemetry);

        let mut app =
            App::new_with_paths(1, telemetry.clone(), control.to_string_lossy().to_string());

        // No control file yet -> cube stays at identity rotation.
        app.step();
        let before = app
            .world
            .get_component::<Transform>(app.cube)
            .unwrap()
            .rotation;
        assert_eq!(before, Quat::IDENTITY);

        // External agent writes a control file asking for a specific rotation.
        let want = Quat::from_euler(EulerRot::XYZ, 0.3, 0.6, 0.9);
        std::fs::write(
            &control,
            serde_json::json!({ "set_rotation": { "x": 0.3, "y": 0.6, "z": 0.9 } }).to_string(),
        )
        .unwrap();
        // Give the filesystem a moment so the mtime definitely advances.
        std::thread::sleep(Duration::from_millis(20));

        app.step();
        let applied = app
            .world
            .get_component::<Transform>(app.cube)
            .unwrap()
            .rotation;
        assert!(
            (applied - want).length() < 1e-4,
            "rotation should be hot-applied: {applied:?} vs {want:?}"
        );

        // Stepping again without rewriting the control file must NOT re-apply
        // (the loop is idempotent between writes), guarding against resets.
        app.step();
        let still = app
            .world
            .get_component::<Transform>(app.cube)
            .unwrap()
            .rotation;
        assert!(
            (still - want).length() < 1e-4,
            "should remain stable when control is unchanged"
        );

        // A new control file with a different rotation replaces the old one.
        let want2 = Quat::from_euler(EulerRot::XYZ, -0.5, 0.0, 0.0);
        std::fs::write(
            &control,
            serde_json::json!({ "set_rotation": { "x": -0.5, "y": 0.0, "z": 0.0 } }).to_string(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        app.step();
        let applied2 = app
            .world
            .get_component::<Transform>(app.cube)
            .unwrap()
            .rotation;
        assert!(
            (applied2 - want2).length() < 1e-4,
            "rotation should re-hot-apply on a new write"
        );

        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&telemetry);
    }

    #[test]
    fn telemetry_loop_emits_world_state_across_ticks() {
        // The engine must emit a parseable telemetry frame (what the AI agent
        // reads) after enough deterministic ticks.
        let dir = std::env::temp_dir();
        let telemetry = dir.join("nova_telemetry_emit_test.json");
        let control = dir.join("nova_control_emit_test.json");
        let _ = std::fs::remove_file(&telemetry);

        let mut app =
            App::new_with_paths(7, telemetry.clone(), control.to_string_lossy().to_string());
        // Run enough ticks to cross the telemetry emission interval (30 ticks).
        for _ in 0..(TELEMETRY_INTERVAL + 1) {
            app.step();
        }
        let text = std::fs::read_to_string(&telemetry).expect("telemetry file written");
        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(frame["schema_version"], 1);
        assert_eq!(frame["seed"], 7);
        // The cube (and camera) must appear in the emitted entity dump.
        let entities = frame["entities"].as_array().unwrap();
        assert!(entities.len() >= 2, "expected cube + camera entities");
        assert!(entities
            .iter()
            .any(|e| e["components"].get("Mesh").is_some()));
        assert!(entities
            .iter()
            .any(|e| e["components"].get("Camera").is_some()));

        let _ = std::fs::remove_file(&telemetry);
    }

    // ---- Editor wiring (headless: builds a DrawList without a GPU) --------

    #[test]
    fn editor_ui_builds_nonempty_draw_list() {
        let mut app = App::new(0xBEEF);
        app.viewport_size = (1280, 720);
        let draw: DrawList = app.build_editor_ui();
        // Toolbar + hierarchy + inspector + asset panels each emit primitives.
        assert!(!draw.is_empty(), "editor must produce drawable primitives");
    }

    #[test]
    fn editor_layout_excludes_panels_from_viewport() {
        let l = editor_layout((1280, 720));
        // The corner of the hierarchy panel is over a panel, not the viewport.
        assert!(l.over_panel(Vec2::new(20.0, 60.0)));
        // The center of the viewport region is not over any panel.
        let center = (l.viewport.min + l.viewport.max) * 0.5;
        assert!(!l.over_panel(center), "viewport center must be interactive");
    }

    #[test]
    fn toggle_keys_change_editor_state() {
        let mut app = App::new(0x5A);
        let key = |s: &str| Key::Character(s.into());
        let pressed = true;

        let before = app.editor_enabled;
        app.handle_key(&key("e"), pressed);
        assert_ne!(app.editor_enabled, before);

        let m0 = app.gizmo_mode;
        app.handle_key(&key("g"), pressed);
        assert_ne!(app.gizmo_mode, m0);

        let t0 = app.tool;
        app.handle_key(&key("h"), pressed);
        assert_ne!(app.tool, t0);

        let p0 = app.editor.playing;
        app.handle_key(&key("p"), pressed);
        assert_ne!(app.editor.playing, p0);
    }

    #[test]
    fn gizmo_drag_is_undoable() {
        // A 3D gizmo drag must land in the undo history and be reversible.
        let mut app = App::new(0xCAFE);
        app.viewport_size = (1280, 720);

        let before = app
            .world
            .get_component::<Transform>(app.cube)
            .copied()
            .unwrap();
        app.editor.select(app.cube);
        // Begin a drag from the viewport center, then move 100px right.
        app.gizmo_drag = Some((Vec2::new(640.0, 360.0), app.cube, before));
        app.pointer = Vec2::new(740.0, 360.0);
        app.render_frame();

        let after = app
            .world
            .get_component::<Transform>(app.cube)
            .copied()
            .unwrap();
        assert!(
            (after.translation.x - before.translation.x).abs() > 1e-3,
            "drag should move the cube along x"
        );

        // Release records the drag into history.
        app.on_pointer_release();
        assert!(app.editor.history.can_undo());

        // Undo restores the exact pre-drag pose.
        app.editor.history.undo(&mut app.world);
        let restored = app
            .world
            .get_component::<Transform>(app.cube)
            .copied()
            .unwrap();
        assert!((restored.translation.x - before.translation.x).abs() < 1e-4);
    }

    #[test]
    fn scroll_wheel_dollies_camera() {
        // The mouse wheel must drive viewport zoom via `camera_distance`.
        let mut app = App::new(1);
        app.viewport_size = (1280, 720);
        let d0 = app.camera_distance;
        if let Some(input) = app.world.resource_mut::<InputState>() {
            input.scroll = 2.0;
        }
        app.render_frame();
        assert!(
            app.camera_distance < d0,
            "positive scroll should dolly the camera closer"
        );
    }

    #[test]
    fn asset_browser_selection_spawns_on_viewport_click() {
        // Selecting an asset in the browser and clicking the viewport must
        // actually spawn an entity (makes `selected_asset` do something).
        let mut app = App::new(2);
        app.viewport_size = (1280, 720);
        app.editor.selected_asset = Some("cube.glb".to_string());
        let before = app.world.entity_count();

        app.pointer = Vec2::new(640.0, 400.0);
        app.pointer_down = true;
        app.pointer_pressed = true;
        app.on_pointer_press();

        assert_eq!(
            app.world.entity_count(),
            before + 1,
            "clicking with a selected asset should spawn an entity"
        );
        assert!(app.editor.selected.is_some());
    }
}
