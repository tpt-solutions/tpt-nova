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
}
