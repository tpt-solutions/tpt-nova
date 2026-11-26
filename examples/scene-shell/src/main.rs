//! A tpt-nova 3D scene in a tpt-shell window. The scene is rendered on the GPU through
//! `AppfrontScene` and an appfront `Viewport3D`. Each frame's pixels are sent to the page,
//! which draws them on a canvas. Windows only.
//!
//! usage: cargo run --release -- <path to WebView2Loader.dll>

use std::thread;
use std::time::Duration;

use glam::{Quat, Vec3};
use tpt_appfront_canvas_3d::{PresentStatus, Viewport3D, ViewportSize};
use tpt_nova_scene_graph::appfront::AppfrontScene;
use tpt_nova_scene_graph::{
    Color, DirectionalLight, Node, NodeId, PerspectiveCamera, Scene, Transform,
};
use tpt_shell::{App, Window};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;

// The page's script and CSS are separate assets: the shell's policy blocks inline code.
const PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>scene</title>
<link rel="stylesheet" href="scene.css">
<canvas id="view"></canvas>
<div id="status">starting</div>
<script src="scene.js"></script>
"#;

const STYLE: &str = r#"html, body { margin: 0; height: 100%; background: #101820; }
canvas { display: block; width: 100vw; height: 100vh; image-rendering: pixelated; }
#status { position: fixed; left: 8px; top: 8px; padding: 4px 8px; color: #fff;
  background: rgba(0, 0, 0, 0.6); font: 14px sans-serif; }
"#;

const SCRIPT: &str = r#"(function () {
  var canvas = document.getElementById('view');
  var ctx = canvas.getContext('2d');
  var status = document.getElementById('status');
  var frames = 0;
  window.tpt.on('viewport.frame', function (frame) {
    try {
      if (canvas.width !== frame.width || canvas.height !== frame.height) {
        canvas.width = frame.width;
        canvas.height = frame.height;
      }
      var binary = atob(frame.data);
      var pixels = new Uint8ClampedArray(binary.length);
      for (var i = 0; i < binary.length; i++) pixels[i] = binary.charCodeAt(i);
      ctx.putImageData(new ImageData(pixels, frame.width, frame.height), 0, 0);
      frames += 1;
      status.textContent = 'frames: ' + frames;
    } catch (err) {
      status.textContent = 'error: ' + err;
    }
  });
})();
"#;

/// A lit cube seen from above and to the side. Returns the scene, its camera and the cube.
fn build_scene() -> (Scene, NodeId, NodeId) {
    let mut scene = Scene::new().with_background(Color::hex("#101820"));
    let cube = scene.add_node(Node::cube().named("cube"));
    let cam = scene.add_node(
        Node::camera(PerspectiveCamera::default())
            .named("main")
            .with_transform(
                Transform::from_position(Vec3::new(0.0, 1.5, 5.0)).looking_at(Vec3::ZERO),
            ),
    );
    scene
        .set_active_camera(Some(cam))
        .expect("the viewport camera is a camera node");
    scene.add_node(Node::light(DirectionalLight::new(
        Vec3::new(-0.3, -1.0, -0.2),
        2.0,
    )));
    (scene, cam, cube)
}

fn main() -> tpt_shell::Result<()> {
    let loader = std::env::args()
        .nth(1)
        .ok_or(tpt_shell::Error::Unsupported {
            feature: "running without a WebView2Loader.dll path",
        })?;
    let running = App::new()
        .name("scene-shell")
        .loader(loader)
        .asset("index.html", "text/html", PAGE)
        .asset("scene.css", "text/css", STYLE)
        .asset("scene.js", "text/javascript", SCRIPT)
        .window(
            Window::new("main")
                .title("tpt-nova scene")
                .size(640.0, 480.0)
                .page("index.html"),
        )
        .build()?;

    let ui = running.ui_handle();
    thread::spawn(move || {
        let (scene, cam, cube) = build_scene();
        let delegate = match AppfrontScene::new(scene, cam) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("scene-shell: {e}");
                return;
            }
        };
        let mut viewport = Viewport3D::new(
            delegate,
            ViewportSize {
                width: WIDTH as f32,
                height: HEIGHT as f32,
            },
        );
        for step in 0u32.. {
            // Turn the cube a little each frame.
            let angle = step as f32 * 0.04;
            if let Some(node) = viewport.delegate_mut().scene_mut().get_mut(cube) {
                node.transform.rotation = Quat::from_rotation_y(angle);
            }
            if viewport.frame(1.0 / 30.0) == PresentStatus::Unsupported {
                eprintln!("scene-shell: no GPU adapter, so the scene cannot be drawn");
                break;
            }
            if let Some(px) = viewport.pixels() {
                if let Err(err) = ui.viewport_frame("main", px.width, px.height, px.rgba) {
                    eprintln!("scene-shell: stopped sending frames: {err}");
                    break;
                }
            }
            thread::sleep(Duration::from_millis(33));
        }
    });

    running.run()
}
