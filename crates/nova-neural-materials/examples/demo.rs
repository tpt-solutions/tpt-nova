//! Headless proof of the neural-material round-trip: prompt -> frames -> (GPU
//! upload is shown as a no-op when no device is present). Run with
//! `cargo run -p nova-neural-materials --example demo`.

use nova_neural_materials::prompt::{FeedSource, MaterialPrompt};
use nova_neural_materials::NeuralMaterialRegistry;

fn main() {
    let mut registry = NeuralMaterialRegistry::default();

    let prompt = MaterialPrompt::new(
        "billboard_01",
        "AI-generated live commercial, neon rain",
        FeedSource::VideoLlm {
            endpoint: "llm://demo".into(),
        },
    )
    .with_resolution(64, 64)
    .with_fps(30.0)
    .with_tag("cinematic");

    registry.register(prompt).expect("open feed");
    println!("registered material 'billboard_01'");

    // Pump a few ticks of decoded frames.
    for tick in 0..3 {
        registry.update();
        let frame = registry.latest("billboard_01").expect("frame");
        let first_texel = &frame.rgba[..4];
        println!(
            "tick {tick}: frame {}x{} ts={}ms first_texel_rgba={first_texel:?}",
            frame.width, frame.height, frame.timestamp_ms
        );
    }

    println!("round-trip OK: prompt produced streamed frames");
}
