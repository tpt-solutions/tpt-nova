//! TPT Nova application shell.
//!
//! Boots a winit window, builds the ECS world (a cube + camera), and runs a
//! deterministic fixed-timestep loop. Input is mapped to actions; an external
//! AI agent can hot-apply changes by writing `nova-control.json`, which the
//! engine polls each tick. Telemetry is dumped to `nova-telemetry.json` on an
//! interval so the agent can observe and self-correct.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::EulerRot;
use nova_ecs::scheduler::Scheduler;
use nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind, Transform};
use nova_ecs::{Entity, Quat, Vec3, World};
use nova_input::{default_action_map, ActionMap, InputState};
use nova_render::Renderer;
use nova_telemetry::{FileSink, TelemetryEmitter};
use serde::Deserialize;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes};

const FIXED_DT: f32 = 1.0 / 60.0;
const TELEMETRY_INTERVAL: u64 = 30; // emit every 30 ticks (~0.5s)
const CONTROL_PATH: &str = "nova-control.json";

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

        App {
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
        }
    }

    fn new(seed: u64) -> Self {
        Self::new_with_paths(
            seed,
            PathBuf::from("nova-telemetry.json"),
            CONTROL_PATH.to_string(),
        )
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

        let _ = self.emitter.maybe_emit(&self.world, tick, seed);
    }

    fn render_frame(&mut self) {
        // Fixed-timestep accumulation.
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f32();
        self.last_time = now;

        let (accumulator, steps) = accumulate_fixed_steps(self.accumulator, elapsed, FIXED_DT);
        self.accumulator = accumulator;
        for _ in 0..steps {
            self.step();
        }

        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) = renderer.render(&self.world) {
                log::error!("render error: {e}");
            }
        }

        // Clear per-frame input deltas.
        if let Some(input) = self.world.resource_mut::<InputState>() {
            input.end_frame();
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
            .with_title("TPT Nova — Phase 1")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let renderer = Renderer::new(Arc::clone(&window)).expect("init renderer");
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
            }
            WinitWindowEvent::RedrawRequested => {
                self.render_frame();
            }
            WinitWindowEvent::KeyboardInput { .. }
            | WinitWindowEvent::CursorMoved { .. }
            | WinitWindowEvent::MouseInput { .. }
            | WinitWindowEvent::MouseWheel { .. } => {
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
}
