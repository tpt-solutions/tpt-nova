# crates.io release tracker

Tracks publishing the TPT Nova workspace crates to crates.io, in dependency order.
Update the checkboxes as each step completes. Publishing is irreversible: a
published version can be yanked but never deleted or overwritten, so do not skip
the dry-run and prerequisite steps.

Current workspace version: `0.1.0` (from `[workspace.package]`).

## 1. Scope

25 crates are publish candidates. `tpt-nova-xtask` sets `publish = false` and is
excluded.

Decisions to make before publishing (not yet decided):

- [ ] `tpt-nova-app` (winit shell binary) — publish as a library-style crate, or set `publish = false`?
- [ ] `tpt-nova-sample-game` (forkable project template) — publish, or set `publish = false`?
- [ ] `tpt-nova-gameplay-example` (player controller example) — publish, or keep as internal example?

Recommendation (revised after checking dependencies): `tpt-nova-sample-game` has no
blocking dependencies, so it can be published. `tpt-nova-app`, `tpt-nova-gameplay-example`
and several other crates are blocked by unpublished git dependencies (section 2). See
section 2 for the current blocker list before deciding.

## 2. Prerequisites (blockers, verified)

These were confirmed by `cargo publish --dry-run`. Nothing downstream of tier 0
can be published until they are fixed.

- [x] **Internal dependencies need a version.** Done: every internal path dependency
  (root `[workspace.dependencies]`, `app`, `splat`, `agent-api`) now carries `version = "0.1.0"`.
  Before the fix, `cargo publish -p tpt-nova-telemetry --dry-run` failed with
  `all dependencies must have a version requirement specified when publishing`.
- [x] **Dev-dependency cycle between `tpt-nova-telemetry` and `tpt-nova-physics`.** Done:
  `telemetry`'s dev-dep on `physics` is now path-only, matching `physics`'s dev-dep on
  `telemetry`. Path-only dev-deps are stripped on publish, so no publish-time cycle remains.
  `scripting` and `agent-api` dev-deps are path-only (`agent-api`'s `ingest` dev-dep is
  workspace-versioned and has no cycle).
- [x] **Optional dependency affects ordering.** `tpt-nova-render` has an optional
  dependency on `tpt-nova-neural-materials` (feature `neural`). Cargo counts it for
  publish order, so `render` is tier 2, not tier 1.
- [x] **Unused dependency.** Done: removed the unused `tpt-nova-ecs` dependency from
  `crates/tpt-nova-rag/Cargo.toml`. `cargo check --workspace --all-targets` passes.
- [ ] **Git dependencies block publishing (new, verified).** crates.io rejects any
  dependency declared with `git =`, even an optional one. Affected crates and their
  sources:
  - `tpt-nova-input`: `tpt-window-core` (git, tpt-window). **Not on crates.io.**
  - `tpt-nova-render`: `tpt-window` (git). **Not on crates.io.** Also in
    `tpt-nova-scene-graph` (normal and dev) and `tpt-nova-app`.
  - `tpt-nova-scene-graph`: `tpt-appfront-canvas-3d` (git, optional). **Not on crates.io.**
  - `tpt-nova-drivers`: `tpt-wasm`, `tpt-wasm-runtime`, `tpt-wasm-types`,
    `tpt-wasm-format` (git, optional). **Not on crates.io.**
  - `tpt-nova-audio`: `tpt-av-cadence-core` and `tpt-av-cadence-flac` (git, pinned to
    rev `48b8e4b`). **Both exist on crates.io at 0.1.0**, but the pinned git revision may
    differ from the published API. Switching requires a build check.
  - `tpt-nova-app`: `tpt-window` (git).
  - Not published, so irrelevant to this release: `examples/scene-shell` (excluded from the workspace).

  Options (decision needed, see section 1):
  1. Publish the `tpt-solutions` `tpt-window`, `tpt-wasm` and `tpt-appfront` crates first,
     from their own repos, then switch the git deps to versions.
  2. Set `publish = false` on the blocked crates (`input`, `render`, `scene-graph`,
     `drivers`, `app`) and remove them from the tier table. Their dependents
     (`gameplay-example`, which needs `input`) are then also blocked.
  3. Feature-gate the git-backed code out of the published crates. This is a code change,
     not a manifest change.
- [ ] **Audio pin vs published cadence.** Decide whether `tpt-nova-audio` should switch
  from the git pin to `version = "0.1.0"` on crates.io. The git pin was intentional
  for reproducible builds; `deny.toml` allows the git source for it.
- [ ] **crates.io name check.** Confirm all 25 names are free (or owned by us) on crates.io.
- [ ] **Publish token.** Run `cargo login` yourself in a terminal. Do not store the
  token in this repo or in this file.
- [ ] **Release commit is green.** `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
  all pass on the commit being published.
- [ ] **Changelogs and version are final.** Per-crate `CHANGELOG.md` entries exist
  for `0.1.0`; the root `CHANGELOG.md` matches.
- [ ] **Packaging is clean.** `cargo package -p <crate> --list` shows no stray
  files (no `target/`, no local assets that should not ship). Check `include`/`exclude`
  in each crate's manifest if `assets/` or large binaries are present.
- [ ] **Repository URL.** `[workspace.package] repository` points at
  `https://github.com/tpt-nova/tpt-nova`. Confirm that is the correct public org/repo.

## 3. Publish order

Computed from the normal-dependency graph (`cargo metadata`), including optional
dependencies. A crate may only be published after every crate in a lower tier
is live on crates.io. Within a tier, order does not matter.

| Tier | Crate | Internal deps (normal) | Done |
|------|-------|------------------------|------|
| 0 | `tpt-nova-ecs` | — | [ ] |
| 0 | `tpt-nova-input` | — (**blocked**: git dep `tpt-window-core`) | [ ] |
| 0 | `tpt-nova-ui` | — | [ ] |
| 0 | `tpt-nova-audio` | — (**blocked**: git-pinned cadence crates; see section 2) | [ ] |
| 0 | `tpt-nova-export` | — | [ ] |
| 1 | `tpt-nova-telemetry` | ecs | [ ] |
| 1 | `tpt-nova-anim` | ecs | [ ] |
| 1 | `tpt-nova-drivers` | ecs | [ ] |
| 1 | `tpt-nova-neural-materials` | ecs | [ ] |
| 1 | `tpt-nova-overlay` | ecs | [ ] |
| 1 | `tpt-nova-physics` | ecs | [ ] |
| 1 | `tpt-nova-rag` | ecs | [ ] |
| 1 | `tpt-nova-scripting` | ecs | [ ] |
| 1 | `tpt-nova-scripting-embedded` | ecs | [ ] |
| 2 | `tpt-nova-render` | ecs, ui, neural-materials (optional) (**blocked**: git dep `tpt-window`) | [ ] |
| 2 | `tpt-nova-ingest` | ecs, anim | [ ] |
| 2 | `tpt-nova-scene` | ecs, physics | [ ] |
| 2 | `tpt-nova-editor` | ecs, physics, ui | [ ] |
| 2 | `tpt-nova-gameplay-example` | ecs, input, physics, scripting (**blocked** via `input`) | [ ] |
| 3 | `tpt-nova-scene-graph` | ecs, render, anim (**blocked**: git deps `tpt-window`, `tpt-appfront-canvas-3d`) | [ ] |
| 3 | `tpt-nova-splat` | ecs, ingest, overlay, telemetry | [ ] |
| 3 | `tpt-nova-videocap` | ecs, ingest | [ ] |
| 4 | `tpt-nova-agent-api` | ecs, telemetry, rag, overlay, splat | [ ] |
| 4 | `tpt-nova-app` | ecs, input, render, telemetry, ui, editor, overlay, neural-materials, scene, splat (**blocked**: git dep `tpt-window`) | [ ] |
| 5 | `tpt-nova-sample-game` | ecs, ingest, scene, agent-api, splat, export | [ ] |

Notes on the table:

- `tpt-nova-drivers` is blocked by four optional git deps (`tpt-wasm*`) and is not in
  the table above because it has no internal dependents; add it once unblocked.
- `tpt-nova-scene-graph` and `tpt-nova-drivers` are workspace members but are not
  listed in `CLAUDE.md`/`README.md`'s crate table. Confirm they are intended for
  release before publishing them.
- Tier 0 and tier 1 are the only tiers that can be dry-run before the earlier tiers
  are live. Later tiers can only be fully verified by publishing in order.

## 4. Per-crate procedure

Repeat for each crate, in table order:

1. `cargo publish -p <crate> --dry-run` — must succeed. For tier ≥1 this requires
   the lower tiers to already be on crates.io; a failure here before then is expected
   only if it is the "no matching package" error for a dependency that is not yet published.
2. `cargo publish -p <crate>`.
3. Wait until the new version is visible in the index: `cargo info <crate>@0.1.0`
   (or retry the next dry-run). Do not start the next tier until every crate in the
   current tier shows up.
4. Tick the checkbox above.

If a publish fails partway through a tier, stop. Fix the cause, then continue from
the first unpublished crate. Never publish a crate whose dependency is not yet live.

## 5. After the release

- [ ] Tag the release commit: `git tag v0.1.0` and push the tag (confirm with the user first).
- [ ] Confirm each crate's docs build on docs.rs.
- [ ] Update `README.md` install instructions to show the crates.io versions.
- [ ] Record the published versions and date in `CHANGELOG.md`.

## 6. Rollback

- A bad version can be yanked (`cargo yank --version 0.1.0 -p <crate>`), which blocks
  new dependents but does not remove it. A fixed release needs a new version (`0.1.1`).
- Yanking does not change the publish order for the replacement version.
