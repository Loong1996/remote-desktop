use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdpDesc {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SignalingMessage {
    #[serde(rename_all = "camelCase")]
    AgentOnline { token: String },
    #[serde(rename_all = "camelCase")]
    Connect { device_id: String },
    #[serde(rename_all = "camelCase")]
    Incoming {
        session_id: String,
        relay_policy: String,
        ice_servers: Vec<IceServer>,
    },
    #[serde(rename_all = "camelCase")]
    SessionReady {
        session_id: String,
        relay_policy: String,
        ice_servers: Vec<IceServer>,
    },
    #[serde(rename_all = "camelCase")]
    Sdp { session_id: String, sdp: SdpDesc },
    #[serde(rename_all = "camelCase")]
    Ice {
        session_id: String,
        candidate: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    PeerLeft { session_id: String },
    Error { code: String, message: String },
}
