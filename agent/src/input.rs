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
