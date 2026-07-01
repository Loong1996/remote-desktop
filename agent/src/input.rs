use enigo::{Axis, Button, Coordinate, Direction, Enigo, InputResult, Key, Keyboard, Mouse, Settings};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

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
