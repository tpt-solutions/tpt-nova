//! Headless example of `nova-render`'s pure (no-GPU) helpers: build an
//! orthographic screen matrix and turn a `nova-ui` draw list into GPU-ready
//! vertices. Runs without a window or GPU.

use glam::{Vec2, Vec3};
use nova_render::build_ui_vertices;
use nova_render::sprite::screen_ortho;
use nova_ui::{Color, DrawCommand, DrawList, Rect};

fn main() {
    let (w, h) = (1280.0f32, 720.0f32);

    // An orthographic projection mapping screen pixels to clip space — the same
    // matrix the 2D sprite pipeline feeds the vertex shader.
    let ortho = screen_ortho(w, h);
    println!(
        "ortho maps screen center to clip {:?}",
        ortho.transform_point3(Vec3::new(w * 0.5, h * 0.5, 0.0))
    );

    // Assemble a small UI draw list and bake it into vertices/indices.
    let draw: DrawList = vec![
        DrawCommand::Rect {
            rect: Rect::from_min_size(Vec2::new(100.0, 100.0), Vec2::new(200.0, 60.0)),
            color: Color::rgb(0.2, 0.9, 0.3),
            rounding: 4.0,
        },
        DrawCommand::Text {
            pos: Vec2::new(110.0, 110.0),
            text: "hello nova-render".to_string(),
            color: Color::rgb(1.0, 1.0, 1.0),
            size: 16.0,
        },
    ];

    let (verts, indices) = build_ui_vertices(&draw, (w as u32, h as u32));
    println!(
        "baked {} draw commands into {} vertices / {} indices",
        draw.len(),
        verts.len(),
        indices.len()
    );
}
