use crate::input::{InputEvent, InputInjector};
use crate::protocol::IceServer;
use anyhow::Result;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::Mutex;
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

pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
    ice_buffer: Mutex<IceBuffer<RTCIceCandidateInit>>,
    _injector: Option<InputInjector>,
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

impl PeerSession {
    /// Production constructor: owns a real enigo-backed injector.
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
        // we pick it up here in on_data_channel and wire input forwarding.
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

    /// Handle a remote offer and return the local answer SDP. Waits for ICE
    /// gathering to complete so the returned answer already contains all
    /// local candidates (non-trickle), which keeps signaling simple.
    pub async fn accept_offer(&self, offer_sdp: &str) -> Result<String> {
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
        self.pc.set_remote_description(offer).await?;
        let flushed = { self.ice_buffer.lock().unwrap().on_remote_set() };
        for init in flushed {
            if let Err(e) = self.pc.add_ice_candidate(init).await {
                tracing::warn!("failed to add buffered ice candidate: {e}");
            }
        }
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
        let ready = { self.ice_buffer.lock().unwrap().accept(init) };
        if let Some(init) = ready {
            self.pc.add_ice_candidate(init).await?;
        }
        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }
}

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
