//! Keyboard/mouse input as an ECS resource, with basic action mapping.
//!
//! `nova-input` is decoupled from the engine: it consumes winit window events
//! and accumulates them into an [`InputState`] resource. Gameplay code reads
//! semantic actions through an [`ActionMap`] instead of raw keys, so rebinding
//! is a data change rather than a code change.

pub use winit::event::MouseButton;
pub use winit::keyboard::{KeyCode, PhysicalKey};

use std::collections::{HashMap, HashSet};

use winit::event::{ElementState, WindowEvent};

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
    /// Text typed on the keyboard since the last `end_frame`, in input order.
    /// Populated from winit `KeyEvent::text` on press; the bespoke UI is
    /// pointer-only, so this is the bridge that lets a widget (e.g. the
    /// Highlight & Fix instruction field) receive keystrokes without owning the
    /// OS keyboard focus. Cleared by `end_frame`.
    pub text_entered: String,
    /// Whether a real cursor position has been observed yet. Until the first
    /// `CursorMoved` we have no "prior position" to diff against, so the first
    /// sample must not produce a spurious delta.
    seeded: bool,
}

impl InputState {
    /// Apply a winit window event, mutating the accumulated state.
    pub fn apply_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let code = match event.physical_key {
                    PhysicalKey::Code(c) => c,
                    _ => return,
                };
                Self::apply_key(self, code, event.state);
                // Capture typed text for pointer-only UI widgets. `text` is only
                // present on the press edge for printable keys.
                if event.state == ElementState::Pressed {
                    if let Some(text) = &event.text {
                        self.text_entered.push_str(text);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.apply_cursor_moved(position.x, position.y);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.apply_mouse_button(*button, *state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.apply_mouse_wheel(*delta);
            }
            _ => {}
        }
    }

    /// Apply a single key transition. Split out from [`InputState::apply_event`]
    /// so the keyboard mapping can be unit-tested without constructing a winit
    /// `KeyEvent` (which has a private field and no public constructor).
    pub(crate) fn apply_key(&mut self, code: KeyCode, state: winit::event::ElementState) {
        use winit::event::ElementState;
        match state {
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

    /// Apply a cursor movement. Like [`InputState::apply_key`], this is the
    /// testable core that [`InputState::apply_event`] delegates to.
    pub(crate) fn apply_cursor_moved(&mut self, x: f64, y: f64) {
        let x = x as f32;
        let y = y as f32;
        let dx = x - self.mouse_pos.0;
        let dy = y - self.mouse_pos.1;
        let had_prior = self.seeded;
        self.mouse_pos = (x, y);
        self.seeded = true;
        // Only accumulate delta once we have a prior position.
        if had_prior {
            self.mouse_delta.0 += dx;
            self.mouse_delta.1 += dy;
        }
    }

    /// Apply a mouse button transition.
    pub(crate) fn apply_mouse_button(
        &mut self,
        button: MouseButton,
        state: winit::event::ElementState,
    ) {
        use winit::event::ElementState;
        match state {
            ElementState::Pressed => {
                self.buttons.insert(button);
            }
            ElementState::Released => {
                self.buttons.remove(&button);
            }
        }
    }

    /// Apply a scroll-wheel delta.
    pub(crate) fn apply_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        use winit::event::MouseScrollDelta;
        match delta {
            MouseScrollDelta::LineDelta(_, y) => self.scroll += y,
            MouseScrollDelta::PixelDelta(p) => self.scroll += p.y as f32,
        }
    }

    /// Clear per-frame accumulators. Call once at the end of each frame.
    pub fn end_frame(&mut self) {
        self.pressed_this_frame.clear();
        self.released_this_frame.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll = 0.0;
        self.text_entered.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::keyboard::KeyCode;

    // ---- Keyboard event -> InputState mapping ----------------------------

    #[test]
    fn key_press_sets_down_and_pressed_sets() {
        let mut s = InputState::default();
        s.apply_key(KeyCode::KeyW, ElementState::Pressed);
        assert!(s.is_key_down(KeyCode::KeyW));
        assert!(s.key_just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn key_release_removes_from_down_and_records_release() {
        let mut s = InputState::default();
        s.apply_key(KeyCode::KeyW, ElementState::Pressed);
        s.apply_key(KeyCode::KeyW, ElementState::Released);
        assert!(!s.is_key_down(KeyCode::KeyW));
        assert!(s.released_this_frame.contains(&KeyCode::KeyW));
        // `pressed_this_frame` is only cleared by `end_frame`, so a press that
        // happened earlier in the same frame is still recorded there.
        assert!(s.pressed_this_frame.contains(&KeyCode::KeyW));
    }

    #[test]
    fn repeat_press_does_not_re_add_to_pressed_set() {
        let mut s = InputState::default();
        s.apply_key(KeyCode::KeyA, ElementState::Pressed);
        // Auto-repeat: key already down, another press arrives.
        s.apply_key(KeyCode::KeyA, ElementState::Pressed);
        assert_eq!(s.pressed_this_frame.len(), 1);
    }

    #[test]
    fn apply_event_ignores_unrelated_window_events() {
        // `WindowEvent::Focused` carries no input; it must not disturb state.
        let mut s = InputState::default();
        s.apply_key(KeyCode::KeyW, ElementState::Pressed);
        s.apply_event(&WindowEvent::Focused(true));
        assert!(s.is_key_down(KeyCode::KeyW));
        assert_eq!(s.scroll, 0.0);
        assert_eq!(s.mouse_pos, (0.0, 0.0));
    }

    // ---- Mouse event -> InputState mapping -------------------------------
    // These call the `pub(crate)` helpers that `apply_event` delegates to, since
    // winit's `WindowEvent` variants carry a `DeviceId` with no public
    // constructor.

    #[test]
    fn cursor_moved_updates_position_and_delta() {
        let mut s = InputState::default();
        s.apply_cursor_moved(10.0, 20.0);
        assert_eq!(s.mouse_pos, (10.0, 20.0));
        // First move: no prior position, delta stays zero.
        assert_eq!(s.mouse_delta, (0.0, 0.0));

        s.apply_cursor_moved(15.0, 25.0);
        assert_eq!(s.mouse_pos, (15.0, 25.0));
        assert_eq!(s.mouse_delta, (5.0, 5.0));
    }

    #[test]
    fn mouse_button_down_and_up() {
        let mut s = InputState::default();
        s.apply_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert!(s.buttons.contains(&MouseButton::Left));

        s.apply_mouse_button(MouseButton::Left, ElementState::Released);
        assert!(!s.buttons.contains(&MouseButton::Left));
    }

    #[test]
    fn mouse_wheel_line_and_pixel_deltas() {
        let mut s = InputState::default();
        s.apply_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 3.0));
        assert_eq!(s.scroll, 3.0);

        s.apply_mouse_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0_f64, 10.0_f64,
        )));
        assert_eq!(s.scroll, 13.0);
    }

    #[test]
    fn text_entered_cleared_by_end_frame() {
        let mut s = InputState::default();
        s.text_entered.push_str("hello");
        assert_eq!(s.text_entered, "hello");
        s.end_frame();
        assert_eq!(s.text_entered, "");
        // Held keys persist across frames (only per-frame text is cleared).
        s.keys.insert(KeyCode::KeyW);
        assert!(s.is_key_down(KeyCode::KeyW));
    }

    // ---- Action mapping --------------------------------------------------

    #[test]
    fn default_action_map_has_expected_bindings() {
        let map = default_action_map();
        let state = InputState::default();
        // Unbound keys report no activity.
        assert!(!map.is_active(&state, "move_forward"));
        assert!(!map.is_active(&state, "does_not_exist"));

        // W and ArrowUp both drive move_forward.
        assert!(map
            .bindings()
            .get("move_forward")
            .map(|k| k.contains(&KeyCode::KeyW) && k.contains(&KeyCode::ArrowUp))
            .unwrap_or(false));
    }

    #[test]
    fn action_active_when_bound_key_held() {
        let map = default_action_map();
        let mut state = InputState::default();
        state.keys.insert(KeyCode::KeyW);
        assert!(map.is_active(&state, "move_forward"));
        assert!(!map.is_active(&state, "move_back"));

        state.keys.insert(KeyCode::ArrowDown);
        assert!(map.is_active(&state, "move_back"));
    }

    #[test]
    fn action_just_triggered_only_on_press_frame() {
        let map = default_action_map();
        let mut state = InputState::default();
        state.keys.insert(KeyCode::KeyW);
        // Held, not freshly pressed -> just_triggered is false.
        assert!(map.is_active(&state, "move_forward"));
        assert!(!map.just_triggered(&state, "move_forward"));

        state.pressed_this_frame.insert(KeyCode::KeyW);
        assert!(map.just_triggered(&state, "move_forward"));
    }

    #[test]
    fn rebinding_replaces_previous_keys() {
        let mut map = ActionMap::new();
        map.bind("jump", vec![KeyCode::Space]);
        assert!(map.is_active(
            &InputState {
                keys: {
                    let mut h = std::collections::HashSet::new();
                    h.insert(KeyCode::Space);
                    h
                },
                ..Default::default()
            },
            "jump"
        ));

        map.bind("jump", vec![KeyCode::KeyF]);
        let mut state = InputState::default();
        state.keys.insert(KeyCode::Space);
        assert!(!map.is_active(&state, "jump"));
        state.keys.insert(KeyCode::KeyF);
        assert!(map.is_active(&state, "jump"));
    }

    #[test]
    fn unbound_action_is_noop() {
        let map = ActionMap::new();
        let mut state = InputState::default();
        state.keys.insert(KeyCode::KeyW);
        assert!(!map.is_active(&state, "anything"));
        assert!(!map.just_triggered(&state, "anything"));
    }
}
