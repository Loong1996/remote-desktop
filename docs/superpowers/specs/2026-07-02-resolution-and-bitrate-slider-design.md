# Resolution Hot-Switch + Bitrate Slider — Design

Date: 2026-07-02
Status: approved (user: 三档可选默认高清 / 码率滑杆 / 热切换短暂停顿)

## Problem

The sharpness bottleneck is capture resolution, not bitrate: the agent captures the
display scaled into 1280×720 (≈1106×720) and the browser upscales it, so text is
permanently soft and bitrate presets show no visible difference on static content
(verified live: encoder quality saturates below 3 Mbps on a static desktop; release
build encodes even 3024×1964 at ~127 fps ceiling, so higher resolutions are cheap).
Users also want a manual bitrate control beyond the 5 presets.

## Scope

1. Resolution presets, hot-switchable in-session: `sd` / `hd` / `native`, default `hd`.
2. Manual bitrate slider (0.25–20 Mbps) alongside the existing preset dropdown.
3. Out of scope: fps control, per-window capture, multi-display, changing the
   20 Mbps bitrate clamp, Windows/Linux capture parity (they keep current behavior).

## Protocol (@rd/protocol + agent/src/control.rs)

New control message, both directions unchanged otherwise:

```ts
{ t: "resolution"; preset: "sd" | "hd" | "native" }
```

- TS: extend `ControlMessage` union + `parseControlMessage` (reject unknown presets).
- Rust: `ControlMessage::Resolution { preset: ResolutionPreset }` with
  `#[serde(rename_all = "lowercase")] enum ResolutionPreset { Sd, Hd, Native }`.
- Quality message (`bitrateBps`) unchanged; slider reuses it.

## Agent

### Preset → capture size mapping (`video/mod.rs`)

Two layers, so the mapping is unit-testable without a display:

```rust
pub fn preset_size(preset: ResolutionPreset, dw: u32, dh: u32) -> (u32, u32) // pure
pub fn preset_capture_size(preset: ResolutionPreset) -> (u32, u32) // queries display, calls preset_size
```

Given the main display's logical size `(dw, dh)` (from `main_display_size()`):
- `Sd` → `fit_aspect(dw, dh, 1280, 720)` (current behavior)
- `Hd` → `(even(dw), even(dh))` — display logical points (new session default)
- `Native` → `(even(dw * 2), even(dh * 2))` — Retina physical pixels; on non-Retina
  displays this is a 2× upscale by SCK's scaler (accepted; harmless)

If the display query fails, fall back to `fit_aspect`-style behavior with 1280×720
(same as today's fallback).

### Pipeline command channel (`video/pipeline.rs`)

Replace `bitrate_rx: Receiver<u32>` with `cmd_rx: Receiver<PipelineCmd>`:

```rust
pub enum PipelineCmd { Bitrate(u32), Resolution(u32, u32) }
```

`VideoPipeline::start` gains a source factory so the pipeline thread can rebuild
its capturer:

```rust
source_factory: Box<dyn Fn(u32, u32) -> Box<dyn ScreenCapturer> + Send>
```

(fps stays a fixed pipeline parameter). On `Resolution(w, h)`, in the pipeline
thread between frames:
1. Drop the old capturer (stops SCK stream) and old frame channel (discards stale
   frames at the old size).
2. Create a fresh frame channel; build + start a new capturer via the factory.
3. Rebuild the encoder at (w, h) with the **current** bitrate (track last-applied
   bitrate in the loop; encoder is rebuilt explicitly — do not rely on openh264
   adapting to changed frame dimensions), force IDR on the next frame.
4. Update `dst_w`/`dst_h` locals used by `bgra_to_i420`.

If the new capturer fails to start (e.g. permission revoked), log a warning and
attempt to restore a capturer at the previous size; if that also fails, the video
freezes but the session stays up (input/clipboard unaffected).

`Bitrate(bps)` keeps today's behavior (encoder rebuild via `set_bitrate`).

### wire_control (`webrtc_peer.rs`)

- `Resolution { preset }` → `preset_capture_size(preset)` → send
  `PipelineCmd::Resolution(w, h)`.
- Initial capture size changes from `target_capture_size(1280, 720)` to
  `preset_capture_size(Hd)`.

## Web (`packages/web`)

### Resolution dropdown (SessionView)

`data-testid="resolution-select"`, options 流畅 720p (`sd`) / 高清 (`hd`, default) /
视网膜 (`native`). On change: `sendControl({ t: "resolution", preset })`. Enabled
only when connected. The stats HUD's resolution field is the user-visible
confirmation that the switch applied.

### Bitrate slider (SessionView)

`data-testid="bitrate-slider"`: `<input type="range" min={250000} max={20000000}
step={250000}>` with a live Mbps label. Dragging sends
`{ t: "quality", bitrateBps }` debounced at 200 ms. Two-way link with the preset
dropdown:
- choosing a preset sets the slider to that preset's bps;
- a slider value matching no preset shows the dropdown as 自定义 (a disabled-less
  extra option rendered only in that state).

## Testing

- Protocol: TS `parseControlMessage` accepts the three presets, rejects others;
  Rust serde round-trip for `Resolution`/`ResolutionPreset`.
- `preset_capture_size`: mapping + evenness (display size mockable? — pure helper
  takes `(dw, dh)` as args for testability; the SCK query stays at the call site).
- Pipeline: fake `ScreenCapturer` + factory recording created sizes; send
  `Resolution` cmd, assert old capturer dropped, new one started at the new size,
  encoder rebuilt (recording encoder) and IDR forced; `Bitrate` still applies.
- Web (RTL): resolution select sends the message; slider sends debounced quality
  message; preset↔slider linkage; 自定义 state.
- E2E (manual, this machine): connect, switch to 视网膜, HUD shows 3024×1964 and
  session/input stay alive; switch back to 720p; drag slider, kbps burst follows.

## Rollout

Frontend + agent both change; agent must be rebuilt (release) and restarted.
Old web against new agent: fine (agent default hd). New web against old agent:
resolution messages are unknown to the old parser and ignored — degrade gracefully.
