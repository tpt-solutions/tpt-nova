# tpt-nova-rag

> Local vector-DB + RAG retrieval for TPT Nova: index project assets/docs and answer context queries

Local vector database and retrieval-augmented generation over project assets and docs, with a deterministic feature-hashing embedder by default and an optional local neural embedder.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Features

| Feature | Enables |
|---------|---------|
| `real-embeddings` | Adds `RealEmbedder`, which sends text to a local embeddings server (Ollama or `llama-server --embedding`) over plain `http://` using the OpenAI-compatible `/v1/embeddings` route. No model weights, inference runtime or TLS stack are linked into the crate. Only `http://` is accepted. |

## Workspace dependencies

- `tpt-nova-ecs`

## Testing

```sh
cargo test -p tpt-nova-rag
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
