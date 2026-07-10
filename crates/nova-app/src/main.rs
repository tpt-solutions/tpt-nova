//! TPT Nova application shell.
//!
//! Boots a winit window, builds the ECS world (a cube + camera), and runs a
//! deterministic fixed-timestep loop. Input is mapped to actions; an external
//! AI agent can hot-apply changes by writing `nova-control.json`, which the
//! engine polls each tick. Telemetry is dumped to `nova-telemetry.json` on an
//! interval so the agent can observe and self-correct.

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
}

impl App {
    fn new(seed: u64) -> Self {
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
                FileSink::new(std::path::PathBuf::from("nova-telemetry.json")),
                TELEMETRY_INTERVAL,
            ),
            cube,
            camera,
            last_time: Instant::now(),
            accumulator: 0.0,
            control_mtime: None,
        }
    }

    fn step(&mut self) {
        // External control override (hot-apply without restart).
        apply_control(&mut self.world, self.cube, &mut self.control_mtime);

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
        self.accumulator += elapsed.min(0.25); // clamp huge stalls

        while self.accumulator >= FIXED_DT {
            self.step();
            self.accumulator -= FIXED_DT;
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

fn apply_control(world: &mut World, cube: Entity, last_mtime: &mut Option<u64>) {
    let meta = match std::fs::metadata(CONTROL_PATH) {
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

    let text = match std::fs::read_to_string(CONTROL_PATH) {
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
