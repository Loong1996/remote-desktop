use enigo::{Axis, Button, Coordinate, Direction, Enigo, InputResult, Key, Keyboard, Mouse, Settings};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Map a `KeyboardEvent.code` (a physical key position) to a macOS virtual
/// keycode (`kVK_ANSI_*` / `kVK_*` from Carbon HIToolbox `Events.h`).
///
/// This is the macOS injection path: `code` is layout-independent and
/// position-based — exactly what a macOS virtual keycode is — so we map straight
/// to the keycode and post it with `enigo.raw()`. That deliberately bypasses
/// enigo's `Key::Unicode` path, whose char→keycode reverse lookup
/// (`UCKeyTranslate`) fails with `OSStatus -25340` in a headless/background
/// process context, breaking every letter/digit/symbol key. Modifiers still
/// compose correctly: `raw()` maintains the CGEvent flag state by keycode, so a
/// held Shift keycode uppercases a following letter keycode.
#[cfg(target_os = "macos")]
pub fn code_to_macos_keycode(code: &str) -> Option<u16> {
    let kc: u16 = match code {
        // ANSI letters
        "KeyA" => 0x00, "KeyB" => 0x0B, "KeyC" => 0x08, "KeyD" => 0x02,
        "KeyE" => 0x0E, "KeyF" => 0x03, "KeyG" => 0x05, "KeyH" => 0x04,
        "KeyI" => 0x22, "KeyJ" => 0x26, "KeyK" => 0x28, "KeyL" => 0x25,
        "KeyM" => 0x2E, "KeyN" => 0x2D, "KeyO" => 0x1F, "KeyP" => 0x23,
        "KeyQ" => 0x0C, "KeyR" => 0x0F, "KeyS" => 0x01, "KeyT" => 0x11,
        "KeyU" => 0x20, "KeyV" => 0x09, "KeyW" => 0x0D, "KeyX" => 0x07,
        "KeyY" => 0x10, "KeyZ" => 0x06,
        // Top-row digits
        "Digit1" => 0x12, "Digit2" => 0x13, "Digit3" => 0x14, "Digit4" => 0x15,
        "Digit5" => 0x17, "Digit6" => 0x16, "Digit7" => 0x1A, "Digit8" => 0x1C,
        "Digit9" => 0x19, "Digit0" => 0x1D,
        // Symbols / punctuation
        "Minus" => 0x1B, "Equal" => 0x18, "BracketLeft" => 0x21,
        "BracketRight" => 0x1E, "Backslash" => 0x2A, "Semicolon" => 0x29,
        "Quote" => 0x27, "Backquote" => 0x32, "Comma" => 0x2B,
        "Period" => 0x2F, "Slash" => 0x2C,
        // Editing / whitespace (macOS Delete key = ForwardDelete = 0x75)
        "Enter" => 0x24, "Tab" => 0x30, "Space" => 0x31, "Backspace" => 0x33,
        "Escape" => 0x35, "Delete" => 0x75,
        // Arrows
        "ArrowLeft" => 0x7B, "ArrowRight" => 0x7C, "ArrowDown" => 0x7D,
        "ArrowUp" => 0x7E,
        // Navigation cluster
        "Home" => 0x73, "End" => 0x77, "PageUp" => 0x74, "PageDown" => 0x79,
        // Modifiers
        "ShiftLeft" => 0x38, "ShiftRight" => 0x3C, "ControlLeft" => 0x3B,
        "ControlRight" => 0x3E, "AltLeft" => 0x3A, "AltRight" => 0x3D,
        "MetaLeft" => 0x37, "MetaRight" => 0x36, "CapsLock" => 0x39,
        // Function row
        "F1" => 0x7A, "F2" => 0x78, "F3" => 0x63, "F4" => 0x76, "F5" => 0x60,
        "F6" => 0x61, "F7" => 0x62, "F8" => 0x64, "F9" => 0x65, "F10" => 0x6D,
        "F11" => 0x67, "F12" => 0x6F,
        // Numpad
        "Numpad0" => 0x52, "Numpad1" => 0x53, "Numpad2" => 0x54, "Numpad3" => 0x55,
        "Numpad4" => 0x56, "Numpad5" => 0x57, "Numpad6" => 0x58, "Numpad7" => 0x59,
        "Numpad8" => 0x5B, "Numpad9" => 0x5C, "NumpadDecimal" => 0x41,
        "NumpadAdd" => 0x45, "NumpadSubtract" => 0x4E, "NumpadMultiply" => 0x43,
        "NumpadDivide" => 0x4B, "NumpadEnter" => 0x4C, "NumpadEqual" => 0x51,
        _ => return None,
    };
    Some(kc)
}

/// Inject a key press/release for a `KeyboardEvent.code`. On macOS this routes
/// through the physical virtual-keycode table + `enigo.raw()` (bypassing the
/// broken `Key::Unicode`/UCKeyTranslate lookup), falling back to the layout
/// `Key` path only for codes the keycode table doesn't cover. On other
/// platforms it uses the `Key` path, which works there.
fn inject_key(enigo: &mut Enigo, code: &str, direction: Direction) -> InputResult<()> {
    #[cfg(target_os = "macos")]
    {
        if let Some(kc) = code_to_macos_keycode(code) {
            return enigo.raw(kc, direction);
        }
    }
    match code_to_key(code) {
        Some(k) => enigo.key(k, direction),
        None => {
            tracing::warn!("unmapped key code: {code}");
            Ok(())
        }
    }
}

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

/// Tracks which keys/buttons are currently pressed, so they can be released if
/// the input channel closes mid-hold (web crash / tab close / disconnect) —
/// otherwise a held modifier or button would stick on the被控端.
#[derive(Default)]
struct PressedState {
    keys: std::collections::HashSet<String>,
    buttons: std::collections::HashSet<MouseButton>,
}

impl PressedState {
    fn apply(&mut self, ev: &InputEvent) {
        match ev {
            InputEvent::KDown { code } => {
                self.keys.insert(code.clone());
            }
            InputEvent::KUp { code } => {
                self.keys.remove(code);
            }
            InputEvent::MDown { button } => {
                self.buttons.insert(button.clone());
            }
            InputEvent::MUp { button } => {
                self.buttons.remove(button);
            }
            _ => {}
        }
    }

    fn pending_releases(&self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for code in &self.keys {
            out.push(InputEvent::KUp { code: code.clone() });
        }
        for button in &self.buttons {
            out.push(InputEvent::MUp { button: button.clone() });
        }
        out
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
    let mut pressed = PressedState::default();
    while let Ok(ev) = rx.recv() {
        pressed.apply(&ev);
        if let Err(e) = inject(&mut enigo, &ev) {
            tracing::warn!("failed to inject {ev:?}: {e}");
        }
    }
    // Channel closed (session ended / web gone): release anything still held so
    // no key or button sticks down on the被控端.
    for ev in pressed.pending_releases() {
        if let Err(e) = inject(&mut enigo, &ev) {
            tracing::warn!("failed to release {ev:?} on shutdown: {e}");
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
        InputEvent::KDown { code } => inject_key(enigo, code, Direction::Press),
        InputEvent::KUp { code } => inject_key(enigo, code, Direction::Release),
    }
}

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

    // On macOS injection goes through physical virtual keycodes (see
    // `code_to_macos_keycode`), bypassing the broken `Key::Unicode`/UCKeyTranslate
    // reverse lookup. Spot-check the table against Carbon `kVK_*` constants.
    #[cfg(target_os = "macos")]
    #[test]
    fn code_to_macos_keycode_maps_physical_positions() {
        assert_eq!(code_to_macos_keycode("KeyA"), Some(0x00));
        assert_eq!(code_to_macos_keycode("KeyH"), Some(0x04));
        assert_eq!(code_to_macos_keycode("KeyZ"), Some(0x06));
        assert_eq!(code_to_macos_keycode("Digit0"), Some(0x1D));
        assert_eq!(code_to_macos_keycode("Digit1"), Some(0x12));
        assert_eq!(code_to_macos_keycode("Space"), Some(0x31));
        assert_eq!(code_to_macos_keycode("Enter"), Some(0x24));
        // Backspace vs Delete is the classic macOS keycode gotcha: Backspace is
        // kVK_Delete (0x33), the browser's Delete is kVK_ForwardDelete (0x75).
        assert_eq!(code_to_macos_keycode("Backspace"), Some(0x33));
        assert_eq!(code_to_macos_keycode("Delete"), Some(0x75));
        // Digit row is deliberately "out of order" at 5/6 in the hardware map.
        assert_eq!(code_to_macos_keycode("Digit5"), Some(0x17));
        assert_eq!(code_to_macos_keycode("Digit6"), Some(0x16));
        // Left/right modifier codes are numerically "reversed" (L > R here).
        assert_eq!(code_to_macos_keycode("ShiftLeft"), Some(0x38));
        assert_eq!(code_to_macos_keycode("MetaLeft"), Some(0x37));
        assert_eq!(code_to_macos_keycode("MetaRight"), Some(0x36));
        assert_eq!(code_to_macos_keycode("ArrowUp"), Some(0x7E));
        // F-keys are not contiguous: F3 and F5 sit far from F1.
        assert_eq!(code_to_macos_keycode("F1"), Some(0x7A));
        assert_eq!(code_to_macos_keycode("F3"), Some(0x63));
        assert_eq!(code_to_macos_keycode("F5"), Some(0x60));
        assert_eq!(code_to_macos_keycode("Numpad5"), Some(0x57));
        assert_eq!(code_to_macos_keycode("NumpadEnter"), Some(0x4C));
        assert_eq!(code_to_macos_keycode("MediaPlayPause"), None);
    }
}

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

#[cfg(test)]
mod pressed_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tracks_and_releases_held_keys_and_buttons() {
        let mut s = PressedState::default();
        s.apply(&InputEvent::KDown { code: "ShiftLeft".into() });
        s.apply(&InputEvent::KDown { code: "KeyA".into() });
        s.apply(&InputEvent::KUp { code: "KeyA".into() }); // released, should not linger
        s.apply(&InputEvent::MDown { button: MouseButton::Left });
        let rel: HashSet<String> = s
            .pending_releases()
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect();
        // ShiftLeft still down, KeyA released, Left button still down
        assert!(rel.contains(&format!("{:?}", InputEvent::KUp { code: "ShiftLeft".into() })));
        assert!(rel.contains(&format!("{:?}", InputEvent::MUp { button: MouseButton::Left })));
        assert!(!rel.contains(&format!("{:?}", InputEvent::KUp { code: "KeyA".into() })));
        assert_eq!(rel.len(), 2);
    }

    #[test]
    fn nothing_to_release_when_all_up() {
        let mut s = PressedState::default();
        s.apply(&InputEvent::KDown { code: "KeyB".into() });
        s.apply(&InputEvent::KUp { code: "KeyB".into() });
        assert!(s.pending_releases().is_empty());
    }
}
