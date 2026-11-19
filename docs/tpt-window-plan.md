# tpt-window: plan for a permissive windowing and input crate

Status: plan. The repository does not exist yet. Name, licence, and layout
below are proposals.

## Why

`winit` (Apache-2.0 only) provides our window, event loop, and keyboard and
mouse input. It is the largest source of Apache-only exceptions in
`deny.toml`. The TPT media collection has no windowing layer of its own (see
"What the media collection covers" below), so tpt-window has to be written.

Goal: a windowing crate with the same functionality we use today, under
MIT OR Apache-2.0, that tpt-nova can switch to without dropping features.

## Scope: parity with what we use

Measured from our code (`tpt-nova-app`, `tpt-nova-input`, `tpt-nova-render`,
`tpt-nova-render/src/pbr.rs`). Every item must work before winit is removed.

| Area | What we use from winit | Notes |
|---|---|---|
| Event loop | `EventLoop`, `ApplicationHandler`, `ActiveEventLoop`, `ControlFlow` | Callback model, same shape, so `tpt-nova-app` migrates with little change. |
| Window | `Window`, `WindowAttributes`, `WindowId`, `request_redraw`, `inner_size`, `scale_factor` | Create, resize, redraw request, DPI scale. |
| Events | `WindowEvent::{CloseRequested, RedrawRequested, Resized, KeyboardInput, ModifiersChanged, MouseInput, MouseWheel, CursorMoved}` | `RedrawRequested`, `Resized`, `CloseRequested`, keys, mouse buttons, scroll, cursor position. |
| Keys | `KeyCode`, `Key`, `NamedKey`, `PhysicalKey`, `ElementState`, `ModifiersState` | Physical (scan) and logical key codes. |
| Mouse | `MouseButton`, `MouseScrollDelta`, `PhysicalPosition` | Pixel and line scroll. |
| DPI | `PhysicalSize`, `LogicalSize`, `PhysicalPosition` | Our own types, no `dpi` crate. |
| GPU | `raw-window-handle` (`HasWindowHandle`, `HasDisplayHandle`) | Lets `wgpu` create a surface. |

Not used by us today, so not required for parity: IME, touch, drag and drop,
gamepads, monitor enumeration, fullscreen, cursor grab, and window icons. They
are listed under "Later" so they are not lost; the first release does not need
them.

## Backends

Each backend is a feature-gated module with the same public API.

1. **Windows (first).** Crate `windows` (MIT OR Apache-2.0), using
   `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_UI_HiDpi`.
   Win32 class registration, message pump, `WM_*` to `WindowEvent` mapping,
   DPI awareness per-monitor v2. We are on Windows, so this backend is the one
   we can test immediately.
2. **Linux, X11.** `x11rb` (MIT OR Apache-2.0). Window, `XInput` for keys
   and mouse, `RandR` for scale.
3. **Linux, Wayland.** `wayland-client` (MIT). Later; X11 covers current
   Linux use through XWayland.
4. **macOS.** `objc2` and `objc2-app-kit` (MIT). `NSApplication` run loop,
   `NSWindow` and `NSView`, key events via `NSEvent`.

## Keyboard mapping

Our `KeyCode` values come from the physical key (scan code). The mapping
tables are the most error-prone part, so they are generated and tested:

- Windows: scan code plus `MapVirtualKeyEx` for logical keys.
- X11: keycode to keysym via `XkbGetMap`, scan code via the physical keycode.
- macOS: `kVK_*` virtual key codes.

Each backend gets a table test: known scan code in, expected `KeyCode` out.

## Public API sketch

```rust
pub trait ApplicationHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop);
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent);
}
```

The names mirror winit on purpose. The migration in tpt-nova is then mostly an
import change, plus the handful of `Window` methods that differ.

## Repository layout (proposed)

```
tpt-window/
  Cargo.toml          workspace
  crates/
    tpt-window/       public API, types, ApplicationHandler, key codes
    tpt-window-win32/ Windows backend
    tpt-window-x11/   X11 backend
    tpt-window-cocoa/ macOS backend
  deny.toml           same policy as tpt-nova: no Apache-2.0-only deps
  LICENSE-MIT, LICENSE-APACHE
```

Backends are separate crates so that a Linux build does not compile Win32
code. Start with `tpt-window` and `tpt-window-win32` only.

## Migration in tpt-nova

1. Add tpt-window as a git dependency pinned to a commit (same pattern as
   `tpt-av-cadence-flac`), and add it to `deny.toml`'s `allow-git`.
2. Put it behind a `tpt-window` feature in `tpt-nova-app`, `tpt-nova-input`,
   and `tpt-nova-render`. Both backends build side by side.
3. Run the sample scene and the input tests on both backends, compare
   behaviour, then make tpt-window the default.
4. Remove `winit` and its exceptions (`winit`, `dpi`, `ab_glyph`,
   `ab_glyph_rasterizer`, `owned_ttf_parser`, `gethostname`) from `deny.toml`.

## Other consumers

`tpt-visual` (TPT Solutions media stack) also depends on `winit` directly,
for its example and compositor. Once tpt-window is stable, the same migration
applies there.

`tpt-av-ui` gets its window from `eframe`, which uses winit internally. Replacing
that needs an `egui` backend on tpt-window, which is out of scope for this plan.

## What the media collection covers

Checked against the ten repositories in the TPT AV stack (`tpt-kinetix`,
`tpt-cadence`, `tpt-visual`, `tpt-audio`, `tpt-voice`, `tpt-av-control`,
`tpt-av-asset`, `tpt-av-ui`, `tpt-av-sync`, `tpt-av-test`):

- **Windowing: not covered.** No native window code exists in any of them.
  `tpt-visual` and `tpt-av-ui` both depend on winit (directly or through
  eframe). tpt-window is new work.
- **Audio output: partly covered.** `tpt-audio/tpt-av-audio-io` has a real
  WASAPI backend (`windows` crate, MIT OR Apache-2.0), which can replace
  `cpal` on Windows. It also has a PipeWire backend that shells out to `pw-cat`
  and a CoreAudio backend that is still a stub. See the audio section of
  `docs/license-replacement-plan.md`.
- **GPU: not a replacement.** `tpt-visual` uses `wgpu` 30, the same version we use.
  It is a useful reference for rendering, not a substitute.
- **FLAC: covered** (done, `tpt-av-cadence-flac`).

## Risks

- Windows IME and keyboard layout edge cases. Mitigation: parity tests on the
  keys we use; IME is out of scope for parity.
- Wayland is more involved than X11. Mitigation: XWayland first.
- macOS has no test machine in this setup. Mitigation: CI on macOS runners,
  as already in the matrix.
- Effort: the Windows backend is the largest single piece. The other backends
  come after it, one at a time.

## Milestones

1. **M1.** tpt-window public API and Windows backend. The sample app opens a
   window, renders with wgpu, and reports key and mouse events.
2. **M2.** Input parity tests on Windows. Feature flag in tpt-nova; both
   backends run in CI.
3. **M3.** X11 backend, same tests.
4. **M4.** macOS backend.
5. **M5.** Remove winit from tpt-nova and `deny.toml`. Migrate tpt-visual.
6. **Later.** Wayland, IME, touch, gamepads, monitors, fullscreen, icons.
