//! Keyboard/mouse input as an ECS resource, with basic action mapping.
//!
//! `nova-input` is decoupled from the engine: it consumes winit window events
//! and accumulates them into an [`InputState`] resource. Gameplay code reads
//! semantic actions through an [`ActionMap`] instead of raw keys, so rebinding
//! is a data change rather than a code change.

pub use winit::event::MouseButton;
pub use winit::keyboard::{KeyCode, PhysicalKey};

use std::collections::{HashMap, HashSet};

use winit::event::WindowEvent;

/// Live and per-frame input state.
#[derive(Debug, Default, Clone)]
pub struct InputState {
    /// Keys currently held down.
    pub keys: HashSet<KeyCode>,
    /// Keys that went down since the last `end_frame`.
    pub pressed_this_frame: HashSet<KeyCode>,
    /// Keys that went up since the last `end_frame`.
    pub released_this_frame: HashSet<KeyCode>,
    /// Current cursor position in logical pixels.
    pub mouse_pos: (f32, f32),
    /// Cursor movement since the last `end_frame`.
    pub mouse_delta: (f32, f32),
    /// Mouse buttons currently held.
    pub buttons: HashSet<MouseButton>,
    /// Accumulated scroll delta since the last `end_frame`.
    pub scroll: f32,
}

impl InputState {
    /// Apply a winit window event, mutating the accumulated state.
    pub fn apply_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::event::ElementState;
                let code = match event.physical_key {
                    PhysicalKey::Code(c) => c,
                    _ => return,
                };
                match event.state {
                    ElementState::Pressed => {
                        if !self.keys.contains(&code) {
                            self.pressed_this_frame.insert(code);
                        }
                        self.keys.insert(code);
                    }
                    ElementState::Released => {
                        self.keys.remove(&code);
                        self.released_this_frame.insert(code);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as f32;
                let y = position.y as f32;
                let dx = x - self.mouse_pos.0;
                let dy = y - self.mouse_pos.1;
                self.mouse_pos = (x, y);
                // Only accumulate delta once we have a prior position.
                if self.mouse_pos != (x, y) {
                    self.mouse_delta.0 += dx;
                    self.mouse_delta.1 += dy;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                use winit::event::ElementState;
                match state {
                    ElementState::Pressed => {
                        self.buttons.insert(*button);
                    }
                    ElementState::Released => {
                        self.buttons.remove(button);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                match delta {
                    MouseScrollDelta::LineDelta(_, y) => self.scroll += *y,
                    MouseScrollDelta::PixelDelta(p) => self.scroll += p.y as f32,
                }
            }
            _ => {}
        }
    }

    /// Clear per-frame accumulators. Call once at the end of each frame.
    pub fn end_frame(&mut self) {
        self.pressed_this_frame.clear();
        self.released_this_frame.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll = 0.0;
    }

    pub fn is_key_down(&self, code: KeyCode) -> bool {
        self.keys.contains(&code)
    }

    pub fn key_just_pressed(&self, code: KeyCode) -> bool {
        self.pressed_this_frame.contains(&code)
    }
}

/// Maps semantic action names to one or more physical keys.
#[derive(Debug, Clone, Default)]
pub struct ActionMap {
    bindings: HashMap<String, Vec<KeyCode>>,
}

impl ActionMap {
    pub fn new() -> Self {
        ActionMap::default()
    }

    /// Bind an action to a set of keys. Any previous binding is replaced.
    pub fn bind(&mut self, action: impl Into<String>, keys: Vec<KeyCode>) -> &mut Self {
        self.bindings.insert(action.into(), keys);
        self
    }

    /// True if any bound key for `action` is currently held.
    pub fn is_active(&self, state: &InputState, action: &str) -> bool {
        self.bindings
            .get(action)
            .map(|keys| keys.iter().any(|k| state.keys.contains(k)))
            .unwrap_or(false)
    }

    /// True if any bound key for `action` was pressed this frame.
    pub fn just_triggered(&self, state: &InputState, action: &str) -> bool {
        self.bindings
            .get(action)
            .map(|keys| keys.iter().any(|k| state.pressed_this_frame.contains(k)))
            .unwrap_or(false)
    }

    pub fn bindings(&self) -> &HashMap<String, Vec<KeyCode>> {
        &self.bindings
    }
}

/// Convenience constructor for the default action map used by the sample app.
pub fn default_action_map() -> ActionMap {
    let mut map = ActionMap::new();
    map.bind("move_forward", vec![KeyCode::KeyW, KeyCode::ArrowUp])
        .bind("move_back", vec![KeyCode::KeyS, KeyCode::ArrowDown])
        .bind("move_left", vec![KeyCode::KeyA, KeyCode::ArrowLeft])
        .bind("move_right", vec![KeyCode::KeyD, KeyCode::ArrowRight])
        .bind("spin_up", vec![KeyCode::KeyQ])
        .bind("spin_down", vec![KeyCode::KeyE]);
    map
}
