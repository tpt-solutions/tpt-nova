//! Flagship "agent fix loop" demo: RAG context + control API + highlight/fix.
//!
//! This ties together the three agent-facing crates end to end (logic only, no
//! GPU):
//!
//! 1. `nova-overlay` — a "Highlight & Fix" marquee selects a region and builds
//!    a structured [`AiFixRequest`] (region, the entities inside it, and an
//!    instruction string).
//! 2. `nova-rag` (via `nova-agent-api::rag::RagAssistant`) — retrieves project
//!    context for that instruction so an agent can ground its fix.
//! 3. `nova-agent-api` — turns the decision into a typed [`AgentCommand`] and
//!    applies it through the same [`ControlChannel`] an external agent would
//!    use, proving the loop closes (read highlight → retrieve context → emit
//!    command → engine applies it).
//!
//! Run with: `cargo run -p nova-agent-api --example agent_fix_loop --features rag`

use nova_agent_api::rag::RagAssistant;
use nova_agent_api::{apply_command, AgentCommand, ControlChannel, ControlFile, EntityRef};
use nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind, Transform};
use nova_ecs::{Mat4, Vec3, World};
use nova_overlay::{build_fix_request, SelectionTool};

/// A camera at (0,0,5) looking down -Z, used to project the highlighted entity.
fn camera_view_proj() -> (World, Mat4, (u32, u32)) {
    let mut world = World::new();
    let cam = world.spawn();
    world.add_component(cam, Transform::from_translation(Vec3::new(0.0, 0.0, 5.0)));
    world.add_component(cam, Camera::default());
    world.add_component(cam, GlobalTransform::identity());
    nova_ecs::scene_graph::propagate_transforms(&mut world);

    let cam_t = Transform::from_translation(Vec3::new(0.0, 0.0, 5.0));
    let view = cam_t.matrix().inverse();
    let proj = Camera {
        aspect: 1.0,
        ..Default::default()
    };
    let vp = proj.perspective() * view;
    (world, vp, (800, 600))
}

fn main() {
    // --- 1. Highlight & Fix: drag a marquee around the cube -----------------
    let (mut world, vp, size) = camera_view_proj();
    let e = world.spawn();
    world.add_component(e, Transform::from_translation(Vec3::ZERO));
    world.add_component(e, Mesh { kind: MeshKind::Cube });
    world.add_component(e, GlobalTransform::identity());

    let mut tool = SelectionTool::new();
    tool.begin(360, 260);
    tool.drag(440, 340);
    let req = tool
        .build_request(&world, vp, size, "move the highlighted cube up")
        .expect("marquee selects the cube");
    println!("== Highlight & Fix request ==");
    println!("{}", req.prompt);
    assert_eq!(req.entity_ids, vec![format!("{e}")]);

    // --- 2. RAG: ground the fix instruction in project context ------------
    let mut idx = nova_rag::Index::default_new();
    idx.add_documents([
        nova_rag::Document::new(
            "transform.md",
            "set_transform moves an entity by translation rotation_euler_xyz and scale",
        ),
        nova_rag::Document::new(
            "agent_api.md",
            "AgentCommand SetTransform targets an entity by id or stable name",
        ),
    ]);
    let assistant = RagAssistant::from_index(idx);
    let context = assistant
        .context_for("how do I move the highlighted cube?")
        .expect("retrieve context");
    println!("\n== RAG context for the fix ==");
    println!("{context}");

    // --- 3. Agent API: emit + apply the fix through the control channel ----
    // The agent decides: lift the highlighted entity by +2 on Y. The command is
    // written to a control file and applied via the same ControlChannel an
    // external agent drives the engine with — closing the loop.
    let cmd = AgentCommand::SetTransform {
        target: EntityRef::Id(format!("{e}")),
        translation: Some([0.0, 2.0, 0.0]),
        rotation_euler_xyz: None,
        scale: None,
    };
    let control_path = std::env::temp_dir().join("nova_agent_fix_loop.json");
    let _ = std::fs::remove_file(&control_path);
    std::fs::write(
        &control_path,
        ControlFile::new(vec![cmd]).to_json().unwrap(),
    )
    .unwrap();

    let mut channel = ControlChannel::new(&control_path);
    let applied = channel.poll(&mut world).expect("apply control file");
    println!("\n== Control channel applied {} command(s) ==", applied);
    assert_eq!(applied, 1);

    // Verify the engine actually moved the highlighted cube.
    let t = world.get_component::<Transform>(e).unwrap();
    println!("highlighted entity transform after fix: {t:?}");
    assert!(
        (t.translation.y - 2.0).abs() < 1e-3,
        "agent fix loop must move the highlighted entity"
    );

    let _ = std::fs::remove_file(&control_path);
    println!("\nAgent fix loop closed end-to-end: highlight -> RAG -> command -> applied.");
}
