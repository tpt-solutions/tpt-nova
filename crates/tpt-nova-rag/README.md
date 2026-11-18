# tpt-nova-rag

> Local vector-DB + RAG retrieval for TPT Nova: index project assets/docs and answer context queries

Local vector database and retrieval-augmented generation over project assets and docs, with a deterministic feature-hashing embedder by default and an optional local neural embedder.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Features

| Feature | Enables |
|---------|---------|
| `real-embeddings` | Swaps in a real sentence-embedding model (fastembed's local All-MiniLM-L6-v2) behind `RealEmbedder`. Enabling this downloads model weights on first use (network required once); inference itself is fully local with no API calls. |

## Workspace dependencies

- `tpt-nova-ecs`

## Testing

```sh
cargo test -p tpt-nova-rag
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
