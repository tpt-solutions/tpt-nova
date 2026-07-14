//! Headless `nova-render` demo of its GPU-free data pieces.
//!
//! `Renderer::new` needs a real window + GPU, so this example instead
//! showcases the two pure, unit-testable parts of the crate:
//!   1. `build_ui_vertices` — turning an editor `DrawList` (panels/buttons)
//!      into a vertex/index buffer ready for the 2D UI overlay pass.
//!   2. The camera view-projection math (`Camera::perspective` composed with a
//!      world transform's inverse), independent of any device.
//!
//! Run with: `cargo run -p nova-render --example ui_draw_list`
//! (or `cargo build -p nova-render --examples` to type-check it).

use glam::Vec2;
use nova_ecs::transform::{Camera, GlobalTransform};
use nova_ecs::{Mat4, Quat, Vec3};
use nova_render::build_ui_vertices;
use nova_ui::{Color, DrawCommand, Rect, Ui, UiInput};

fn main() {
    // ---- 1. Build an editor-style DrawList and bake it to buffers ----------
    let area = Rect::from_min_size(Vec2::new(40.0, 40.0), Vec2::new(240.0, 400.0));
    let input = UiInput {
        pointer: Vec2::new(60.0, 45.0),
        pointer_down: true,
        pointer_pressed: true,
        ..Default::default()
    };
    let mut ui = Ui::new(input);
    ui.begin_panel(area, Some("Hierarchy"));
    ui.label("e0#0");
    ui.button("e1#0");
    ui.end_panel();
    let draw = ui.finish();
    assert!(!draw.is_empty(), "panel must emit draw primitives");

    let (verts, indices) = build_ui_vertices(&draw, (800, 600));
    // Each Rect command becomes 4 vertices / 6 indices.
    let rect_count = draw
        .iter()
        .filter(|c| matches!(c, DrawCommand::Rect { .. }))
        .count();
    assert_eq!(verts.len(), rect_count * 4);
    assert_eq!(indices.len(), rect_count * 6);
    println!(
        "ui_draw_list: {rect_count} rects -> {v} verts, {i} indices",
        v = verts.len(),
        i = indices.len()
    );

    // A zero/negative-area rect is dropped silently.
    let degenerate: nova_ui::DrawList = vec![DrawCommand::Rect {
        rect: Rect::from_min_size(Vec2::ZERO, Vec2::new(0.0, 10.0)),
        color: Color::rgb(1.0, 0.0, 0.0),
        rounding: 0.0,
    }];
    let (empty_verts, empty_idx) = build_ui_vertices(&degenerate, (800, 600));
    assert!(empty_verts.is_empty() && empty_idx.is_empty());

    // ---- 2. Camera view-projection math -----------------------------------
    let cam = Camera {
        aspect: 16.0 / 9.0,
        ..Camera::default()
    };
    // Camera sits 5 units back on +Z looking down -Z (like `compute_view_proj`).
    let gt = GlobalTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0)));
    let view_proj = cam.perspective() * gt.0.inverse();

    // The world origin should land in front of the camera (view-space z = -5).
    let origin_clip = view_proj * Vec3::new(0.0, 0.0, 0.0).extend(1.0);
    let origin_view = gt.0.inverse() * Vec3::new(0.0, 0.0, 0.0).extend(1.0);
    assert!(
        (origin_view.z + 5.0).abs() < 1e-4,
        "origin must sit 5 units ahead"
    );

    // A point directly under the camera should project to clip x,y ~ 0.
    let p = Vec3::new(0.0, 0.0, 0.0);
    let clip = view_proj * p.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    println!(
        "ui_draw_list: world {p:?} -> ndc ({:.3}, {:.3}, {:.3})",
        ndc.x, ndc.y, ndc.z
    );
    assert!(
        ndc.x.abs() < 1e-3 && ndc.y.abs() < 1e-3,
        "centered point maps to center"
    );

    // `Quat` is re-exported too, so rotation-based cameras compose cleanly.
    let _rot = Quat::IDENTITY;
    println!("ui_draw_list: OK (origin_clip.w = {:.3})", origin_clip.w);
}
