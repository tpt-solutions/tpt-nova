# nova-sample-game — a forkable TPT Nova project template

`nova-sample-game` is more than a smoke test: it is the recommended **starting
point for your own TPT Nova game**. Fork this crate (or copy it into a new
workspace member) and replace `run_pipeline()` / `build_world()` with your own
systems — everything else (ECS, physics, scene save/load, the agent control
API, splat ingestion, and packaging) comes for free.

## What it demonstrates

`run_pipeline()` exercises the *entire* engine in one flow:

1. **Physics** — a dynamic player box falls onto a fixed ground (Rapier3D).
2. **Scene save/load** — the world is serialized to RON and reloaded.
3. **Agent control** — an external-style `AgentCommand` spawns an NPC and moves
   it through the stable `nova-agent-api` surface.
4. **Splat ingestion** — a Gaussian Splat point cloud is loaded and a convex-hull
   collision proxy is derived from it.
5. **Packaging** — the scene is bundled into a `.novapack` asset container and
   unpacked, proving the standalone distribution path.

Every step is also covered by `#[test]`s, so the same code runs headlessly in
CI and live in the demo binary.

## Using it as a template

```sh
# from the workspace root
cp -r crates/nova-sample-game crates/my-game
# edit my-game/Cargo.toml: rename the package, then build/run it
cargo run -p my-game
```

Wire your gameplay by adding systems to the `World`/`Scheduler`, spawning your
own entities, and emitting telemetry for external AI agents to observe. The
editor (`cargo run -p nova-app`) can drive the same world at runtime.

See the top-level [GETTING_STARTED.md](../GETTING_STARTED.md) and
[todo.md](../todo.md) for the broader picture.
