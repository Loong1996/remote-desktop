use crate::protocol::IceServer;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
}

fn to_rtc_ice(servers: Vec<IceServer>) -> Vec<RTCIceServer> {
    servers
        .into_iter()
        .map(|s| {
            let urls = match s.urls {
                serde_json::Value::String(u) => vec![u],
                serde_json::Value::Array(a) => a
                    .into_iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                _ => vec![],
            };
            RTCIceServer {
                urls,
                username: s.username.unwrap_or_default(),
                credential: s.credential.unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect()
}

fn wire_echo(dc: Arc<RTCDataChannel>) {
    let dc_for_msg = dc.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let dc = dc_for_msg.clone();
        Box::pin(async move {
            if let Ok(text) = String::from_utf8(msg.data.to_vec()) {
                let _ = dc.send_text(format!("echo:{text}")).await;
            }
        })
    }));
}

impl PeerSession {
    /// Construct an answerer peer session.
    ///
    /// `local_ice_tx` receives each locally-gathered ICE candidate (serialized
    /// from `RTCIceCandidateInit` to a `serde_json::Value`) as it is
    /// discovered, so the signaling loop can trickle them to the remote peer.
    /// The final `None` candidate emitted by webrtc-rs (gathering complete) is
    /// skipped.
    pub async fn new(
        ice_servers: Vec<IceServer>,
        local_ice_tx: UnboundedSender<serde_json::Value>,
    ) -> Result<PeerSession> {
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        let config = RTCConfiguration {
            ice_servers: to_rtc_ice(ice_servers),
            ..Default::default()
        };
        let pc = Arc::new(api.new_peer_connection(config).await?);

        // Emit each local ICE candidate through the sink so the signaling loop
        // can trickle it to the remote peer.
        pc.on_ice_candidate(Box::new(move |c| {
            let tx = local_ice_tx.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    match c.to_json() {
                        Ok(init) => match serde_json::to_value(init) {
                            Ok(v) => {
                                let _ = tx.send(v);
                            }
                            Err(e) => {
                                tracing::warn!("failed to serialize local ice candidate: {e}");
                            }
                        },
                        Err(e) => {
                            tracing::warn!("failed to convert local ice candidate to json: {e}");
                        }
                    }
                }
            })
        }));

        // agent is the answerer: the remote side creates the data channel;
        // we pick it up here in on_data_channel and wire the echo behavior.
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            wire_echo(dc);
            Box::pin(async {})
        }));

        Ok(PeerSession { pc })
    }

    /// Handle a remote offer and return the local answer SDP. Waits for ICE
    /// gathering to complete so the returned answer already contains all
    /// local candidates (non-trickle), which keeps signaling simple.
    pub async fn accept_offer(&self, offer_sdp: &str) -> Result<String> {
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
        self.pc.set_remote_description(offer).await?;
        let answer = self.pc.create_answer(None).await?;
        let mut gather_complete = self.pc.gathering_complete_promise().await;
        self.pc.set_local_description(answer).await?;
        let _ = gather_complete.recv().await;
        let local = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| anyhow::anyhow!("no local description after gathering"))?;
        Ok(local.sdp)
    }

    pub async fn add_remote_ice(&self, candidate: serde_json::Value) -> Result<()> {
        let init: RTCIceCandidateInit = serde_json::from_value(candidate)?;
        self.pc.add_ice_candidate(init).await?;
        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }
}
