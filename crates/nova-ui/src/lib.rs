//! Immediate-mode 2D UI for TPT Nova.
//!
//! Widgets are declared every frame; there is no retained widget tree. Each
//! frame you build a [`Ui`] from the current [`UiInput`], emit panels / buttons
//! / labels, and finish with a [`DrawList`] of renderer-agnostic primitives
//! (colored rectangles and text). A backend (e.g. the wgpu 2D pipeline) turns
//! those primitives into quads and glyphs, so the UI itself stays free of GPU
//! types and is fully unit-testable.
//!
//! ```
//! use nova_ui::{Ui, UiInput, Rect};
//! use glam::Vec2;
//!
//! let input = UiInput { pointer: Vec2::new(60.0, 80.0), pointer_pressed: true, ..Default::default() };
//! let mut ui = Ui::new(input);
//! ui.begin_panel(Rect::from_min_size(Vec2::new(40.0, 40.0), Vec2::new(200.0, 120.0)), Some("Menu"));
//! let play = ui.button("Play");
//! ui.label("v0.1.0");
//! ui.end_panel();
//! let _draw = ui.finish();
//! assert!(play.clicked);
//! ```

use glam::Vec2;

pub mod world;

pub use world::{
    draw_world_widgets, overlay_color, project_anchor, project_anchors, project_to_screen,
    WorldAnchor, WorldWidget,
};

/// RGBA color, components in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b, a: 1.0 }
    }
}

/// An axis-aligned rectangle in screen (pixel) space, y-down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Rect {
            min,
            max: min + size,
        }
    }
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    /// Shrink the rectangle inwards by `pad` on every side.
    pub fn shrink(&self, pad: f32) -> Rect {
        Rect {
            min: self.min + Vec2::splat(pad),
            max: self.max - Vec2::splat(pad),
        }
    }
}

/// Pointer/keyboard state for one UI frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiInput {
    /// Pointer position in screen pixels.
    pub pointer: Vec2,
    /// Pointer button currently held.
    pub pointer_down: bool,
    /// Pointer button went down this frame (a click edge).
    pub pointer_pressed: bool,
}

/// A drawing primitive produced by the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    /// A filled, optionally-rounded rectangle.
    Rect {
        rect: Rect,
        color: Color,
        rounding: f32,
    },
    /// A run of text with its top-left origin and pixel height.
    Text {
        pos: Vec2,
        text: String,
        color: Color,
        size: f32,
    },
}

/// The ordered list of primitives to render for a frame.
pub type DrawList = Vec<DrawCommand>;

/// The result of interacting with a widget this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Response {
    pub rect: Rect,
    pub hovered: bool,
    /// Pointer is held down over the widget.
    pub held: bool,
    /// A full click (press edge) landed on the widget this frame.
    pub clicked: bool,
}

/// Visual + layout parameters.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub text_size: f32,
    pub padding: f32,
    pub spacing: f32,
    pub title_bar_height: f32,
    pub panel_bg: Color,
    pub title_bg: Color,
    pub text_color: Color,
    pub button_bg: Color,
    pub button_hover: Color,
    pub button_active: Color,
    pub rounding: f32,
    /// Approximate glyph advance as a fraction of text size (monospace model).
    pub char_advance: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            text_size: 16.0,
            padding: 8.0,
            spacing: 6.0,
            title_bar_height: 22.0,
            panel_bg: Color::rgba(0.12, 0.12, 0.14, 0.95),
            title_bg: Color::rgba(0.20, 0.20, 0.26, 1.0),
            text_color: Color::rgb(0.92, 0.92, 0.95),
            button_bg: Color::rgba(0.24, 0.24, 0.30, 1.0),
            button_hover: Color::rgba(0.32, 0.32, 0.42, 1.0),
            button_active: Color::rgba(0.40, 0.44, 0.60, 1.0),
            rounding: 3.0,
            char_advance: 0.55,
        }
    }
}

impl Theme {
    fn line_height(&self) -> f32 {
        self.text_size * 1.3
    }
    fn text_width(&self, text: &str) -> f32 {
        text.chars().count() as f32 * self.text_size * self.char_advance
    }
}

/// A vertical layout region: widgets stack top-to-bottom from `cursor`.
#[derive(Debug, Clone, Copy)]
struct Layout {
    cursor: Vec2,
    bounds: Rect,
}

/// The per-frame UI builder.
pub struct Ui {
    input: UiInput,
    theme: Theme,
    draw: DrawList,
    stack: Vec<Layout>,
}

impl Ui {
    pub fn new(input: UiInput) -> Self {
        Ui::with_theme(input, Theme::default())
    }

    pub fn with_theme(input: UiInput, theme: Theme) -> Self {
        Ui {
            input,
            theme,
            draw: Vec::new(),
            stack: Vec::new(),
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Consume the UI and return its draw list.
    pub fn finish(self) -> DrawList {
        self.draw
    }

    fn current(&mut self) -> &mut Layout {
        self.stack
            .last_mut()
            .expect("no active layout; call begin_panel first")
    }

    /// Reserve `size` at the current cursor and advance the layout downward.
    fn allocate(&mut self, size: Vec2) -> Rect {
        let spacing = self.theme.spacing;
        let layout = self.current();
        let min = layout.cursor;
        let rect = Rect::from_min_size(min, size);
        layout.cursor.y += size.y + spacing;
        rect
    }

    fn interact(&self, rect: Rect) -> Response {
        let hovered = rect.contains(self.input.pointer);
        Response {
            rect,
            hovered,
            held: hovered && self.input.pointer_down,
            clicked: hovered && self.input.pointer_pressed,
        }
    }

    // ---- Containers -----------------------------------------------------

    /// Begin a panel occupying `rect`, with an optional title bar. Subsequent
    /// widgets are laid out inside it until [`Ui::end_panel`].
    pub fn begin_panel(&mut self, rect: Rect, title: Option<&str>) {
        self.draw.push(DrawCommand::Rect {
            rect,
            color: self.theme.panel_bg,
            rounding: self.theme.rounding,
        });

        let mut content_top = rect.min.y;
        if let Some(title) = title {
            let bar = Rect::from_min_size(
                rect.min,
                Vec2::new(rect.width(), self.theme.title_bar_height),
            );
            self.draw.push(DrawCommand::Rect {
                rect: bar,
                color: self.theme.title_bg,
                rounding: self.theme.rounding,
            });
            self.draw.push(DrawCommand::Text {
                pos: Vec2::new(rect.min.x + self.theme.padding, rect.min.y + 4.0),
                text: title.to_string(),
                color: self.theme.text_color,
                size: self.theme.text_size,
            });
            content_top += self.theme.title_bar_height;
        }

        let bounds = Rect {
            min: Vec2::new(rect.min.x, content_top),
            max: rect.max,
        };
        let inner = bounds.shrink(self.theme.padding);
        self.stack.push(Layout {
            cursor: inner.min,
            bounds: inner,
        });
    }

    pub fn end_panel(&mut self) {
        self.stack.pop();
    }

    // ---- Widgets --------------------------------------------------------

    /// A non-interactive text label.
    pub fn label(&mut self, text: &str) -> Response {
        let size = Vec2::new(self.theme.text_width(text), self.theme.line_height());
        let rect = self.allocate(size);
        self.draw.push(DrawCommand::Text {
            pos: rect.min,
            text: text.to_string(),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        self.interact(rect)
    }

    /// A clickable button. Inspect [`Response::clicked`].
    pub fn button(&mut self, label: &str) -> Response {
        let pad = self.theme.padding;
        let w = self.theme.text_width(label) + pad * 2.0;
        let h = self.theme.line_height() + pad;
        // Fill the panel width when the label is narrow, for a tidy stack.
        let avail = self.current().bounds.width();
        let size = Vec2::new(w.max(avail.min(w.max(80.0))), h);
        let rect = self.allocate(size);
        let resp = self.interact(rect);

        let bg = if resp.held {
            self.theme.button_active
        } else if resp.hovered {
            self.theme.button_hover
        } else {
            self.theme.button_bg
        };
        self.draw.push(DrawCommand::Rect {
            rect,
            color: bg,
            rounding: self.theme.rounding,
        });
        self.draw.push(DrawCommand::Text {
            pos: Vec2::new(rect.min.x + pad, rect.min.y + pad * 0.5),
            text: label.to_string(),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        resp
    }

    /// Add vertical space.
    pub fn space(&mut self, amount: f32) {
        self.current().cursor.y += amount;
    }
}

/// State for a horizontal drag widget (e.g. [`Ui::drag_float`]). The host
/// owns one per active drag and passes it in each frame; the widget borrows
/// it to remember where the drag started so it can apply a *relative* delta
/// (the widget itself is rebuilt from scratch every frame in immediate
/// mode, so the drag origin cannot live on the transient `Ui`).
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    pub active: bool,
    start_pointer: Vec2,
    start_value: f32,
}

impl Default for DragState {
    fn default() -> Self {
        DragState {
            active: false,
            start_pointer: Vec2::ZERO,
            start_value: 0.0,
        }
    }
}

impl DragState {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ui {
    // ---- Editable widgets ------------------------------------------------

    /// A clickable checkbox that toggles `value` on the press edge. Returns true
    /// if the value changed this frame.
    pub fn checkbox(&mut self, label: &str, value: &mut bool) -> bool {
        let h = self.theme.line_height();
        let size = Vec2::new(self.current().bounds.width(), h);
        let rect = self.allocate(size);
        let resp = self.interact(rect);
        if resp.clicked {
            *value = !*value;
        }
        let box_size = h * 0.7;
        self.draw.push(DrawCommand::Rect {
            rect: Rect::from_min_size(rect.min, Vec2::new(box_size, box_size)),
            color: if *value {
                self.theme.button_active
            } else {
                self.theme.button_bg
            },
            rounding: 2.0,
        });
        self.draw.push(DrawCommand::Text {
            pos: Vec2::new(
                rect.min.x + box_size + self.theme.padding,
                rect.min.y + self.theme.padding * 0.5,
            ),
            text: label.to_string(),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        resp.clicked
    }

    /// A horizontally-draggable float. While the pointer is held inside the
    /// widget (after a press), `value` changes by `speed` per pixel of horizontal
    /// movement, relative to the drag origin captured on press. `drag` must be
    /// owned by the host and passed every frame so the drag survives across
    /// frames. Returns true if `value` changed this frame.
    pub fn drag_float(
        &mut self,
        label: &str,
        value: &mut f32,
        speed: f32,
        drag: &mut DragState,
    ) -> bool {
        let h = self.theme.line_height();
        let size = Vec2::new(self.current().bounds.width(), h);
        let rect = self.allocate(size);
        let resp = self.interact(rect);

        if resp.clicked {
            drag.active = true;
            drag.start_pointer = self.input.pointer;
            drag.start_value = *value;
        }
        let mut changed = false;
        if drag.active {
            if self.input.pointer_down {
                let nv = drag.start_value + (self.input.pointer.x - drag.start_pointer.x) * speed;
                changed = (nv - *value).abs() > 1e-6;
                *value = nv;
            } else {
                drag.active = false;
            }
        }

        self.draw.push(DrawCommand::Text {
            pos: Vec2::new(
                rect.min.x + self.theme.padding,
                rect.min.y + self.theme.padding * 0.5,
            ),
            text: label.to_string(),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        self.draw.push(DrawCommand::Text {
            pos: Vec2::new(
                rect.max.x - self.theme.text_width(&format!("{value:.3}")) - self.theme.padding,
                rect.min.y + self.theme.padding * 0.5,
            ),
            text: format!("{value:.3}"),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        changed
    }

    /// A horizontal slider mapping the pointer's x within the widget to a value
    /// in `[min, max]`. Reads the absolute pointer position, so it needs no host
    /// state and works for both clicks and drags. Returns true if `value`
    /// changed this frame.
    pub fn slider(&mut self, label: &str, value: &mut f32, min: f32, max: f32) -> bool {
        let h = self.theme.line_height();
        let size = Vec2::new(self.current().bounds.width(), h);
        let rect = self.allocate(size);
        let resp = self.interact(rect);

        let mut changed = false;
        if resp.held || resp.clicked {
            let t = ((self.input.pointer.x - rect.min.x) / rect.width().max(1e-3)).clamp(0.0, 1.0);
            let nv = min + t * (max - min);
            changed = (nv - *value).abs() > 1e-6;
            *value = nv;
        }

        // Track background.
        let track = Rect::from_min_size(
            Vec2::new(rect.min.x + self.theme.padding, rect.min.y + h * 0.5 - 2.0),
            Vec2::new(rect.width() - self.theme.padding * 2.0, 4.0),
        );
        self.draw.push(DrawCommand::Rect {
            rect: track,
            color: self.theme.button_bg,
            rounding: 2.0,
        });
        // Knob.
        let t = if max > min {
            ((*value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let knob_x = track.min.x + t * track.width();
        self.draw.push(DrawCommand::Rect {
            rect: Rect::from_min_size(
                Vec2::new(knob_x - 4.0, track.min.y - 2.0),
                Vec2::new(8.0, 8.0),
            ),
            color: self.theme.button_active,
            rounding: 2.0,
        });
        self.draw.push(DrawCommand::Text {
            pos: Vec2::new(
                rect.min.x + self.theme.padding,
                rect.min.y + self.theme.padding * 0.5,
            ),
            text: format!("{label}: {value:.3}"),
            color: self.theme.text_color,
            size: self.theme.text_size,
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_rect() -> Rect {
        Rect::from_min_size(Vec2::new(40.0, 40.0), Vec2::new(200.0, 160.0))
    }

    #[test]
    fn button_click_detected_when_pointer_inside_and_pressed() {
        // First, find where the button lands with no interaction.
        let mut probe = Ui::new(UiInput::default());
        probe.begin_panel(panel_rect(), Some("Menu"));
        let r = probe.button("Play");
        probe.end_panel();
        let center = (r.rect.min + r.rect.max) * 0.5;

        // Now click at that position.
        let input = UiInput {
            pointer: center,
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut ui = Ui::new(input);
        ui.begin_panel(panel_rect(), Some("Menu"));
        let play = ui.button("Play");
        ui.end_panel();
        assert!(play.hovered);
        assert!(play.clicked);
    }

    #[test]
    fn no_click_when_pointer_outside() {
        let input = UiInput {
            pointer: Vec2::new(5.0, 5.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut ui = Ui::new(input);
        ui.begin_panel(panel_rect(), None);
        let b = ui.button("Play");
        ui.end_panel();
        assert!(!b.hovered);
        assert!(!b.clicked);
    }

    #[test]
    fn widgets_stack_without_overlapping() {
        let mut ui = Ui::new(UiInput::default());
        ui.begin_panel(panel_rect(), None);
        let a = ui.button("First");
        let b = ui.button("Second");
        ui.end_panel();
        assert!(
            b.rect.min.y >= a.rect.max.y,
            "second widget should be below the first: {} vs {}",
            b.rect.min.y,
            a.rect.max.y
        );
    }

    #[test]
    fn draw_list_has_panel_and_widget_primitives() {
        let mut ui = Ui::new(UiInput::default());
        ui.begin_panel(panel_rect(), Some("Title"));
        ui.label("Hello");
        ui.button("Ok");
        ui.end_panel();
        let draw = ui.finish();
        let rects = draw
            .iter()
            .filter(|c| matches!(c, DrawCommand::Rect { .. }))
            .count();
        let texts = draw
            .iter()
            .filter(|c| matches!(c, DrawCommand::Text { .. }))
            .count();
        // panel bg + title bar + button bg = 3 rects; title + label + button = 3 texts.
        assert_eq!(rects, 3);
        assert_eq!(texts, 3);
    }

    #[test]
    fn panel_without_title_omits_title_primitives() {
        let mut ui = Ui::new(UiInput::default());
        ui.begin_panel(panel_rect(), None);
        ui.button("Ok");
        ui.end_panel();
        let draw = ui.finish();
        let rects = draw
            .iter()
            .filter(|c| matches!(c, DrawCommand::Rect { .. }))
            .count();
        let texts = draw
            .iter()
            .filter(|c| matches!(c, DrawCommand::Text { .. }))
            .count();
        // panel bg + button bg = 2 rects; button label = 1 text (no title bar).
        assert_eq!(rects, 2);
        assert_eq!(texts, 1);
    }

    #[test]
    fn checkbox_toggles_on_click() {
        let input = UiInput {
            pointer: Vec2::new(60.0, 60.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut ui = Ui::new(input);
        ui.begin_panel(panel_rect(), None);
        let mut v = false;
        let changed = ui.checkbox("on", &mut v);
        ui.end_panel();
        assert!(changed);
        assert!(v);
        // A second click toggles back off.
        let input2 = UiInput {
            pointer: Vec2::new(60.0, 60.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut ui2 = Ui::new(input2);
        ui2.begin_panel(panel_rect(), None);
        let changed2 = ui2.checkbox("on", &mut v);
        ui2.end_panel();
        assert!(changed2);
        assert!(!v);
    }

    #[test]
    fn drag_float_changes_value_relative_to_origin() {
        // Press to begin the drag, then move +30px to the right while held.
        let press = UiInput {
            pointer: Vec2::new(60.0, 60.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut drag = DragState::new();
        let mut v = 1.0f32;

        let mut ui = Ui::new(press);
        ui.begin_panel(panel_rect(), None);
        let _ = ui.drag_float("x", &mut v, 0.1, &mut drag);
        ui.end_panel();
        assert!(drag.active, "drag should be active after a press inside");

        let move_in = UiInput {
            pointer: Vec2::new(90.0, 60.0),
            pointer_down: true,
            pointer_pressed: false,
        };
        let mut ui2 = Ui::new(move_in);
        ui2.begin_panel(panel_rect(), None);
        let changed = ui2.drag_float("x", &mut v, 0.1, &mut drag);
        ui2.end_panel();
        assert!(changed);
        assert!(
            (v - 4.0).abs() < 1e-3,
            "value = start 1.0 + 30px * 0.1 = 4.0, got {v}"
        );

        // Releasing ends the drag.
        let release = UiInput {
            pointer: Vec2::new(90.0, 60.0),
            pointer_down: false,
            pointer_pressed: false,
        };
        let mut ui3 = Ui::new(release);
        ui3.begin_panel(panel_rect(), None);
        let _ = ui3.drag_float("x", &mut v, 0.1, &mut drag);
        ui3.end_panel();
        assert!(!drag.active);
    }

    #[test]
    fn slider_maps_pointer_x_to_value() {
        // First widget in an untitled panel_rect (40,40)+200 wide: x in [48,232].
        let input = UiInput {
            pointer: Vec2::new(140.0, 56.0),
            pointer_down: true,
            pointer_pressed: true,
        };
        let mut ui = Ui::new(input);
        ui.begin_panel(panel_rect(), None);
        let mut v = 0.0f32;
        let changed = ui.slider("s", &mut v, 0.0, 10.0);
        ui.end_panel();
        assert!(changed);
        // Pointer near the middle of the track should map to ~5.0.
        assert!(v > 4.0 && v < 6.0, "middle maps to ~5, got {v}");
    }
}
