use serde::{Deserialize, Serialize};

/// Control-channel messages (clipboard + quality), mirrored from
/// packages/protocol/src/control.ts. The serde tag + kebab-case renaming must
/// match the TypeScript wire tags byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum ControlMessage {
    ClipSet { text: String },
    ClipRequest,
    ClipMode { mode: ClipMode },
    Quality {
        #[serde(rename = "bitrateBps")]
        bitrate_bps: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipMode {
    Off,
    Oneway,
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_variant_from_web_tags() {
        let cases = [
            (r#"{"t":"clip-set","text":"hi"}"#, ControlMessage::ClipSet { text: "hi".into() }),
            (r#"{"t":"clip-request"}"#, ControlMessage::ClipRequest),
            (r#"{"t":"clip-mode","mode":"both"}"#, ControlMessage::ClipMode { mode: ClipMode::Both }),
            (r#"{"t":"quality","bitrateBps":3000000}"#, ControlMessage::Quality { bitrate_bps: 3_000_000 }),
        ];
        for (json, want) in cases {
            let got: ControlMessage = serde_json::from_str(json).unwrap();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn serializes_clip_mode_with_web_tags() {
        let s = serde_json::to_string(&ControlMessage::ClipMode { mode: ClipMode::Oneway }).unwrap();
        assert_eq!(s, r#"{"t":"clip-mode","mode":"oneway"}"#);
    }
}
