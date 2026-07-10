//! Example gameplay module: a top-down, player-controlled 2D entity.
//!
//! This crate compiles to a `cdylib` the engine hot-loads through
//! [`nova_scripting`], and to an `rlib` so the same logic can be unit-tested
//! in-process. All the behavior lives in [`PlayerController::update`] — the
//! engine itself contains none of it, proving the hot-reload boundary: edit
//! this file, rebuild the dylib, and the running engine picks it up.

use glam::Vec2;
use nova_ecs::component::Component;
use nova_ecs::transform::Transform;
use nova_ecs::{Entity, World};
use nova_input::{ActionMap, InputState};
use nova_physics::RigidBody2D;
use nova_scripting::{export_gameplay, GameplayModule, ScriptContext};

/// Marker + tuning for a player-controlled entity.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    /// Movement speed in world units per second.
    pub speed: f32,
}

impl Default for Player {
    fn default() -> Self {
        Player { speed: 5.0 }
    }
}

impl Component for Player {}

/// The hot-reloadable gameplay module.
#[derive(Default)]
pub struct PlayerController;

/// Read the current desired movement direction from input actions.
///
/// Uses the `move_left/right/forward/back` semantic actions so bindings stay in
/// data. Returns a (possibly zero) direction vector, normalized when diagonal.
pub fn desired_direction(world: &World) -> Vec2 {
    let input = match world.resource::<InputState>() {
        Some(i) => i,
        None => return Vec2::ZERO,
    };
    let actions = match world.resource::<ActionMap>() {
        Some(a) => a,
        None => return Vec2::ZERO,
    };
    let mut dir = Vec2::ZERO;
    if actions.is_active(input, "move_right") {
        dir.x += 1.0;
    }
    if actions.is_active(input, "move_left") {
        dir.x -= 1.0;
    }
    if actions.is_active(input, "move_forward") {
        dir.y += 1.0;
    }
    if actions.is_active(input, "move_back") {
        dir.y -= 1.0;
    }
    if dir != Vec2::ZERO {
        dir = dir.normalize();
    }
    dir
}

impl GameplayModule for PlayerController {
    fn name(&self) -> &str {
        "player-controller"
    }

    fn update(&mut self, ctx: &mut ScriptContext) {
        let dir = desired_direction(ctx.world);

        // Snapshot players first to avoid holding an immutable borrow while we
        // mutate their components.
        let players: Vec<(Entity, f32)> = ctx
            .world
            .query_1::<Player>()
            .into_iter()
            .map(|(e, p)| (e, p.speed))
            .collect();

        for (e, speed) in players {
            let velocity = dir * speed;
            // Move the transform directly (works with or without physics).
            if let Some(t) = ctx.world.get_component_mut::<Transform>(e) {
                t.translation.x += velocity.x * ctx.dt;
                t.translation.y += velocity.y * ctx.dt;
            }
            // If the entity is physics-driven, also drive its kinematic body so
            // collisions are respected when the physics step runs.
            if let Some(rb) = ctx.world.get_component_mut::<RigidBody2D>(e) {
                rb.linvel = velocity;
            }
        }
    }
}

// Generate the C ABI exports the engine loads.
export_gameplay!(PlayerController);

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;
    use nova_ecs::Vec3;
    use nova_input::{default_action_map, InputState, KeyCode};

    fn world_with_player_pressing_right() -> (World, Entity) {
        let mut world = World::new();
        let mut input = InputState::default();
        input.keys.insert(KeyCode::KeyD); // bound to "move_right"
        world.add_resource(input);
        world.add_resource(default_action_map());

        let player = world.spawn();
        world.add_component(player, Transform::from_translation(Vec3::ZERO));
        world.add_component(player, Player::default());
        (world, player)
    }

    #[test]
    fn player_moves_right_with_input() {
        let (mut world, player) = world_with_player_pressing_right();
        let mut controller = PlayerController;
        let mut ctx = ScriptContext {
            world: &mut world,
            dt: 1.0,
            tick: 0,
        };
        controller.update(&mut ctx);
        let t = world.get_component::<Transform>(player).unwrap();
        assert!(
            t.translation.x > 4.9,
            "expected ~5.0, got {}",
            t.translation.x
        );
        assert!(t.translation.y.abs() < 1e-6);
    }

    #[test]
    fn no_input_no_movement() {
        let mut world = World::new();
        world.add_resource(InputState::default());
        world.add_resource(default_action_map());
        let player = world.spawn();
        world.add_component(player, Transform::from_translation(Vec3::ZERO));
        world.add_component(player, Player::default());

        let mut controller = PlayerController;
        let mut ctx = ScriptContext {
            world: &mut world,
            dt: 1.0,
            tick: 0,
        };
        controller.update(&mut ctx);
        let t = world.get_component::<Transform>(player).unwrap();
        assert_eq!(t.translation.x, 0.0);
    }

    /// Exercise the exact C-ABI entry points the engine uses, in-process:
    /// create -> drive update through the erased `Box<dyn GameplayModule>` ->
    /// destroy. This proves the hot-reload boundary round-trips correctly.
    #[test]
    fn abi_entry_points_roundtrip() {
        assert_eq!(_nova_gameplay_abi_version(), nova_scripting::ABI_VERSION);

        let (mut world, player) = world_with_player_pressing_right();

        let ptr: *mut c_void = _nova_gameplay_create();
        assert!(!ptr.is_null());
        // SAFETY: `ptr` is a `*mut Box<dyn GameplayModule>` from create().
        unsafe {
            let module = &mut *(ptr as *mut Box<dyn GameplayModule>);
            let mut ctx = ScriptContext {
                world: &mut world,
                dt: 1.0,
                tick: 0,
            };
            module.update(&mut ctx);
        }
        let t = world.get_component::<Transform>(player).unwrap();
        assert!(t.translation.x > 4.9);

        // SAFETY: destroy the instance created above.
        unsafe { _nova_gameplay_destroy(ptr) };
    }
}
