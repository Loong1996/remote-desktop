# Plan 3 — Mouse/Keyboard Injection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture mouse/keyboard on the web control end and inject them on the Rust agent (被控端) over the existing WebRTC data channel, replacing the Plan 2b echo.

**Architecture:** Web serializes DOM events into `InputEvent` JSON (relative 0..1 coords) and sends them over the data channel (renamed `echo`→`input`). The agent parses each frame into a Rust `InputEvent` serde mirror and hands it to an `InputInjector` that owns an `enigo::Enigo` on a dedicated OS thread, injecting serially. No video yet: the web side uses a placeholder "remote screen" surface plus an on-screen event log. Includes a pre-offer ICE-candidate buffer (BACKLOG item A) as insurance before real input traffic.

**Tech Stack:** Rust (`enigo` 0.6, `macos-accessibility-client` 0.0.2 macOS-only, existing `webrtc` 0.11 / `tokio` / `serde`); TypeScript/React (native DOM events, existing `@rd/protocol`, Vitest).

## Global Constraints

- Toolchains: Node ≥ 20; Rust via rustup. `cargo` lives in `~/.cargo/bin` — prefix every cargo command with `export PATH="$HOME/.cargo/bin:$PATH"`.
- Data channel name is `"input"` (both ends change together). Agent's `on_data_channel` is label-agnostic; only the web `createDataChannel` label changes.
- The wire contract is `@rd/protocol` (`packages/protocol/src/input.ts`): variant tag field `t` ∈ `mmove|mdown|mup|wheel|kdown|kup`; fields `x,y` (∈[0,1]), `button` ∈ `left|right|middle`, `dx,dy`, `code` (`KeyboardEvent.code`). Rust serde must match this JSON byte-for-byte.
- New deps: `enigo = "0.6"`; `macos-accessibility-client = "0.0.2"` under `[target.'cfg(target_os = "macos")'.dependencies]`.
- TDD: write the failing test first, watch it fail, implement minimal, watch it pass, commit. Frequent commits. Commit messages in English ending with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Do not break green baselines: Node `npm test` (54) and `npm run typecheck` stay clean; web `npm run build` stays clean. Agent `cargo test` count changes as echo tests become input tests — keep it green at every task boundary.
- Fire-and-forget: the agent sends no ack back over the channel.

## File Structure

```
agent/
  Cargo.toml                 # Modify: add enigo + (macos) macos-accessibility-client deps
  src/
    lib.rs                   # Modify: add `pub mod input;` and `pub mod permission;`
    input.rs                 # Create: InputEvent serde mirror + pure mappers + InputInjector
    permission.rs            # Create: check_input_permission()
    webrtc_peer.rs           # Modify: IceBuffer, wire_input (replaces wire_echo), injector ownership, new_with_input_sink
    main.rs                  # Modify: call permission::check_input_permission() at startup
  tests/
    input_loopback.rs        # Create (replaces echo_loopback.rs): data channel frame → parsed InputEvent
    ice_trickle.rs           # Modify: channel "input", assert InputEvent received
    echo_loopback.rs         # Delete (superseded by input_loopback.rs)
packages/web/src/
  rtc.ts                     # Modify: sendInput, mouseCoords, mouseButtonName, channel "input", drop echo
  rtc.test.ts                # Modify: add encoding tests
  pages/SessionView.tsx      # Modify: placeholder capture surface + event log (replaces echo chat UI)
docs/superpowers/
  plan3-input-smoke.md       # Create: end-to-end manual smoke steps
```

## Parallel Groups

- **Group A (agent track):** Tasks 1–6, all under `agent/`. Dispatch **sequentially** — same directory, avoid git-index races.
- **Group B (web track):** Tasks 7–8, all under `packages/web/`. **Disjoint from Group A → dispatch concurrently with the agent track.** The only cross-track contract is the channel name `"input"` and the `InputEvent` JSON shape (both fixed above).
- **Task 9 (docs):** after A and B land.

Each task ends with an independently testable deliverable + its own commit. Controller commits each task separately.

---

## Task 1: Pre-offer ICE candidate buffer (BACKLOG item A)

**Files:**
- Modify: `agent/src/webrtc_peer.rs`
- Test: inline `#[cfg(test)]` in `agent/src/webrtc_peer.rs`

**Interfaces:**
- Produces: `IceBuffer<T>` with `new()`, `accept(&mut self, T) -> Option<T>`, `on_remote_set(&mut self) -> Vec<T>`. `PeerSession::add_remote_ice`/`accept_offer` behavior changes; public signatures unchanged.

- [ ] **Step 1: Write the failing unit test**

Add to the bottom of `agent/src/webrtc_peer.rs`, inside the existing `#[cfg(test)] mod tests` (create the module if absent):

```rust
#[cfg(test)]
mod ice_buffer_tests {
    use super::IceBuffer;

    #[test]
    fn buffers_until_remote_set_then_drains_in_order() {
        let mut b: IceBuffer<i32> = IceBuffer::new();
        assert_eq!(b.accept(1), None); // buffered
        assert_eq!(b.accept(2), None); // buffered
        assert_eq!(b.on_remote_set(), vec![1, 2]); // flushed in order
        assert_eq!(b.accept(3), Some(3)); // now passes through
        assert_eq!(b.on_remote_set(), Vec::<i32>::new()); // idempotent, nothing pending
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml ice_buffer`
Expected: FAIL to compile — `cannot find type IceBuffer`.

- [ ] **Step 3: Implement `IceBuffer`**

Add near the top of `agent/src/webrtc_peer.rs` (after imports):

```rust
/// Buffers remote ICE candidates that arrive before the remote description is
/// set. `accept` returns `Some(c)` when the candidate can be added immediately,
/// or `None` when it has been buffered. `on_remote_set` marks the remote
/// description as set and returns the buffered candidates to flush, in order.
pub(crate) struct IceBuffer<T> {
    remote_set: bool,
    pending: Vec<T>,
}

impl<T> IceBuffer<T> {
    pub(crate) fn new() -> Self {
        Self { remote_set: false, pending: Vec::new() }
    }
    pub(crate) fn accept(&mut self, candidate: T) -> Option<T> {
        if self.remote_set {
            Some(candidate)
        } else {
            self.pending.push(candidate);
            None
        }
    }
    pub(crate) fn on_remote_set(&mut self) -> Vec<T> {
        self.remote_set = true;
        std::mem::take(&mut self.pending)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml ice_buffer`
Expected: PASS (1 test).

- [ ] **Step 5: Wire `IceBuffer` into `PeerSession`**

In `agent/src/webrtc_peer.rs`, add `use std::sync::Mutex;` (std, not tokio — critical sections hold no `.await`). Add a field to the struct:

```rust
pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
    ice_buffer: Mutex<IceBuffer<RTCIceCandidateInit>>,
}
```

In `PeerSession::new`, initialize it where the struct is returned:

```rust
Ok(PeerSession { pc, ice_buffer: Mutex::new(IceBuffer::new()) })
```

Replace `add_remote_ice` with the buffering version:

```rust
pub async fn add_remote_ice(&self, candidate: serde_json::Value) -> Result<()> {
    let init: RTCIceCandidateInit = serde_json::from_value(candidate)?;
    let ready = { self.ice_buffer.lock().unwrap().accept(init) };
    if let Some(init) = ready {
        self.pc.add_ice_candidate(init).await?;
    }
    Ok(())
}
```

In `accept_offer`, immediately after `self.pc.set_remote_description(offer).await?;` add the flush (log-and-continue on a bad buffered candidate so a stray one can't fail the answer):

```rust
let flushed = { self.ice_buffer.lock().unwrap().on_remote_set() };
for init in flushed {
    if let Err(e) = self.pc.add_ice_candidate(init).await {
        tracing::warn!("failed to add buffered ice candidate: {e}");
    }
}
```

- [ ] **Step 6: Add an integration test that a pre-offer candidate no longer errors**

Append to `agent/tests/ice_trickle.rs`:

```rust
#[tokio::test]
async fn pre_offer_remote_candidate_is_buffered_not_dropped() {
    let (agent_ice_tx, _rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let agent = rd_agent::webrtc_peer::PeerSession::new(vec![], agent_ice_tx)
        .await
        .unwrap();
    // A candidate arriving before accept_offer must be accepted (buffered), not error.
    let candidate = serde_json::json!({
        "candidate": "candidate:1 1 udp 2130706431 127.0.0.1 54321 typ host",
        "sdpMid": "0",
        "sdpMLineIndex": 0
    });
    agent.add_remote_ice(candidate).await.unwrap();
    agent.close().await.unwrap();
}
```

- [ ] **Step 7: Run the agent test suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml`
Expected: PASS — previous 10 + new buffer unit test + new integration test all green.

- [ ] **Step 8: Commit**

```bash
git add agent/src/webrtc_peer.rs agent/tests/ice_trickle.rs
git commit -m "fix(agent): buffer pre-offer remote ICE candidates

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Rust `InputEvent` serde mirror

**Files:**
- Create: `agent/src/input.rs`
- Modify: `agent/src/lib.rs`
- Test: inline `#[cfg(test)]` in `agent/src/input.rs`

**Interfaces:**
- Produces: `pub enum MouseButton { Left, Right, Middle }`; `pub enum InputEvent { MMove{x:f64,y:f64}, MDown{button:MouseButton}, MUp{button:MouseButton}, Wheel{dx:f64,dy:f64}, KDown{code:String}, KUp{code:String} }`, both `Serialize + Deserialize + Clone + Debug + PartialEq`.

- [ ] **Step 1: Write the failing round-trip test**

Create `agent/src/input.rs` with only the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmove_matches_ts_wire_shape() {
        let ev = InputEvent::MMove { x: 0.5, y: 0.25 };
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"t":"mmove","x":0.5,"y":0.25}"#
        );
    }

    #[test]
    fn button_serializes_lowercase() {
        let ev = InputEvent::MDown { button: MouseButton::Left };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"t":"mdown","button":"left"}"#);
    }

    #[test]
    fn deserializes_all_variants_from_web_json() {
        let cases = [
            (r#"{"t":"mmove","x":0.1,"y":0.2}"#, InputEvent::MMove { x: 0.1, y: 0.2 }),
            (r#"{"t":"mdown","button":"right"}"#, InputEvent::MDown { button: MouseButton::Right }),
            (r#"{"t":"mup","button":"middle"}"#, InputEvent::MUp { button: MouseButton::Middle }),
            (r#"{"t":"wheel","dx":-3.0,"dy":10.0}"#, InputEvent::Wheel { dx: -3.0, dy: 10.0 }),
            (r#"{"t":"kdown","code":"KeyA"}"#, InputEvent::KDown { code: "KeyA".into() }),
            (r#"{"t":"kup","code":"Escape"}"#, InputEvent::KUp { code: "Escape".into() }),
        ];
        for (json, expected) in cases {
            assert_eq!(serde_json::from_str::<InputEvent>(json).unwrap(), expected);
        }
    }
}
```

- [ ] **Step 2: Add the module and run to verify it fails**

Add to `agent/src/lib.rs`:

```rust
pub mod input;
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib input::`
Expected: FAIL to compile — `InputEvent`/`MouseButton` not found.

- [ ] **Step 3: Implement the types**

Prepend to `agent/src/input.rs` (above the test module):

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Rust serde mirror of the TS `InputEvent` (`packages/protocol/src/input.ts`).
/// The JSON wire shape is the contract: tag field `t`, coords `x,y` ∈ [0,1].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum InputEvent {
    #[serde(rename = "mmove")]
    MMove { x: f64, y: f64 },
    #[serde(rename = "mdown")]
    MDown { button: MouseButton },
    #[serde(rename = "mup")]
    MUp { button: MouseButton },
    #[serde(rename = "wheel")]
    Wheel { dx: f64, dy: f64 },
    #[serde(rename = "kdown")]
    KDown { code: String },
    #[serde(rename = "kup")]
    KUp { code: String },
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib input::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/src/input.rs agent/src/lib.rs
git commit -m "feat(agent): add InputEvent serde mirror of @rd/protocol

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Pure input mappers (coords, wheel, button, keycode)

**Files:**
- Modify: `agent/Cargo.toml` (add `enigo`), `agent/src/input.rs`
- Test: inline `#[cfg(test)]` in `agent/src/input.rs`

**Interfaces:**
- Produces: `pub fn map_coord(x: f64, y: f64, w: i32, h: i32) -> (i32, i32)`; `pub fn pixels_to_clicks(delta: f64) -> i32`; `pub fn map_button(b: &MouseButton) -> enigo::Button`; `pub fn code_to_key(code: &str) -> Option<enigo::Key>`.

- [ ] **Step 1: Add the `enigo` dependency**

In `agent/Cargo.toml`, under `[dependencies]`, add:

```toml
enigo = "0.6"
```

- [ ] **Step 2: Write the failing mapper tests**

Add a second test module to `agent/src/input.rs`:

```rust
#[cfg(test)]
mod mapper_tests {
    use super::*;
    use enigo::{Button, Key};

    #[test]
    fn map_coord_scales_and_clamps() {
        assert_eq!(map_coord(0.0, 0.0, 1920, 1080), (0, 0));
        assert_eq!(map_coord(1.0, 1.0, 1920, 1080), (1919, 1079)); // clamp to w-1/h-1
        assert_eq!(map_coord(0.5, 0.5, 1920, 1080), (960, 540));
        assert_eq!(map_coord(1.5, -0.2, 1920, 1080), (1919, 0)); // out-of-range clamped
    }

    #[test]
    fn pixels_to_clicks_signs_and_minimum() {
        assert_eq!(pixels_to_clicks(0.0), 0);
        assert_eq!(pixels_to_clicks(100.0), 1);
        assert_eq!(pixels_to_clicks(-100.0), -1);
        assert_eq!(pixels_to_clicks(30.0), 1);   // small non-zero rounds to ±1, never 0
        assert_eq!(pixels_to_clicks(-30.0), -1);
        assert_eq!(pixels_to_clicks(250.0), 3);  // round(2.5)
    }

    #[test]
    fn map_button_covers_all() {
        assert!(matches!(map_button(&MouseButton::Left), Button::Left));
        assert!(matches!(map_button(&MouseButton::Right), Button::Right));
        assert!(matches!(map_button(&MouseButton::Middle), Button::Middle));
    }

    #[test]
    fn code_to_key_letters_digits_symbols() {
        assert!(matches!(code_to_key("KeyA"), Some(Key::Unicode('a'))));
        assert!(matches!(code_to_key("KeyZ"), Some(Key::Unicode('z'))));
        assert!(matches!(code_to_key("Digit7"), Some(Key::Unicode('7'))));
        assert!(matches!(code_to_key("Numpad3"), Some(Key::Unicode('3'))));
        assert!(matches!(code_to_key("Minus"), Some(Key::Unicode('-'))));
        assert!(matches!(code_to_key("Slash"), Some(Key::Unicode('/'))));
    }

    #[test]
    fn code_to_key_named_keys() {
        assert!(matches!(code_to_key("Enter"), Some(Key::Return)));
        assert!(matches!(code_to_key("Escape"), Some(Key::Escape)));
        assert!(matches!(code_to_key("Space"), Some(Key::Space)));
        assert!(matches!(code_to_key("ArrowUp"), Some(Key::UpArrow)));
        assert!(matches!(code_to_key("ArrowRight"), Some(Key::RightArrow)));
        assert!(matches!(code_to_key("F5"), Some(Key::F5)));
        assert!(matches!(code_to_key("F12"), Some(Key::F12)));
        assert!(matches!(code_to_key("ShiftLeft"), Some(Key::Shift)));
        assert!(matches!(code_to_key("ControlRight"), Some(Key::Control)));
        assert!(matches!(code_to_key("MetaLeft"), Some(Key::Meta)));
    }

    #[test]
    fn code_to_key_unknown_is_none() {
        assert!(code_to_key("MediaPlayPause").is_none());
        assert!(code_to_key("Fn").is_none());
        assert!(code_to_key("").is_none());
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib mapper_tests`
Expected: FAIL to compile — mapper functions and `enigo` not found (first build pulls enigo — slow).

- [ ] **Step 4: Implement the mappers**

Add to `agent/src/input.rs` (above the test modules), and add `use enigo::{Button, Key};` to the file's imports:

```rust
use enigo::{Button, Key};

/// Map relative coords (0..1) to absolute pixels on a `w`×`h` display,
/// clamping both the input range and the result to on-screen pixels.
pub fn map_coord(x: f64, y: f64, w: i32, h: i32) -> (i32, i32) {
    let cx = x.clamp(0.0, 1.0);
    let cy = y.clamp(0.0, 1.0);
    let px = (cx * w as f64).round() as i32;
    let py = (cy * h as f64).round() as i32;
    (px.clamp(0, (w - 1).max(0)), py.clamp(0, (h - 1).max(0)))
}

/// Convert a browser wheel delta (≈100px per notch, deltaMode 0) into enigo
/// scroll "clicks" (15° rotations). Any non-zero delta yields at least ±1.
pub fn pixels_to_clicks(delta: f64) -> i32 {
    if delta == 0.0 {
        return 0;
    }
    let clicks = (delta / 100.0).round() as i32;
    if clicks == 0 {
        if delta > 0.0 { 1 } else { -1 }
    } else {
        clicks
    }
}

pub fn map_button(b: &MouseButton) -> Button {
    match b {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

/// Map a `KeyboardEvent.code` string to an enigo `Key`. Returns `None` for
/// codes we don't handle (caller logs and skips). Letters map to lowercase
/// Unicode; case is produced by a separately-held Shift, matching real keyboards.
pub fn code_to_key(code: &str) -> Option<Key> {
    if let Some(rest) = code.strip_prefix("Key") {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() {
                return Some(Key::Unicode(c.to_ascii_lowercase()));
            }
        }
    }
    if let Some(rest) = code.strip_prefix("Digit").or_else(|| code.strip_prefix("Numpad")) {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_digit() {
                return Some(Key::Unicode(c));
            }
        }
    }
    if let Some(rest) = code.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u8>() {
            return match n {
                1 => Some(Key::F1),
                2 => Some(Key::F2),
                3 => Some(Key::F3),
                4 => Some(Key::F4),
                5 => Some(Key::F5),
                6 => Some(Key::F6),
                7 => Some(Key::F7),
                8 => Some(Key::F8),
                9 => Some(Key::F9),
                10 => Some(Key::F10),
                11 => Some(Key::F11),
                12 => Some(Key::F12),
                _ => None,
            };
        }
    }
    match code {
        "Minus" => Some(Key::Unicode('-')),
        "Equal" => Some(Key::Unicode('=')),
        "BracketLeft" => Some(Key::Unicode('[')),
        "BracketRight" => Some(Key::Unicode(']')),
        "Backslash" => Some(Key::Unicode('\\')),
        "Semicolon" => Some(Key::Unicode(';')),
        "Quote" => Some(Key::Unicode('\'')),
        "Backquote" => Some(Key::Unicode('`')),
        "Comma" => Some(Key::Unicode(',')),
        "Period" => Some(Key::Unicode('.')),
        "Slash" => Some(Key::Unicode('/')),
        "Enter" => Some(Key::Return),
        "Tab" => Some(Key::Tab),
        "Escape" => Some(Key::Escape),
        "Backspace" => Some(Key::Backspace),
        "Delete" => Some(Key::Delete),
        "Space" => Some(Key::Space),
        "ArrowUp" => Some(Key::UpArrow),
        "ArrowDown" => Some(Key::DownArrow),
        "ArrowLeft" => Some(Key::LeftArrow),
        "ArrowRight" => Some(Key::RightArrow),
        "ShiftLeft" | "ShiftRight" => Some(Key::Shift),
        "ControlLeft" | "ControlRight" => Some(Key::Control),
        "AltLeft" | "AltRight" => Some(Key::Alt),
        "MetaLeft" | "MetaRight" => Some(Key::Meta),
        "CapsLock" => Some(Key::CapsLock),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),
        // enigo's Key::Insert only exists on Windows / non-macOS Unix in 0.6.1;
        // macOS keyboards have no Insert key, so it falls through to None there.
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "Insert" => Some(Key::Insert),
        _ => None,
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib`
Expected: PASS — all `input.rs` unit tests (serde + mappers).

- [ ] **Step 6: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/input.rs
git commit -m "feat(agent): pure input mappers (coord/wheel/button/keycode) via enigo

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `InputInjector` (enigo on a dedicated thread)

**Files:**
- Modify: `agent/src/input.rs`
- Test: inline `#[cfg(test)]` (a non-injecting drain test + an `#[ignore]` real-injection smoke) in `agent/src/input.rs`

**Interfaces:**
- Consumes: `InputEvent`, `map_coord`/`pixels_to_clicks`/`map_button`/`code_to_key` (Task 3).
- Produces: `pub struct InputInjector`; `InputInjector::start() -> InputInjector`; `InputInjector::sender(&self) -> std::sync::mpsc::Sender<InputEvent>`. Dropping the injector closes the channel; the worker thread exits when all senders drop.

- [ ] **Step 1: Write the failing tests**

Add a third test module to `agent/src/input.rs`:

```rust
#[cfg(test)]
mod injector_tests {
    use super::*;

    // While the injector is alive the channel is open, so sends succeed. (No
    // display is required: if enigo init fails the worker drain-drops instead
    // of closing the channel.)
    #[test]
    fn sender_accepts_events_while_alive() {
        let injector = InputInjector::start();
        let tx = injector.sender();
        assert!(tx.send(InputEvent::MMove { x: 0.5, y: 0.5 }).is_ok());
        assert!(tx.send(InputEvent::KDown { code: "KeyA".into() }).is_ok());
        drop(tx);
        drop(injector); // closes channel; worker exits
    }

    // Real injection: requires a display + (macOS) Accessibility permission.
    // Run explicitly with: cargo test --manifest-path agent/Cargo.toml -- --ignored
    #[test]
    #[ignore]
    fn injects_a_mouse_move() {
        let injector = InputInjector::start();
        injector
            .sender()
            .send(InputEvent::MMove { x: 0.5, y: 0.5 })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib injector_tests`
Expected: FAIL to compile — `InputInjector` not found.

- [ ] **Step 3: Implement `InputInjector`**

Add to `agent/src/input.rs`. Extend the enigo import to `use enigo::{Axis, Button, Coordinate, Direction, Enigo, InputResult, Key, Keyboard, Mouse, Settings};` and add:

```rust
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

/// Owns an `enigo::Enigo` on a dedicated OS thread and injects `InputEvent`s
/// serially. The data-channel callback only parses frames and pushes them to
/// `sender()`; this keeps `Enigo` (which is `!Send`) off the async runtime and
/// serializes injection in event order.
pub struct InputInjector {
    tx: Sender<InputEvent>,
    _handle: JoinHandle<()>,
}

impl InputInjector {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<InputEvent>();
        let handle = std::thread::spawn(move || injector_loop(rx));
        Self { tx, _handle: handle }
    }

    pub fn sender(&self) -> Sender<InputEvent> {
        self.tx.clone()
    }
}

fn injector_loop(rx: Receiver<InputEvent>) {
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(
                "input injection unavailable ({e}); dropping input events. \
                 On macOS grant Accessibility in System Settings → Privacy & \
                 Security → Accessibility; on Linux use X11 (Wayland limits synthetic input)."
            );
            for _ in rx {} // drain so senders don't error, until channel closes
            return;
        }
    };
    while let Ok(ev) = rx.recv() {
        if let Err(e) = inject(&mut enigo, &ev) {
            tracing::warn!("failed to inject {ev:?}: {e}");
        }
    }
}

fn inject(enigo: &mut Enigo, ev: &InputEvent) -> InputResult<()> {
    match ev {
        InputEvent::MMove { x, y } => {
            let (w, h) = enigo.main_display()?;
            let (px, py) = map_coord(*x, *y, w, h);
            enigo.move_mouse(px, py, Coordinate::Abs)
        }
        InputEvent::MDown { button } => enigo.button(map_button(button), Direction::Press),
        InputEvent::MUp { button } => enigo.button(map_button(button), Direction::Release),
        InputEvent::Wheel { dx, dy } => {
            let cy = pixels_to_clicks(*dy);
            if cy != 0 {
                enigo.scroll(cy, Axis::Vertical)?;
            }
            let cx = pixels_to_clicks(*dx);
            if cx != 0 {
                enigo.scroll(cx, Axis::Horizontal)?;
            }
            Ok(())
        }
        InputEvent::KDown { code } => match code_to_key(code) {
            Some(k) => enigo.key(k, Direction::Press),
            None => {
                tracing::warn!("unmapped key code: {code}");
                Ok(())
            }
        },
        InputEvent::KUp { code } => match code_to_key(code) {
            Some(k) => enigo.key(k, Direction::Release),
            None => {
                tracing::warn!("unmapped key code: {code}");
                Ok(())
            }
        },
    }
}
```

Note: `Button`/`Key` are already imported from Task 3; merge the import lines so `enigo::{...}` is imported once.

- [ ] **Step 4: Run to verify the non-ignored test passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib injector_tests`
Expected: PASS (`sender_accepts_events_while_alive`; `injects_a_mouse_move` shows as ignored).

- [ ] **Step 5: Commit**

```bash
git add agent/src/input.rs
git commit -m "feat(agent): InputInjector — enigo on a dedicated thread

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Accessibility permission check

**Files:**
- Modify: `agent/Cargo.toml`, `agent/src/lib.rs`, `agent/src/main.rs`
- Create: `agent/src/permission.rs`
- Test: inline `#[cfg(test)]` in `agent/src/permission.rs`

**Interfaces:**
- Produces: `pub fn check_input_permission() -> bool` — returns whether input injection is expected to work; logs actionable guidance when not. Always `true` on non-macOS.

- [ ] **Step 1: Add the macOS-only dependency**

In `agent/Cargo.toml`, add a target-scoped section (place after `[dependencies]`):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
macos-accessibility-client = "0.0.2"
```

- [ ] **Step 2: Write the failing test**

Create `agent/src/permission.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_returns_true_off_macos() {
        // Off macOS the check is a no-op that must report available.
        #[cfg(not(target_os = "macos"))]
        assert!(check_input_permission());
        // On macOS it reflects the live Accessibility trust state, which a unit
        // test can't assert; just confirm it runs without panicking.
        #[cfg(target_os = "macos")]
        {
            let _trusted: bool = check_input_permission();
        }
    }
}
```

- [ ] **Step 3: Register the module and run to verify it fails**

Add to `agent/src/lib.rs`:

```rust
pub mod permission;
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib permission::`
Expected: FAIL to compile — `check_input_permission` not found.

- [ ] **Step 4: Implement the check**

Prepend to `agent/src/permission.rs`:

```rust
/// Check whether the process can inject input, logging actionable guidance when
/// it can't. On macOS this queries the Accessibility trust state (and prompts on
/// first run); elsewhere it's a no-op that returns true.
pub fn check_input_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        let trusted = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
        if !trusted {
            tracing::warn!(
                "macOS Accessibility permission not granted — mouse/keyboard \
                 injection will not work. Approve this program under System \
                 Settings → Privacy & Security → Accessibility, then restart it."
            );
        }
        trusted
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib permission::`
Expected: PASS (1 test).

- [ ] **Step 6: Call it at startup**

In `agent/src/main.rs`, after the `tracing_subscriber` init line and before loading config, add:

```rust
    if !rd_agent::permission::check_input_permission() {
        tracing::warn!("continuing without input permission; session will connect but injection is disabled");
    }
```

- [ ] **Step 7: Verify the agent still builds**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build --manifest-path agent/Cargo.toml`
Expected: builds clean.

- [ ] **Step 8: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/permission.rs agent/src/lib.rs agent/src/main.rs
git commit -m "feat(agent): macOS Accessibility permission check at startup

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Wire input into `PeerSession` (replace echo)

**Files:**
- Modify: `agent/src/webrtc_peer.rs`
- Create: `agent/tests/input_loopback.rs`
- Delete: `agent/tests/echo_loopback.rs`
- Modify: `agent/tests/ice_trickle.rs`

**Interfaces:**
- Consumes: `InputInjector` (Task 4), `InputEvent` (Task 2).
- Produces: `PeerSession::new(ice_servers, local_ice_tx)` unchanged (now owns an `InputInjector`); new test seam `pub async fn new_with_input_sink(ice_servers: Vec<IceServer>, local_ice_tx: UnboundedSender<serde_json::Value>, input_tx: std::sync::mpsc::Sender<InputEvent>) -> Result<PeerSession>`.

- [ ] **Step 1: Write the failing loopback test**

Create `agent/tests/input_loopback.rs` (mirrors the deleted echo test, but asserts a parsed `InputEvent` arrives on the sink):

```rust
use rd_agent::input::InputEvent;
use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// The agent (answerer) receives a data-channel frame, parses it into an
// InputEvent, and forwards it to the input sink. This test injects a test
// sink (std mpsc) instead of a real enigo injector, so no display is needed.
#[tokio::test]
async fn agent_forwards_parsed_input_events() {
    let (agent_ice_tx, _agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<InputEvent>();
    let agent = PeerSession::new_with_input_sink(vec![], agent_ice_tx, input_tx)
        .await
        .unwrap();

    let api = APIBuilder::new().build();
    let web = Arc::new(
        api.new_peer_connection(RTCConfiguration::default())
            .await
            .unwrap(),
    );
    let dc = web.create_data_channel("input", None).await.unwrap();

    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move {
            let _ = dc3
                .send_text(r#"{"t":"mdown","button":"left"}"#.to_string())
                .await;
        })
    }));

    let offer = web.create_offer(None).await.unwrap();
    let mut web_gather = web.gathering_complete_promise().await;
    web.set_local_description(offer).await.unwrap();
    let _ = web_gather.recv().await;
    let full_offer = web.local_description().await.unwrap();

    let answer_sdp = agent.accept_offer(&full_offer.sdp).await.unwrap();
    let answer = RTCSessionDescription::answer(answer_sdp).unwrap();
    web.set_remote_description(answer).await.unwrap();

    // Block for the parsed event on a worker thread (std mpsc), with a timeout.
    let got = tokio::task::spawn_blocking(move || {
        input_rx.recv_timeout(std::time::Duration::from_secs(10))
    })
    .await
    .unwrap()
    .expect("timed out waiting for input event");

    assert_eq!(got, InputEvent::MDown { button: rd_agent::input::MouseButton::Left });

    agent.close().await.unwrap();
    web.close().await.unwrap();
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --test input_loopback`
Expected: FAIL to compile — `new_with_input_sink` not found.

- [ ] **Step 3: Add injector ownership + the input sink to `PeerSession`**

In `agent/src/webrtc_peer.rs`:

Add imports:

```rust
use crate::input::{InputEvent, InputInjector};
use std::sync::mpsc::Sender;
```

Extend the struct (keep the `ice_buffer` field from Task 1):

```rust
pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
    ice_buffer: Mutex<IceBuffer<RTCIceCandidateInit>>,
    _injector: Option<InputInjector>,
}
```

- [ ] **Step 4: Replace `wire_echo` with `wire_input` and refactor `new`**

Delete the `wire_echo` function and replace it with:

```rust
fn wire_input(dc: Arc<RTCDataChannel>, input_tx: Sender<InputEvent>) {
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let input_tx = input_tx.clone();
        Box::pin(async move {
            let text = match String::from_utf8(msg.data.to_vec()) {
                Ok(t) => t,
                Err(_) => {
                    tracing::warn!("dropping non-utf8 input frame");
                    return;
                }
            };
            match serde_json::from_str::<InputEvent>(&text) {
                Ok(ev) => {
                    let _ = input_tx.send(ev);
                }
                Err(e) => tracing::warn!("dropping malformed input event: {e}"),
            }
        })
    }));
}
```

Refactor `PeerSession::new` into a private `build` + two public constructors. Replace the existing `pub async fn new(...)` body: keep everything up to and including the `on_ice_candidate` wiring inside `build`, change the `on_data_channel` closure to call `wire_input`, and thread `input_tx` through:

```rust
impl PeerSession {
    /// Production constructor: owns a real enigo-backed injector.
    pub async fn new(
        ice_servers: Vec<IceServer>,
        local_ice_tx: UnboundedSender<serde_json::Value>,
    ) -> Result<PeerSession> {
        let injector = InputInjector::start();
        let tx = injector.sender();
        let mut session = Self::build(ice_servers, local_ice_tx, tx).await?;
        session._injector = Some(injector);
        Ok(session)
    }

    /// Test seam: forward parsed input events to a caller-provided sink instead
    /// of a real injector (no display/permission needed).
    pub async fn new_with_input_sink(
        ice_servers: Vec<IceServer>,
        local_ice_tx: UnboundedSender<serde_json::Value>,
        input_tx: Sender<InputEvent>,
    ) -> Result<PeerSession> {
        Self::build(ice_servers, local_ice_tx, input_tx).await
    }

    async fn build(
        ice_servers: Vec<IceServer>,
        local_ice_tx: UnboundedSender<serde_json::Value>,
        input_tx: Sender<InputEvent>,
    ) -> Result<PeerSession> {
        // ... existing MediaEngine/registry/api/pc construction ...
        // ... existing on_ice_candidate wiring ...

        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let input_tx = input_tx.clone();
            wire_input(dc, input_tx);
            Box::pin(async {})
        }));

        Ok(PeerSession {
            pc,
            ice_buffer: Mutex::new(IceBuffer::new()),
            _injector: None,
        })
    }
}
```

(Move the body of the old `new` verbatim into `build`, adding the `input_tx` capture; `new`/`new_with_input_sink` are thin wrappers.)

- [ ] **Step 5: Run the loopback test to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --test input_loopback`
Expected: PASS.

- [ ] **Step 6: Delete the obsolete echo test, update the trickle test**

Delete the echo test:

```bash
git rm agent/tests/echo_loopback.rs
```

In `agent/tests/ice_trickle.rs`: (a) change `create_data_channel("echo", ...)` to `create_data_channel("input", ...)`; (b) replace the echo assertion with input-event delivery. Change the agent construction to use the input sink and replace the on_message/echo block. Concretely, replace the agent-side setup:

```rust
    let (agent_ice_tx, mut agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<rd_agent::input::InputEvent>();
    let agent = Arc::new(
        rd_agent::webrtc_peer::PeerSession::new_with_input_sink(vec![], agent_ice_tx, input_tx)
            .await
            .unwrap(),
    );
```

Replace the data channel + echo-expectation block with:

```rust
    let dc = web.create_data_channel("input", None).await.unwrap();

    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move {
            let _ = dc3.send_text(r#"{"t":"kdown","code":"KeyA"}"#.to_string()).await;
        })
    }));

    // ... existing trickle SDP handshake (unchanged) ...

    let got = tokio::task::spawn_blocking(move || {
        input_rx.recv_timeout(std::time::Duration::from_secs(15))
    })
    .await
    .unwrap()
    .expect("timed out waiting for input event over trickle ICE");
    assert_eq!(got, rd_agent::input::InputEvent::KDown { code: "KeyA".into() });
```

Remove the now-unused `DataChannelMessage` import if the trickle test no longer references it.

- [ ] **Step 7: Run the whole agent suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml`
Expected: PASS — lib unit tests (serde + mappers + injector + permission), `input_loopback`, `ice_trickle` (both cases), `protocol_roundtrip`. No `echo_loopback`.

- [ ] **Step 8: Commit**

```bash
git add agent/src/webrtc_peer.rs agent/tests/input_loopback.rs agent/tests/ice_trickle.rs
git commit -m "feat(agent): inject InputEvents over data channel, replace echo

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Web input encoding in `rtc.ts`

**Files:**
- Modify: `packages/web/src/rtc.ts`, `packages/web/src/rtc.test.ts`

**Interfaces:**
- Consumes: `InputEvent`, `parseInputEvent` from `@rd/protocol`.
- Produces: `export function mouseCoords(clientX, clientY, rect: {left,top,width,height}): {x,y}`; `export function mouseButtonName(button: number): MouseButton | null`; `Session.sendInput(ev: InputEvent): void`. Data channel renamed `"input"`; the echo/`onEcho` path is removed.

- [ ] **Step 1: Write the failing encoding tests**

Add to `packages/web/src/rtc.test.ts`:

```ts
import { parseInputEvent } from "@rd/protocol";
import { mouseCoords, mouseButtonName } from "./rtc.js";

test("mouseCoords produces clamped [0,1] relative coords", () => {
  const rect = { left: 100, top: 50, width: 800, height: 600 };
  expect(mouseCoords(500, 350, rect)).toEqual({ x: 0.5, y: 0.5 });
  // out-of-bounds clamps into range
  expect(mouseCoords(0, 0, rect)).toEqual({ x: 0, y: 0 });
  expect(mouseCoords(2000, 2000, rect)).toEqual({ x: 1, y: 1 });
});

test("mouseButtonName maps DOM button ids", () => {
  expect(mouseButtonName(0)).toBe("left");
  expect(mouseButtonName(1)).toBe("middle");
  expect(mouseButtonName(2)).toBe("right");
  expect(mouseButtonName(3)).toBeNull();
});

test("encoded events pass the protocol validator", () => {
  const { x, y } = mouseCoords(500, 350, { left: 100, top: 50, width: 800, height: 600 });
  expect(parseInputEvent({ t: "mmove", x, y })).toEqual({ t: "mmove", x: 0.5, y: 0.5 });
  expect(parseInputEvent({ t: "mdown", button: mouseButtonName(0) })).toEqual({
    t: "mdown",
    button: "left",
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm test -- rtc.test`
Expected: FAIL — `mouseCoords`/`mouseButtonName` not exported.

- [ ] **Step 3: Implement the encoders + `sendInput`, rename the channel**

In `packages/web/src/rtc.ts`:

Add `type InputEvent`, `type MouseButton` to the `@rd/protocol` import.

Add the pure encoders (near the other pure helpers at the top):

```ts
/** Map an absolute pointer position to relative [0,1] coords within `rect`. */
export function mouseCoords(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
): { x: number; y: number } {
  const clamp = (n: number) => Math.min(1, Math.max(0, n));
  return {
    x: clamp((clientX - rect.left) / rect.width),
    y: clamp((clientY - rect.top) / rect.height),
  };
}

/** Map a DOM `MouseEvent.button` id to the protocol button name (or null). */
export function mouseButtonName(button: number): MouseButton | null {
  switch (button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    default:
      return null;
  }
}
```

In `SessionCallbacks`, remove `onEcho`. In the `Session` interface, replace `send: (text: string) => void;` with:

```ts
  /** Send an InputEvent over the "input" data channel (no-op until open). */
  sendInput: (ev: InputEvent) => void;
```

In `connectSession`, change the channel creation and drop the echo message handler:

```ts
    channel = pc.createDataChannel("input");
    channel.onopen = () => {
      setState("connected");
    };
    channel.onclose = () => {
      if (!closed) setState("closed");
    };
```

(Delete the `channel.onmessage = ... onEcho ...` block.)

Replace the returned `send` with `sendInput`:

```ts
  return {
    sendInput(ev: InputEvent) {
      if (channel && channel.readyState === "open") {
        channel.send(JSON.stringify(ev));
      }
    },
    close,
  };
```

Remove the now-unused `onEcho` destructuring in `connectSession` (`const { onState, onError } = callbacks;`).

- [ ] **Step 4: Run to verify it passes**

Run: `npm test -- rtc.test`
Expected: PASS — new encoding tests plus the existing `deriveWsUrl`/`buildConnect`/`buildOffer`/`buildIce` tests.

- [ ] **Step 5: Commit**

```bash
git add packages/web/src/rtc.ts packages/web/src/rtc.test.ts
git commit -m "feat(web): input encoding + sendInput over the input data channel

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Web capture panel in `SessionView`

**Files:**
- Modify: `packages/web/src/pages/SessionView.tsx`

**Interfaces:**
- Consumes: `Session.sendInput` (Task 7), `connectSession` with `{ onState, onError }` (no `onEcho`).
- Produces: UI only. Verified by typecheck + build (no component test exists in this package).

- [ ] **Step 1: Rewrite `SessionView` as a capture surface + event log**

Replace the body of `packages/web/src/pages/SessionView.tsx` with the following (keeps the header/state-badge pattern; swaps the echo chat for a focusable "remote screen" that captures input and logs the last events):

```tsx
import { useEffect, useRef, useState } from "react";
import type { Device, InputEvent } from "@rd/protocol";
import { API_BASE } from "../api.js";
import {
  connectSession,
  mouseButtonName,
  mouseCoords,
  type ConnectionState,
  type Session,
} from "../rtc.js";

export interface SessionViewProps {
  token: string;
  device: Device;
  onExit: () => void;
}

const STATE_LABEL: Record<ConnectionState, string> = {
  connecting: "Connecting…",
  signaling: "Negotiating…",
  connected: "Connected",
  closed: "Disconnected",
  error: "Error",
};

const STATE_COLOR: Record<ConnectionState, string> = {
  connecting: "#f59e0b",
  signaling: "#f59e0b",
  connected: "#22c55e",
  closed: "#9ca3af",
  error: "crimson",
};

function describe(ev: InputEvent): string {
  switch (ev.t) {
    case "mmove":
      return `mmove ${ev.x.toFixed(2)},${ev.y.toFixed(2)}`;
    case "mdown":
      return `mdown ${ev.button}`;
    case "mup":
      return `mup ${ev.button}`;
    case "wheel":
      return `wheel ${ev.dx.toFixed(0)},${ev.dy.toFixed(0)}`;
    case "kdown":
      return `kdown ${ev.code}`;
    case "kup":
      return `kup ${ev.code}`;
  }
}

/**
 * Remote session view. Until video lands (Plan 4), a focusable placeholder
 * "remote screen" captures mouse/keyboard, sends each as an InputEvent over the
 * data channel, and logs the most recent events so the operator can see input
 * is transmitting. Injection happens on the agent (被控端).
 */
export function SessionView({ token, device, onExit }: SessionViewProps) {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const sessionRef = useRef<Session | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  // rAF coalescing for mousemove: keep only the latest position per frame.
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const rafId = useRef<number | null>(null);

  useEffect(() => {
    setState("connecting");
    setError(null);
    setLog([]);
    const session = connectSession(API_BASE, token, device.id, {
      onState: setState,
      onError: setError,
    });
    sessionRef.current = session;
    return () => {
      session.close();
      sessionRef.current = null;
      if (rafId.current !== null) cancelAnimationFrame(rafId.current);
    };
  }, [token, device.id]);

  const connected = state === "connected";

  function emit(ev: InputEvent) {
    sessionRef.current?.sendInput(ev);
    setLog((prev) => [describe(ev), ...prev].slice(0, 20));
  }

  function onMouseMove(e: React.MouseEvent) {
    if (!connected || !surfaceRef.current) return;
    const rect = surfaceRef.current.getBoundingClientRect();
    pendingMove.current = mouseCoords(e.clientX, e.clientY, rect);
    if (rafId.current === null) {
      rafId.current = requestAnimationFrame(() => {
        rafId.current = null;
        const p = pendingMove.current;
        pendingMove.current = null;
        if (p) emit({ t: "mmove", x: p.x, y: p.y });
      });
    }
  }

  function onMouseDown(e: React.MouseEvent) {
    if (!connected) return;
    const button = mouseButtonName(e.button);
    if (button) emit({ t: "mdown", button });
  }

  function onMouseUp(e: React.MouseEvent) {
    if (!connected) return;
    const button = mouseButtonName(e.button);
    if (button) emit({ t: "mup", button });
  }

  function onWheel(e: React.WheelEvent) {
    if (!connected) return;
    emit({ t: "wheel", dx: e.deltaX, dy: e.deltaY });
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (!connected) return;
    if (e.code === "Escape") {
      surfaceRef.current?.blur();
      return;
    }
    e.preventDefault();
    emit({ t: "kdown", code: e.code });
  }

  function onKeyUp(e: React.KeyboardEvent) {
    if (!connected) return;
    e.preventDefault();
    emit({ t: "kup", code: e.code });
  }

  return (
    <div style={{ maxWidth: 720, margin: "5vh auto", fontFamily: "system-ui" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <button onClick={onExit}>← Back to devices</button>
        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            aria-label={STATE_LABEL[state]}
            title={STATE_LABEL[state]}
            style={{
              display: "inline-block", width: 10, height: 10, borderRadius: "50%",
              background: STATE_COLOR[state],
            }}
          />
          <span data-testid="conn-state">{STATE_LABEL[state]}</span>
        </span>
      </div>

      <h2>Session: {device.name}</h2>
      <p style={{ color: "#888" }}>
        Click the panel to capture — mouse & keyboard are sent to <code>{device.id}</code>. Press Esc to release.
      </p>

      {error && <p style={{ color: "crimson" }} role="alert">{error}</p>}

      <div
        ref={surfaceRef}
        data-testid="remote-surface"
        tabIndex={0}
        onMouseMove={onMouseMove}
        onMouseDown={onMouseDown}
        onMouseUp={onMouseUp}
        onWheel={onWheel}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onContextMenu={(e) => e.preventDefault()}
        style={{
          height: 360, borderRadius: 8, border: "2px dashed #cbd5e1",
          background: connected ? "#0f172a" : "#f1f5f9",
          color: connected ? "#94a3b8" : "#94a3b8",
          display: "flex", alignItems: "center", justifyContent: "center",
          textAlign: "center", outline: "none", userSelect: "none", cursor: connected ? "crosshair" : "default",
        }}
      >
        {connected ? "Remote screen (no video yet — input captured here)" : "Waiting for connection…"}
      </div>

      <h3 style={{ marginBottom: 4 }}>Sent events</h3>
      <div
        style={{
          border: "1px solid #eee", borderRadius: 8, padding: 8, height: 140,
          overflowY: "auto", background: "#fafafa", fontFamily: "ui-monospace, monospace", fontSize: 12,
        }}
      >
        {log.length === 0 && <p style={{ color: "#aaa", margin: 0 }}>No events yet.</p>}
        {log.map((line, i) => (
          <div key={i} data-testid="event-line">{line}</div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Typecheck and build the web package**

Run: `npm run typecheck && npm run -w @rd/web build`
Expected: both clean. (If the web build script differs, use the package's existing build command — check `packages/web/package.json`; the Plan 2b baseline used `vite build`.)

- [ ] **Step 3: Run the full JS test + typecheck baseline**

Run: `npm test && npm run typecheck`
Expected: 54+ tests pass (existing 54 plus Task 7's new encoding tests); typecheck clean.

- [ ] **Step 4: Commit**

```bash
git add packages/web/src/pages/SessionView.tsx
git commit -m "feat(web): capture panel + event log replacing echo chat

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: End-to-end smoke doc

**Files:**
- Create: `docs/superpowers/plan3-input-smoke.md`

- [ ] **Step 1: Write the smoke guide**

Create `docs/superpowers/plan3-input-smoke.md` documenting the manual end-to-end check. Base the server/coturn/agent/web bring-up steps on the existing `docs/superpowers/plan2b-e2e-smoke.md` (read it and reuse its exact commands), then add the Plan 3 verification:

```markdown
# Plan 3 — Input injection e2e smoke

Prereq: complete the Plan 2b bring-up (server + coturn + agent + web) from
`plan2b-e2e-smoke.md` so a session connects. macOS: grant the agent binary
Accessibility permission (System Settings → Privacy & Security → Accessibility)
and restart it — otherwise injection is silently disabled (the agent logs a
warning at startup).

## Steps
1. In the web app, open a session to the online device. Wait for the green
   "Connected" badge.
2. Click the dashed "Remote screen" panel to focus it (cursor becomes a crosshair).
3. Move the mouse across the panel → the被控端's real cursor moves; the "Sent
   events" log shows `mmove x,y` lines (coalesced to ~one per frame).
4. Click / right-click / middle-click → real clicks on the被控端; log shows
   `mdown`/`mup` with the button. Right-click does not open the browser menu.
5. Scroll the wheel over the panel → the被控端 scrolls; log shows `wheel dx,dy`.
6. Type (e.g. open a text editor on the被控端 first, focus the panel, type
   "Hello") → text appears on the被控端; log shows `kdown`/`kup KeyH` … Shift
   combos capitalize. Press Esc to release capture.
7. Agent logs (`RUST_LOG=info`) show received events; malformed/non-utf8 frames
   log a warning rather than crashing.

## Expected
Real cursor movement, clicks, scroll, and typing on the被控端, mirrored by the
event log on the control end. No ack is sent back (fire-and-forget).
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plan3-input-smoke.md
git commit -m "docs: Plan 3 input-injection e2e smoke guide

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## After all tasks

- Whole-branch review (subagent-driven-development's final review / requesting-code-review).
- Update `docs/BACKLOG.md`: mark Plan 3 ✅; remove the "Must consider before Plan 3 traffic" item A (now fixed); refresh the test counts (Node 54+, agent tests: lib unit + `input_loopback` + `ice_trickle`×2 + `protocol_roundtrip`). Commit separately.
- `superpowers:finishing-a-development-branch` to choose merge/PR.

## Self-Review (spec coverage)

- Spec §2.1 InputEvent mirror → Task 2. §2.2 injector + mappers → Tasks 3–4. §2.3 permission → Task 5. §2.4 wire into PeerSession + rename + warn on bad frames → Task 6. §2.5 ICE buffer → Task 1. §2.6 web capture + rtc encoders + rename + drop echo → Tasks 7–8. §5 tests → per-task TDD + `#[ignore]` real-injection + web encoding tests. §6 task split (agent ∥ web, docs last) → Parallel Groups. §7 deps → Global Constraints + Tasks 3/5.
- Type consistency: `InputEvent`/`MouseButton` variants identical across Tasks 2/4/6/7; `mouseCoords`/`mouseButtonName`/`sendInput` names identical across Tasks 7/8; `new_with_input_sink` signature identical across Tasks 6 (impl) and its tests.
- No placeholders: every code step contains complete code; every run step has an exact command + expected outcome.
```
