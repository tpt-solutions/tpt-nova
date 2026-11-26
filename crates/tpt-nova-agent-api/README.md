# tpt-nova-agent-api

> Stable external-AI-agent control API for the TPT Nova engine (telemetry read + command apply loop)

Stable, versioned control API for external AI agents: a closed `AgentCommand` set (including splat commands), a `ControlChannel` that reads agent-written control files under size and count caps, and telemetry read-back.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Features

| Feature | Enables |
|---------|---------|
| `rag` | Exposes `rag`: a RAG-backed retrieval assistant wired into the agent loop. |
| `llm` | Exposes `llm`: a real HTTP LLM client (reads NOVA_LLM_ENDPOINT / NOVA_LLM_KEY). |

## Workspace dependencies

- `tpt-nova-ecs`
- `tpt-nova-telemetry`
- `tpt-nova-rag`
- `tpt-nova-overlay`
- `tpt-nova-splat`

## Testing

```sh
cargo test -p tpt-nova-agent-api
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
