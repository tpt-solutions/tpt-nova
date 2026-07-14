# TPT Nova — External AI Agent Protocol

This document is a **developer reference** for driving the TPT Nova engine from an
external AI agent. It covers three coordinated surfaces:

1. the **control-file protocol** (agent → engine: hot-applied mutations),
2. the **telemetry protocol** (engine → agent: observable world snapshots),
3. and the **agent command surface** plus the **Highlight & Fix** overlay and
   RAG context wiring.

All field names, limits, and defaults below were cross-checked against the
source (`crates/nova-app`, `crates/nova-agent-api`, `crates/nova-telemetry`,
`crates/nova-overlay`, `crates/nova-rag`). No Rust code was modified to produce
this document.

The loop in one sentence: **the agent reads `nova-telemetry.json`, decides on a
change, writes a control file (`nova-control.json` by default), and the engine
hot-applies it on the next poll — no engine restart required.**

---

## 1. Control-File Protocol (agent → engine)

An external process drives the engine purely by **writing a JSON file**. The
engine polls that file each tick and re-applies it only when its modification
time changes, so the loop is fully decoupled from the engine's lifecycle.

### 1.1 Polling / hot-apply mechanism

Both the editor shell (`nova-app`) and the reusable `ControlChannel`
(`nova-agent-api`) implement the same watch semantics:

- Each tick the engine reads the file's `metadata.modified()` timestamp
  (milliseconds since the Unix epoch).
- If the mtime equals the last-seen mtime, **nothing is applied** — the loop is
  **idempotent between writes**. A control file written once stays applied
  (e.g. a rotation stays set) and is *not* re-touched or reset on subsequent
  ticks.
- When the mtime changes, the engine (re-)reads and parses the file and applies
  the contained commands exactly once for that version.
- Writing a **new** version (new mtime) replaces/extends the prior state — e.g.
  a second file with a different rotation overrides the first.

> Implementation note: `nova-app::apply_control` (crates/nova-app/src/main.rs)
> tracks `control_mtime: Option<u64>`; `nova-agent-api::ControlChannel` tracks
> the same idea plus an `applied_count` accumulator.

### 1.2 Two control schemas

There are **two** control-file shapes in the tree. They are independent.

#### (a) The editor shell's simple control file — default `nova-control.json`

`nova-app` reads the default path `nova-control.json`
(`crates/nova-app/src/main.rs` → `CONTROL_PATH`). Its schema is intentionally
tiny — it rotates the world's cube entity:

```json
{
  "set_rotation": { "x": 0.3, "y": 0.6, "z": 0.9 }
}
```

- `set_rotation` is **optional** (`#[serde(default)]`). When present, the engine
  sets the cube's local rotation via `Quat::from_euler(EulerRot::XYZ, x, y, z)`
  (radians).
- `RotationXYZ` fields `x`, `y`, `z` are `f32` and default to `0.0`.
- A file with no `set_rotation` (or an unparseable file) is a no-op — bad JSON
  is logged at `warn` level and skipped; the previous state is preserved.
- This is the schema exercised by the editor shell and its regression test
  `ai_control_loop_hot_applies_rotation_and_is_idempotent`.

#### (b) The formal protocol — `nova-agent-api::ControlFile`

`crates/nova-agent-api` formalizes the loop into a **versioned command batch**.
Authoritative struct:

```rust
pub struct ControlFile {
    pub protocol: u32,                 // must equal PROTOCOL_VERSION (1)
    pub commands: Vec<AgentCommand>,   // applied in order, stop at first error
}
```

On-disk JSON:

```json
{
  "protocol": 1,
  "commands": [
    { "op": "spawn", "name": "player", "translation": [5.0, 0.0, 0.0], "mesh": "cube" },
    { "op": "set_rotation", "target": { "id": "e0#0" }, "rotation_euler_xyz": [0.1, 0.2, 0.3] },
    { "op": "set_transform", "target": { "name": "player" },
      "translation": [1.0, 2.0, 3.0], "rotation_euler_xyz": [0.0, 1.57, 0.0], "scale": [1.0, 1.0, 1.0] },
    { "op": "despawn", "target": { "id": "e3#0" } }
  ]
}
```

- `protocol` must equal `PROTOCOL_VERSION` (currently `1`). A mismatch is
  rejected with `AgentApiError::ProtocolMismatch`, so an old agent fails loudly
  instead of silently mis-driving a newer engine.
- `commands` are applied **in order** via `apply_commands`/`apply_command`;
  application stops at the first error (the whole batch is transactional per
  poll — no partial apply within a poll).

### 1.3 Caps & limits (enforced by `ControlChannel::poll`)

These are deliberate anti-abuse bounds, because a control file is written by a
potentially untrusted external process:

| Limit | Constant | Value | Effect when exceeded |
| --- | --- | --- | --- |
| Max file size | `MAX_CONTROL_FILE_BYTES` | **1 MiB** (`1 << 20`) | Rejected before reading (`io::Error` InvalidData) |
| Max commands per poll | `MAX_COMMANDS_PER_POLL` | **10_000** | Rejected with `AgentApiError::ControlParse` |
| Protocol mismatch | `PROTOCOL_VERSION` | **1** | Rejected with `AgentApiError::ProtocolMismatch` |

A missing file yields `Ok(0)` commands applied (no error). An unparseable file
yields `AgentApiError::ControlParse`.

### 1.4 `AgentCommand` reference

`AgentCommand` is the **closed** set of mutations an agent may request — it can
reposition and spawn things, but never touch engine internals or run arbitrary
code. It is `#[serde(tag = "op", rename_all = "snake_case")]`:

| `op` | Fields | Effect |
| --- | --- | --- |
| `spawn` | `name: Option<String>`, `translation: [f32;3]`, `mesh: Option<String>` | New entity with a `Transform` at `translation`; if `mesh` is `"cube"` (or any string) a `Mesh { kind: Cube }` is added. If `name` is set, it is registered in the `EntityRegistry` world resource for later reference. |
| `despawn` | `target: EntityRef` | Removes the entity from the world. Errors if the target is unknown/dead. |
| `set_transform` | `target: EntityRef`, `translation: Option<[f32;3]>`, `rotation_euler_xyz: Option<[f32;3]>`, `scale: Option<[f32;3]>` | Sets whichever of translation/rotation(Euler XYZ, radians)/scale are provided. |
| `set_rotation` | `target: EntityRef`, `rotation_euler_xyz: [f32;3]` | Sets only the local rotation (Euler XYZ, radians). |

`EntityRef` is the union used to name a target:

```rust
pub enum EntityRef {
    Id(String),    // a telemetry id, e.g. "e3#0"
    Name(String),  // a stable name registered on spawn
}
```

Serialized as `{"id": "e3#0"}` or `{"name": "player"}`. The id string format is
`e<index>#<generation>` (parsed by `EntityRef::from_id_string`; malformed ids
yield `AgentApiError::MalformedEntityId`). `resolve` checks the entity is still
alive; unknown references yield `AgentApiError::UnknownEntity`.

> Note the distinction from the editor shell's `set_rotation` above: the
> formal command rotates an arbitrary entity by `EntityRef`, whereas the
> editor's `nova-control.json` rotates only the cube and uses a bare
> `RotationXYZ`.

---

## 2. Telemetry Protocol (engine → agent)

The engine emits a machine-readable snapshot of the whole ECS world so an agent
can observe and self-correct. In the editor shell the default file is
`nova-telemetry.json` and it is written **every 30 ticks**
(`crates/nova-app/src/main.rs` → `TELEMETRY_INTERVAL = 30`, ≈ every 0.5 s at the
60 Hz fixed timestep).

### 2.1 Frame shape

The frame is `nova_telemetry::TelemetryFrame`:

```rust
pub struct TelemetryFrame {
    pub schema_version: u32,        // currently 1
    pub tick: u64,
    pub seed: u64,                  // the world RNG seed (NOVA_SEED, default 0x1234_ABCD)
    pub entities: Vec<EntityDump>,
}

pub struct EntityDump {
    pub id: String,                          // e.g. "e0#0"
    pub components: HashMap<String, Value>, // component name -> serde_json::Value
}
```

`dump_world` iterates every live entity and serializes the components it finds:
`Transform`, `GlobalTransform`, `Mesh`, `Camera`, `Parent`, `Children`.

### 2.2 JSON example (one frame)

```json
{
  "schema_version": 1,
  "tick": 30,
  "seed": 305441741,
  "entities": [
    {
      "id": "e0#0",
      "components": {
        "Transform": {
          "translation": [0.0, 0.0, 0.0],
          "rotation": [0.0, 0.0, 0.0, 1.0],
          "scale": [1.0, 1.0, 1.0]
        },
        "GlobalTransform": {
          "matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
        },
        "Mesh": { "kind": "Cube" }
      }
    },
    {
      "id": "e1#0",
      "components": {
        "Transform": {
          "translation": [0.0, 0.0, 3.5],
          "rotation": [0.0, 0.0, 0.0, 1.0],
          "scale": [1.0, 1.0, 1.0]
        },
        "GlobalTransform": { "matrix": [ /* 16 floats */ ] },
        "Camera": { "fov_y": 1.0, "near": 0.1, "far": 1000.0, "aspect": 1.0 }
      }
    }
  ]
}
```

Component value shapes (exact, from the serializer helpers):

- `Transform`: `{ "translation": [x,y,z], "rotation": [x,y,z,w], "scale": [x,y,z] }`
- `GlobalTransform`: `{ "matrix": [m00..m33] }` — the 4×4 column-major matrix as
  16 floats.
- `Mesh`: `{ "kind": "Cube" }` (the `MeshKind` debug string).
- `Camera`: `{ "fov_y", "near", "far", "aspect" }` (all `f32`).
- `Parent`: `{ "parent": "<entity id>" }`; `Children`: `{ "children": ["<id>", ...] }`.

The file is **pretty-printed JSON, truncated-and-overwritten each emit**, with a
trailing newline.

### 2.3 Sinks

`nova-telemetry` ships three `TelemetrySink`s:

- `FileSink` (default in `nova-app`; `nova-telemetry.json`) — pretty JSON.
- `MsgPackFileSink` — compact **MessagePack** blob (named fields, so it
  round-trips through the same serde types as JSON). Use for high-frequency /
  large payloads. Decode with `rmp_serde` or `encode_msgpack` + `from_slice`.
- `StdoutSink` — pretty JSON to stdout.

`TelemetryEmitter<S>` owns a sink and an interval (ticks). `maybe_emit(world,
tick, seed)` emits only when `tick` is a multiple of the interval **and** isn't a
repeat of the last emitted tick (no double-emit). The interval is clamped to at
least 1. An agent reads the same frame back via
`nova_agent_api::read_telemetry_file(path)`.

---

## 3. Agent Command Surface & RAG Context

### 3.1 `ControlChannel` (the reusable hot-apply loop)

`nova_agent_api::ControlChannel` is the host-side equivalent of the editor's
file watch. Construct it with a path, then call `poll(&mut world)` each tick:

```rust
use nova_agent_api::{ControlChannel, ControlFile, AgentCommand, EntityRef};

let mut ch = ControlChannel::new("nova-control.json");
let applied = ch.poll(&mut world)?;   // 0 if unchanged/absent, else #commands
let total = ch.applied_count();        // cumulative across lifetime
```

It enforces the 1 MiB size cap, the `MAX_COMMANDS_PER_POLL` (10_000) cap, and
the protocol-version check described in §1.3. You can also apply commands
directly without a file via `apply_command(&mut world, &cmd)` /
`apply_commands(&mut world, &[cmd])`.

### 3.2 `RagAssistant` — RAG-backed context

Behind the **`rag` feature** (`nova-agent-api`'s `rag` feature pulls in
`nova-rag`), `nova_agent_api::rag::RagAssistant` gives an agent project context
to ground its fixes. It wraps `nova_rag::RagAgent` (default top-k = 4):

```rust
use nova_agent_api::rag::RagAssistant;

// Index a project tree (hidden / build dirs skipped; empty `extensions` = all):
let assistant = RagAssistant::index_project(".", &["rs", "md", "toml"])?;

// Or build from an already-loaded nova_rag::Index:
let assistant = RagAssistant::from_index(index);

let ctx = assistant.context_for("how do I move the highlighted cube?")?; // prompt-ready block
let hits = assistant.retrieve("physics collider rapier")?;               // Vec<ScoredHit>
```

- `RagAgent` / `Index` / `Document` / `ScoredHit` / `SearchError` come from
  `nova-rag`. Embeddings are pluggable via the `Embedder` trait; the offline
  default is `FeatureHashEmbedder` (deterministic, CI-friendly), with a real
  local neural embedder behind `nova-rag`'s `real-embeddings` feature.
- `context_for(query)` returns exactly the string an agent would paste above its
  instruction before emitting `AgentCommand`s — it includes the retrieved text
  plus `score=` lines so the agent can cite sources.
- `retrieve(query)` returns `Vec<ScoredHit>` (document + cosine-similarity
  score) for explicit citations.

---

## 4. Highlight & Fix Overlay Flow

`crates/nova-overlay` turns a viewport marquee into a structured fix request an
agent can act on. The renderer draws the rectangle; the crate owns the logic
(region math, entity picking, prompt assembly) — fully testable without a GPU.

### 4.1 Flow

1. **Marquee drag** — the host feeds raw pointer events to a `SelectionTool`
   (viewport pixel coords):
   - `SelectionTool::begin(x, y)` on press,
   - `SelectionTool::drag(x, y)` on move,
   - `SelectionTool::current_rect(size)` returns the live `ScreenRect` to draw
     each frame.
2. **Release → request** — `SelectionTool::build_request(&world, view_proj,
   size, instruction)` calls `end()` to get the normalized marquee, then
   `build_fix_request`, which:
   - rejects **empty (zero-area) regions** (`OverlayError::EmptyRegion`),
   - projects each entity's `GlobalTransform` center to screen with
     `project_to_screen` (returns `None` for points behind the camera, `w <= 0`),
   - collects the entities whose centers fall inside the region
     (`pick_entities_in_region`),
   - assembles an `AiFixRequest`.
3. **The `AiFixRequest`** (what the agent consumes):

   ```rust
   pub struct AiFixRequest {
       pub region: [f32; 4],       // normalized x, y, w, h in [0,1]
       pub entity_ids: Vec<String>,// telemetry ids of entities in the region
       pub instruction: String,     // the natural-language instruction
       pub prompt: String,          // ready-to-send combined prompt
   }
   ```

   `build_prompt` renders:

   ```
   FIX REQUEST
   Region (normalized x,y,w,h): [0.450, 0.433, 0.112, 0.150]
   Entities in region: e0#0
   Instruction: move the highlighted cube up
   Use the engine control API to resolve this.
   ```

4. **Loop closed** — the agent reads `entity_ids` (which match telemetry ids),
   grounds the instruction with `RagAssistant::context_for`, then applies
   `AgentCommand`s (e.g. `SetTransform`) through a `ControlChannel`. The engine
   hot-applies them, telemetry reflects the new state, and the agent verifies.

### 4.2 Key types

- `ScreenRect { x0, y0, x1, y1 }` (pixels, top-left origin, y-down) —
  `normalized(size)` → `[x, y, w, h]` in `[0,1]`; `contains(x, y)`;
  `new(a, b)` normalizes drag direction.
- `project_to_screen(world_point, view_proj, size) -> Option<(u32,u32)>` —
  inverse of the renderer's view-projection; `None` when clipped.
- `pick_entities_in_region(world, view_proj, size, region) -> Vec<Entity>`.
- `build_fix_request(world, view_proj, size, region, instruction) ->
  Result<AiFixRequest, OverlayError>`.
- `SelectionTool` — `is_dragging`, `begin`, `drag`, `end`, `current_rect`,
  `last_request`, `build_request`.

---

## 5. Reference Implementation: the Agent Fix Loop

The end-to-end loop (highlight → RAG → command → applied) is demonstrated by the
**`agent_fix_loop` example** in `nova-agent-api`:

```sh
cargo run -p nova-agent-api --example agent_fix_loop --features rag
```

It runs headless (no GPU): it drags a `SelectionTool` marquee around a cube,
builds the `AiFixRequest`, retrieves RAG context for the instruction, then emits
a `SetTransform` `AgentCommand` through a `ControlChannel` and asserts the
highlighted entity actually moved. This is the canonical reference for wiring
all three surfaces together.

> Discrepancy note: the task brief referred to this as *"`cargo run -p
> nova-sample-game` `agent_fix_loop`"*. In the current tree, `nova-sample-game`
> is a separate **headless pipeline demo** (`cargo run -p nova-sample-game`,
> `run_pipeline()` — physics rest, scene save/reload, agent spawn/move,
> splat→collider, asset pack) and does **not** contain `agent_fix_loop`. The
> flagship agent-fix-loop demo lives in `nova-agent-api` (see exact command
> above). Worth reconciling in docs/todo.

---

## Quick reference

| Item | Value / type | Source |
| --- | --- | --- |
| Default control path | `nova-control.json` | nova-app `CONTROL_PATH` |
| Default telemetry path | `nova-telemetry.json` | nova-app `TELEMETRY_INTERVAL`, `default_telemetry_path()` |
| Telemetry interval | every **30 ticks** (~0.5 s @ 60 Hz) | nova-app `TELEMETRY_INTERVAL` |
| Telemetry `schema_version` | `1` | nova-telemetry `dump_world` |
| Protocol version | `PROTOCOL_VERSION = 1` | nova-agent-api |
| Control file size cap | **1 MiB** (`MAX_CONTROL_FILE_BYTES`) | nova-agent-api |
| Commands per poll cap | **10_000** (`MAX_COMMANDS_PER_POLL`) | nova-agent-api |
| Command set | `spawn`, `despawn`, `set_transform`, `set_rotation` | nova-agent-api `AgentCommand` |
| Entity ref | `{"id":"e#g"}` or `{"name":"..."}` | nova-agent-api `EntityRef` |
| Rotation convention | Euler XYZ, **radians** | nova-agent-api / nova-app `EulerRot::XYZ` |
| Idempotency | mtime-watched; applied once per write | nova-app `apply_control`, `ControlChannel::poll` |

### Minimal working example (formal protocol)

```json
{
  "protocol": 1,
  "commands": [
    { "op": "spawn", "name": "cube", "translation": [0, 0, 0], "mesh": "cube" },
    { "op": "set_rotation", "target": { "name": "cube" }, "rotation_euler_xyz": [0, 0.5, 0] }
  ]
}
```

Write it as `nova-control.json` (or any path your `ControlChannel` watches), and
the engine will apply it on the next poll while continuing to emit telemetry the
agent can read back.
