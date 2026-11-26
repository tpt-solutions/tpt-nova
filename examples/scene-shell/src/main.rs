//! A tpt-nova 3D scene in a tpt-shell window. The scene is rendered on the GPU through
//! `AppfrontScene` and an appfront `Viewport3D`. Each frame's pixels are sent to the page,
//! which draws them on a canvas. Windows only.
//!
//! usage: cargo run --release -- <path to WebView2Loader.dll>

use std::thread;
use std::time::Duration;

use glam::{Quat, Vec3};
use tpt_appfront_canvas_3d::{PresentStatus, Viewport3D, ViewportSize};
use tpt_appfront_core::ui_tree::UITree;
use tpt_nova_scene_graph::appfront::AppfrontScene;
use tpt_nova_scene_graph::{
    Color, DirectionalLight, Node, NodeId, PerspectiveCamera, Scene, Transform,
};
use tpt_shell::{App, Window};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;

// The page's script and CSS are separate assets: the shell's policy blocks inline code.
/// The UI, described as an appfront tree: a heading, the viewport the scene is drawn into,
/// and a caption. appfront renders it to HTML, and the viewport becomes a canvas.
fn ui() -> UITree<()> {
    UITree::container(|c| {
        c.heading(1, "tpt-nova scene");
        c.viewport("scene", "A lit cube, turning");
        c.text("Rendered on the GPU by tpt-nova and drawn into this page by tpt-shell.");
    })
}

/// The page: appfront's HTML for the UI, inside a head that links the shell's own assets.
/// The shell's policy blocks inline script and style, so the head only links them.
fn page() -> String {
    let body = tpt_appfront_html::render(&ui());
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>tpt-nova scene</title>
<link rel="stylesheet" href="scene.css">
</head>
<body>
{body}
<script src="scene.js"></script>
</body>
</html>
"#
    )
}

const STYLE: &str = r#"html, body { margin: 0; background: #101820; color: #ddd; font: 14px sans-serif; }
h1 { margin: 12px 12px 4px; font-size: 22px; }
p { margin: 4px 12px; }
canvas { display: block; margin: 8px 12px; image-rendering: pixelated; }
#status { position: fixed; right: 8px; bottom: 8px; padding: 4px 8px; color: #fff;
  background: rgba(0, 0, 0, 0.6); }
"#;

const SCRIPT: &str = r#"(function () {
  var status = document.createElement('div');
  status.id = 'status';
  status.textContent = 'starting';
  document.body.appendChild(status);
  var frames = 0;
  // Frames name their viewport; a frame without a name goes to the first viewport canvas.
  function canvasFor(name) {
    if (name) return document.querySelector('canvas[data-viewport="' + name + '"]');
    return document.querySelector('canvas[data-viewport]');
  }
  window.tpt.on('viewport.frame', function (frame) {
    var canvas = canvasFor(frame.name);
    if (!canvas) return; // not on the page yet; the next frame finds it
    try {
      if (canvas.width !== frame.width || canvas.height !== frame.height) {
        canvas.width = frame.width;
        canvas.height = frame.height;
      }
      var binary = atob(frame.data);
      var pixels = new Uint8ClampedArray(binary.length);
      for (var i = 0; i < binary.length; i++) pixels[i] = binary.charCodeAt(i);
      canvas.getContext('2d').putImageData(new ImageData(pixels, frame.width, frame.height), 0, 0);
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
        .asset("index.html", "text/html", page())
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
                if let Err(err) = ui.viewport_frame_named("main", "scene", px.width, px.height, px.rgba) {
                    eprintln!("scene-shell: stopped sending frames: {err}");
                    break;
                }
            }
            thread::sleep(Duration::from_millis(33));
        }
    });

    running.run()
}
