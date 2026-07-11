//! World-space UI: project 3D anchors to screen rectangles so the same 2D
//! draw primitives can render billboarded nameplates and in-world panels.
//!
//! The math is the standard clip-space projection; no GPU code, so it is fully
//! unit-testable. A backend draws the resulting [`WorldWidget`]s with the
//! usual [`crate::DrawCommand`]s (positioned by `screen`).

use glam::{Mat4, Vec3, Vec4};

use crate::{Color, DrawCommand, Rect, Theme};

/// A point in the world plus the label that should float above it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldAnchor {
    pub position: Vec3,
    pub text: String,
}

/// The screen-space result of projecting a [`WorldAnchor`].
#[derive(Debug, Clone, PartialEq)]
pub struct WorldWidget {
    /// Screen rectangle (pixels, y-down) for the label, centered on the anchor.
    pub screen: Rect,
    pub text: String,
    /// False when the anchor is behind the camera or outside the depth range.
    pub visible: bool,
    /// Linear depth in `[-1, 1]` NDC; useful for z-sorting overlapping tags.
    pub depth: f32,
}

/// Project a world position through `view_proj` to screen pixels.
///
/// `viewport` is (width, height) in pixels. Returns `None` when the point is
/// behind the camera (clip `w <= 0`). The returned z is NDC depth.
pub fn project_to_screen(
    world: Vec3,
    view_proj: Mat4,
    viewport: (f32, f32),
) -> Option<(glam::Vec2, f32)> {
    let clip = view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let sx = (ndc.x * 0.5 + 0.5) * viewport.0;
    let sy = (1.0 - (ndc.y * 0.5 + 0.5)) * viewport.1; // flip y to y-down
    Some((glam::Vec2::new(sx, sy), ndc.z))
}

/// Project one anchor to a screen-space widget centered on its position.
pub fn project_anchor(
    anchor: &WorldAnchor,
    view_proj: Mat4,
    viewport: (f32, f32),
    theme: &Theme,
) -> WorldWidget {
    let text_w = theme.text_width(&anchor.text) + theme.padding * 2.0;
    let text_h = theme.line_height() + theme.padding;
    match project_to_screen(anchor.position, view_proj, viewport) {
        Some((screen, depth)) => {
            let min = glam::Vec2::new(screen.x - text_w * 0.5, screen.y - text_h);
            let screen_rect = Rect::from_min_size(min, glam::Vec2::new(text_w, text_h));
            let visible = (-1.0..=1.0).contains(&depth);
            WorldWidget {
                screen: screen_rect,
                text: anchor.text.clone(),
                visible,
                depth,
            }
        }
        None => WorldWidget {
            screen: Rect::from_min_size(glam::Vec2::ZERO, glam::Vec2::ZERO),
            text: anchor.text.clone(),
            visible: false,
            depth: 1.0,
        },
    }
}

/// Project many anchors at once.
pub fn project_anchors(
    anchors: &[WorldAnchor],
    view_proj: Mat4,
    viewport: (f32, f32),
    theme: &Theme,
) -> Vec<WorldWidget> {
    anchors
        .iter()
        .map(|a| project_anchor(a, view_proj, viewport, theme))
        .collect()
}

/// Emit draw commands for the visible world widgets (a panel rect + label),
/// suitable for feeding straight into a renderer. Widgets farther away (higher
/// depth) are drawn first so nearer tags overlap them.
pub fn draw_world_widgets(widgets: &[WorldWidget], theme: &Theme) -> Vec<DrawCommand> {
    let mut ordered: Vec<&WorldWidget> = widgets.iter().filter(|w| w.visible).collect();
    ordered.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = Vec::new();
    for w in ordered {
        out.push(DrawCommand::Rect {
            rect: w.screen,
            color: theme.panel_bg,
            rounding: theme.rounding,
        });
        out.push(DrawCommand::Text {
            pos: glam::Vec2::new(
                w.screen.min.x + theme.padding,
                w.screen.min.y + theme.padding * 0.5,
            ),
            text: w.text.clone(),
            color: theme.text_color,
            size: theme.text_size,
        });
    }
    out
}

/// Convenience: a single translucent color used for world-space overlays.
pub fn overlay_color() -> Color {
    Color::rgba(0.1, 0.1, 0.14, 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_projects_to_viewport_center() {
        let vp = Mat4::IDENTITY;
        let (screen, depth) = project_to_screen(Vec3::ZERO, vp, (100.0, 100.0)).unwrap();
        assert!((screen.x - 50.0).abs() < 1e-4);
        assert!((screen.y - 50.0).abs() < 1e-4);
        assert!((depth - 0.0).abs() < 1e-4);
    }

    #[test]
    fn point_behind_camera_is_hidden() {
        // A real perspective projection sets clip w = -z, so a point at +z
        // (behind a camera that looks down -Z) has w <= 0 and is hidden.
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.01, 100.0);
        assert!(project_to_screen(Vec3::new(0.0, 0.0, 5.0), proj, (100.0, 100.0)).is_none());
        // A point in front (negative z) is visible.
        assert!(project_to_screen(Vec3::new(0.0, 0.0, -5.0), proj, (100.0, 100.0)).is_some());
    }

    #[test]
    fn widget_visibility_matches_depth_range() {
        let theme = Theme::default();
        let near = WorldAnchor {
            position: Vec3::new(0.0, 0.0, 0.5),
            text: "near".into(),
        };
        let w = project_anchor(&near, Mat4::IDENTITY, (100.0, 100.0), &theme);
        assert!(w.visible);
        assert!(w.screen.width() > 0.0);
        assert!(!w.text.is_empty());
    }

    #[test]
    fn draw_emits_rect_and_text_per_visible_widget() {
        let theme = Theme::default();
        let anchors = vec![
            WorldAnchor {
                position: Vec3::ZERO,
                text: "A".into(),
            },
            WorldAnchor {
                position: Vec3::new(0.0, 0.0, -5.0),
                text: "B".into(),
            },
        ];
        let widgets = project_anchors(&anchors, Mat4::IDENTITY, (200.0, 200.0), &theme);
        let draw = draw_world_widgets(&widgets, &theme);
        // Only "A" is visible: 1 rect + 1 text.
        assert_eq!(draw.len(), 2);
    }

    #[test]
    fn screen_y_is_flipped_from_ndc() {
        let vp = Mat4::IDENTITY;
        let upper = project_to_screen(Vec3::new(0.0, 1.0, 0.0), vp, (100.0, 100.0))
            .unwrap()
            .0;
        let lower = project_to_screen(Vec3::new(0.0, -1.0, 0.0), vp, (100.0, 100.0))
            .unwrap()
            .0;
        assert!(
            upper.y < lower.y,
            "higher world Y should map to smaller screen Y"
        );
    }
}
