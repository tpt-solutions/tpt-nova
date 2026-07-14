# TPT Nova — Public Alpha Release Checklist

This checklist tracks the gates that must be green before tagging `v0.1.0-alpha`.
Items marked **[AUTO]** are enforced by CI; **[MANUAL]** require a human sign-off.

## Build & Test
- [AUTO] `cargo build --workspace` succeeds on Windows, Linux, and macOS
      (CI matrix in `.github/workflows`).
- [AUTO] `cargo test --workspace` is green, including the new crate suites:
  `nova-splat`, `nova-rag`, `nova-export`, `nova-agent-api`, `nova-overlay`,
  `nova-videocap`, and `nova-sample-game`.
- [AUTO] `cargo clippy --workspace --all-targets` reports no errors
      (`-D warnings` in CI).
- [AUTO] Coverage reporting (e.g. `cargo tarpaulin` / `grcov`) runs in CI and a
      baseline is tracked (see `todo.md` Testing Coverage section).

## Documentation
- [MANUAL] `README.md` accurately lists every workspace crate and the pipeline.
- [MANUAL] `CHANGELOG.md` is updated for the release (Unreleased → versioned).
- [MANUAL] Public API surfaces have module-level docs:
  - `nova-agent-api` (external AI control contract + protocol version),
  - `nova-rag` (embedding trait + retrieval),
  - `nova-export` (pack format + platform targets),
  - `nova-splat` / `nova-videocap` (ingestion + collision proxies),
  - `nova-overlay` (Highlight & Fix).

## Licensing & Provenance
- [MANUAL] `LICENSE` (Apache-2.0) present at repo root; every crate's
      `Cargo.toml` carries `license.workspace = true`.
- [MANUAL] Third-party asset licenses reviewed (any sample meshes/models/splats
      shipped in the demo are redistributable).
- [MANUAL] `Cargo.lock` committed for reproducible Alpha builds.

## Packaging & Distribution
- [MANUAL] `nova-export` bundles a standalone build per target
      (`bundle` for `win`/`linux`/`macos`) and the assets into a `.novapack`.
- [MANUAL] The `nova-sample-game` demo runs from its packaged output
      (`nova-sample-game` binary + `assets.novapack` + `manifest.json`).

## Sample Game (proof point)
- [MANUAL] `nova-sample-game::run_pipeline()` passes end-to-end (physics rest,
      scene save/reload, agent spawn/move, splat→collider, asset pack).
- [MANUAL] A short recorded clip / screenshot of the sample running in the
      editor exists for the release announcement.

## Known Limitations (disclose in release notes)
- [MANUAL] Gaussian Splat rendering in `nova-splat` is billboard-approximation
      behind the `render` feature (full anisotropic covariance shader is future
      work).
- [MANUAL] `nova-rag` ships a model-free feature-hash embedder; swapping in a
      neural embedder is a drop-in behind the `Embedder` trait.
- [MANUAL] Networking/multiplayer is explicitly out of scope for Alpha
      (see `todo.md` Open Decisions).

## Sign-off
- [MANUAL] Maintainer review of the above → tag `v0.1.0-alpha` → publish notes.
