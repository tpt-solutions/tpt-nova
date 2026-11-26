# Replacing the Apache-2.0-only dependencies

Status: plan. Nothing in this document has been implemented yet, except where a
step is marked **done**.

Goal: the dependency tree contains no Apache-2.0-only crates (a crate that is
"MIT OR Apache-2.0" is fine). `deny.toml` enforces this. Its `[licenses.exceptions]`
list is the work queue for this plan: each entry should disappear when its
replacement lands.

## Current exceptions

| Crate | Pulled in by | Used for |
|---|---|---|
| `winit` (0.30) | `tpt-nova-app`, `tpt-nova-input`, `tpt-nova-render` | Windows, event loop, input |
| `dpi` | `winit` | DPI types |
| `ab_glyph`, `ab_glyph_rasterizer`, `owned_ttf_parser` | `winit` (`sctk-adwaita`, Linux decorations) | none of ours |
| `gethostname` | `winit` (`x11rb`, Linux) | none of ours |
| `cpal` | `rodio` → `tpt-nova-audio` | audio output device |
| `codespan-reporting` | `naga` → `wgpu-core` (non-optional) | shader diagnostics |
| `spirv` | `naga` `spv-out` → `wgpu` `vulkan` | SPIR-V constants |
| `gl_generator`, `khronos_api`, `glutin_wgl_sys` | `wgpu-hal` `gles` (default feature on Windows) | GL backend bindings |
| `smartstring` (MPL-2.0) | `rhai` → `tpt-nova-scripting-embedded` | string optimisation |

`winit` accounts for most of the list (6 of 13 entries once its transitive
crates are counted). Dropping it also removes `dpi`, `ab_glyph*`,
`owned_ttf_parser`, and `gethostname`.

## Quick wins (no replacement needed)

1. **Drop wgpu's GL backend.** Remove the `gles` feature (`wgpu`
   `default-features = false`, then re-enable `std`, `parking_lot`, `dx12`,
   `metal`, `vulkan`, `wgsl`, `webgpu`). This removes `gl_generator`,
   `khronos_api`, and `glutin_wgl_sys` (3 entries). Cost: no OpenGL fallback
   on machines without Vulkan or DX12. This is a product decision; confirm
   before doing it.
2. **Remove or defer `smartstring`.** It comes from `rhai`'s default features.
   Check whether `rhai` can drop it through its feature flags. If not, see
   option B below.

## Replacements

### A. Windowing and input (`winit`, `dpi`, and its transitive crates)

Decision: **write tpt-window**, our own permissive windowing crate, as a new
repository. See `docs/tpt-window-plan.md` for scope, backends, and milestones.
We will not drop any functionality: parity with what we use today is the
first milestone.

Checked the TPT media collection first. It has no native windowing code, so
it does not replace `winit`. `tpt-visual` and `tpt-av-ui` also depend on winit
(directly or via eframe), so they will migrate to tpt-window later.

SDL3 (Zlib licence) remains a fallback if tpt-window's Windows backend stalls
in M1. It is not the plan of record.

### B. Audio output (`cpal`)

Decision: **use the WASAPI backend from `tpt-av-audio-io`** (TPT media
collection, `tpt-audio` repo, MIT OR Apache-2.0, `windows` crate underneath)
for Windows playback. Keep rodio for decoding only, since rodio's decoders
build without its `playback` feature. Checked: with `default-features = false`
and `lewton` + `minimp3`, rodio pulls in neither `cpal` nor `alsa`.

What this does **not** cover, so it is still open:

- Linux: the media repo's PipeWire backend shells out to `pw-cat` and
  `pw-dump`. That is not a real replacement. A native backend is needed:
  the `alsa` crate (Apache-2.0/MIT, so usable) or `pipewire` (MIT).
- macOS: the CoreAudio backend is a stub. An AUHAL implementation is needed,
  using `objc2-audio-toolbox` (MIT).

Playback migration: `tpt-nova-audio` currently drives rodio's device sink
(`DeviceSinkBuilder`, `Player`, `SpatialPlayer`). Those move to our own
`Mixer` (already device-independent), pulled by `tpt-av-audio-io`'s
`OutputCallback`. Spatial and bus behaviour stays in our `Mixer`, so its tests
continue to apply.

Own option for Linux and macOS, if the media backends are not adopted: write
the platform backends directly (ALSA, CoreAudio) against the same
`OutputCallback` shape.

### C. Shader diagnostics (`codespan-reporting`)

Blocker: `naga` depends on it unconditionally. No feature flag turns it off.

Options, in order of preference:

1. Upstream: ask `gfx-rs/wgpu` to make `codespan-reporting` optional behind
   a `diagnostics` feature. Smallest change, but depends on upstream.
2. Local `[patch.crates-io]` replacement: write `tpt-codespan-reporting`
   (MIT) implementing the subset of the API `naga` uses (`files`,
   `diagnostic`, `term` emit). Verify the subset with `cargo check` on the
   patched tree. This is a few hundred lines, and only the API surface naga
   touches matters.

### D. SPIR-V constants (`spirv`)

Pulled in only by `naga`'s `spv-out`, which `wgpu`'s `vulkan` feature enables.
It is a generated constants crate. The Khronos grammar and headers are MIT.

Own option: generate an MIT-licensed `tpt-spirv` crate from the Khronos
`spirv.core.grammar.json` (MIT) with the same module layout that `naga`
imports, then `[patch]` it in. The generator is a short script.

Alternative: drop Vulkan on Windows (DX12 covers it) and keep it only on
Linux. That still requires the crate on Linux, so it reduces the problem
without removing it.

### E. Scripting string optimisation (`smartstring`, MPL-2.0)

Check `rhai`'s features first. If it cannot be disabled, either accept it as
a documented MPL exception (MPL is file-scoped copyleft and does not infect
the rest of the tree) or replace `rhai`. Replacing a scripting engine is
large; do not start that until it is needed.

## Order of work

1. Quick win 1: confirm the GL-backend decision, then drop `gles`.
2. Windowing: tpt-window (see `docs/tpt-window-plan.md`), Windows backend first.
   Audio: tpt-av-audio-io WASAPI for Windows, then native Linux and macOS
   backends. Together these remove the 7 windowing and audio exceptions.
3. Shader crates: file the upstream request (C), and generate `tpt-spirv` (D)
   in parallel. Both are independent of windowing.
4. `smartstring`: check `rhai` features, then decide (E).
5. After each step, remove the matching exception from `deny.toml` and run
   `cargo deny check licenses` (default features).

### F. Neural embeddings (`real-embeddings`, done)

Status: **done.** `fastembed` (and with it `ort`, `ort-sys`, `tokenizers`,
`hf-hub`, `ureq`, `ring`, `webpki-roots`, `esaxx-rs`, `spm_precompiled`) is
gone. `RealEmbedder` calls a local embeddings server over plain `http://`
(OpenAI-compatible `/v1/embeddings`) using `std::net` and `serde_json`, so it
adds no crates.

Why not an in-process ML runtime: every one that was checked pulls Apache-only
crates. `candle-core` needs `safetensors` and `tokenizers`; `candle-transformers`
adds `esaxx-rs` and `spm_precompiled`; `tract-onnx` needs `prost` 0.11 and
`safetensors` 0.6. `tpt-crypto` was checked as a replacement for `ring`. It is
not a drop-in: it has no rustls `CryptoProvider` and no RSA, and its README says
it is unaudited. Nothing in the real-embeddings path needs TLS any more, so
`ring` no longer needs a replacement.

### G. HTTPS client (`llm` feature, open)

`tpt-nova-agent-api`'s `llm` feature uses `reqwest`, which brings `openssl`
(Apache-2.0 only, via `native-tls`), `sync_wrapper` (Apache-2.0 only) and `ryu`
(Apache-2.0 OR BSL-1.0). `--all-features` fails on these three. Options: keep
the cloud client behind a documented exception, or move the local-LLM path to
plain HTTP like `RealEmbedder`, which covers local servers but not hosted APIs.

## Verification for each step

- `cargo test --workspace` on Windows, Linux, and macOS (CI matrix).
- `cargo deny check licenses bans sources` with the exception removed.
- `cargo tree -i <crate>` shows nothing for the removed crate.
- Manual check: `tpt-nova-app` opens, renders the sample scene, and responds to
  keyboard and mouse. Audio plays a WAV, an OGG, and a FLAC file.
