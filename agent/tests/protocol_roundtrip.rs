use rd_agent::protocol::{SdpDesc, SignalingMessage};

#[test]
fn agent_online_serializes_kebab_type() {
    let m = SignalingMessage::AgentOnline { token: "t1".into() };
    let s = serde_json::to_string(&m).unwrap();
    assert_eq!(s, r#"{"type":"agent-online","token":"t1"}"#);
}

#[test]
fn incoming_deserializes_camelcase_fields() {
    let json = r#"{"type":"incoming","sessionId":"s1","relayPolicy":"relay-fallback","iceServers":[{"urls":"stun:x:1"}]}"#;
    let m: SignalingMessage = serde_json::from_str(json).unwrap();
    match m {
        SignalingMessage::Incoming { session_id, relay_policy, .. } => {
            assert_eq!(session_id, "s1");
            assert_eq!(relay_policy, "relay-fallback");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn sdp_roundtrip_preserves_inner_type() {
    let m = SignalingMessage::Sdp {
        session_id: "s1".into(),
        sdp: SdpDesc { kind: "answer".into(), sdp: "v=0".into() },
    };
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains(r#""type":"sdp""#));
    assert!(s.contains(r#""sdp":{"type":"answer","sdp":"v=0"}"#));
    let back: SignalingMessage = serde_json::from_str(&s).unwrap();
    matches!(back, SignalingMessage::Sdp { .. });
}
