# TPT Nova — External Agent Protocol Reference

This document is the authoritative, code-independent reference for the
machine-to-machine control loop an external AI agent uses to drive the TPT
Nova engine. It is intentionally independent of reading the Rust source in
[`nova-agent-api`](../crates/nova-agent-api); the Rust types
(`PROTOCOL_VERSION`, `AgentCommand`, `ControlFile`, `ControlChannel`) are the
canonical implementation and this text describes their wire format.

The loop has two halves:

1. **Observe** — the engine continuously writes a *telemetry* snapshot of the
   world (entities + components) to a file/socket. The agent reads it.
2. **Act** — the agent writes a *control file* describing the mutations it
   wants; the engine hot-applies it (no restart) the next time it polls.

The agent's power is deliberately bounded to a closed set of commands
(spawn / despawn / transform). It can never read or write arbitrary engine
memory or execute code.

---

## 1. Control file (agent → engine)

The agent writes a JSON file. The engine polls it (by modification time) and
applies any **new** version exactly once.

```jsonc
{
  "protocol": 1,                 // REQUIRED, must equal the engine's PROTOCOL_VERSION
  "commands": [                  // ordered list, applied in sequence
    { "op": "spawn", "name": "hero", "translation": [0, 0, 0], "mesh": "cube" },
    { "op": "set_transform", "target": "hero", "translation": [2, 0, 0], "scale": [2, 2, 2] },
    { "op": "set_rotation", "target": { "id": "e3#0" }, "rotation_euler_xyz": [0, 0.5, 0] },
    { "op": "despawn", "target": { "id": "e7#1" } }
  ]
}
```

### Top-level fields

| Field       | Type            | Notes                                                        |
|-------------|-----------------|--------------------------------------------------------------|
| `protocol`  | integer (`u32`) | Must equal the engine's supported `PROTOCOL_VERSION` (1).    |
| `commands`  | array           | Ordered mutations. May be empty.                             |

If `protocol` does not match, the whole file is **rejected** (the agent is
told the supported version). If `commands` exceeds `MAX_COMMANDS_PER_POLL`
(10,000), the file is rejected to bound per-poll work and prevent a hostile
file from flooding the world with spawns. The control file is also size-capped
at **1 MiB**; larger files are refused.

### Entity references

A command `target` is one of:

```jsonc
{ "id": "e3#0" }     // telemetry id: "e" + index + "#" + generation
{ "name": "hero" }   // stable name registered when the entity was spawned
```

`id` strings must match `e<index>#<generation>`. `name` resolves through the
engine's `EntityRegistry` resource (populated by `spawn` commands that carry a
`name`). An unresolvable reference is an error and stops batch application at
that command.

### Commands (`op`)

All commands are tagged unions with `op` (snake_case):

| `op`              | Fields                                                                 | Effect                                                              |
|-------------------|------------------------------------------------------------------------|---------------------------------------------------------------------|
| `spawn`           | `name?`, `translation: [f32;3]`, `mesh?` ("cube" or omitted)           | Create an entity with a `Transform` (+ `Mesh` if `mesh` given). If `name` is set, register it for later `name` references. |
| `despawn`         | `target: EntityRef`                                                    | Remove the entity from the world.                                   |
| `set_transform`   | `target`, `translation?`, `rotation_euler_xyz?`, `scale?`              | Overwrite the supplied `Transform` sub-fields (Euler XYZ radians). |
| `set_rotation`    | `target`, `rotation_euler_xyz: [f32;3]`                               | Set only the rotation (Euler XYZ radians).                         |

`rotation_euler_xyz` values are in **radians**. Unknown `mesh` strings fall
back to a cube.

---

## 2. Telemetry snapshot (engine → agent)

The engine emits a JSON frame (`nova-telemetry::TelemetryFrame`) on an interval
(default every 30 ticks). Schema:

```jsonc
{
  "schema_version": 1,
  "tick": 30,
  "seed": 305441749,
  "entities": [
    {
      "id": "e1#0",
      "components": {
        "Transform": { "translation": [0, 0, 0], "rotation": [0, 0, 0, 1], "scale": [1, 1, 1] },
        "Mesh": { "kind": "Cube" },
        "GlobalTransform": { /* 4x4 matrix, row-major */ }
      }
    }
  ]
}
```

| Field            | Type    | Notes                                                          |
|------------------|---------|----------------------------------------------------------------|
| `schema_version` | integer | Currently `1`.                                                  |
| `tick`           | integer | Simulation tick the snapshot was taken on.                     |
| `seed`           | integer | The world RNG seed (deterministic runs share a seed).          |
| `entities`       | array   | One entry per live entity.                                     |

Each entity entry has:

| Field        | Type   | Notes                                                          |
|--------------|--------|----------------------------------------------------------------|
| `id`         | string | Telemetry id, `"e<index>#<generation>"` — use it as `target`. |
| `components` | object | Component name → JSON value. Possible keys: `Transform`, `GlobalTransform`, `Mesh`, `Camera`, `Parent`, `Children`, plus any gameplay components present. |

The agent reads this to discover entity ids, then issues `target: { "id": ... }`
commands. A MessagePack variant (`MsgPackFileSink`) of the same frame exists for
high-frequency payloads; the JSON frame remains the human/agent-readable
default.

---

## 3. Polling contract (`ControlChannel`)

Both the shipped `nova-app` and any host using `ControlChannel` follow the same
rules:

- The agent **writes the whole control file** (not a delta) to the agreed path.
- The engine tracks the file's **mtime**; it re-applies only when the mtime
  changes. Rewriting the same content without an mtime change is a no-op
  (idempotent between writes).
- One poll applies at most `MAX_COMMANDS_PER_POLL` commands, in order, stopping
  at the first error.
- Resolution: `id` references are validated against live entities; `name`
  references resolve via the `EntityRegistry`.

---

## 4. Minimal agent session

```text
# 1. Read the current world.
frame = read_telemetry_file("nova-telemetry.json")
hero = frame.entities[0].id            # e.g. "e1#0"

# 2. Write a control file asking the engine to move it.
control = ControlFile::new(vec![
    AgentCommand::SetTransform {
        target: EntityRef::Id(hero),
        translation: Some([2.0, 0.0, 0.0]),
        rotation_euler_xyz: None,
        scale: None,
    },
])
write_text("nova-control.json", control.to_json())

# 3. Wait one poll; re-read telemetry — the entity is now at x=2.
```

See [`nova-agent-api/examples/agent_fix_loop.rs`](../crates/nova-agent-api/examples/agent_fix_loop.rs)
for a full RAG-backed "Highlight & Fix" demo that combines this control loop
with `nova-overlay`'s region picking and `nova-rag` context retrieval.
