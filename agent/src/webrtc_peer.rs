use crate::clipboard;
use crate::control::{ClipMode, ControlMessage};
use crate::input::{InputEvent, InputInjector};
use crate::protocol::IceServer;
use crate::video::openh264_encoder::Openh264Encoder;
use crate::video::pipeline::VideoPipeline;
use crate::video::{make_source, EncodedSample, SampleSink};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
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
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

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
    _video: Option<VideoPipeline>,
    _keep_awake: KeepDisplayAwake,
}

/// Keeps the display awake for the session's lifetime. macOS sleeps the display
/// on a *local* idle timer that remote-injected input does not reset; a slept
/// display makes ScreenCaptureKit report "no display found" → a black remote
/// screen. Held via `caffeinate` for the session, dropped (restoring normal
/// display sleep) when the session ends.
struct KeepDisplayAwake {
    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
}

impl KeepDisplayAwake {
    fn start() -> Self {
        #[cfg(target_os = "macos")]
        {
            // -d: prevent display idle-sleep for the session; -u: declare user
            // activity so an already-asleep display wakes now. No -t → the -d
            // assertion is held until we kill the process on drop.
            let child = match std::process::Command::new("/usr/bin/caffeinate")
                .args(["-d", "-u"])
                .spawn()
            {
                Ok(child) => Some(child),
                Err(e) => {
                    tracing::warn!(
                        "could not keep the display awake ({e}); the remote screen \
                         may go black if the被控端 display sleeps"
                    );
                    None
                }
            };
            Self { child }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self {}
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for KeepDisplayAwake {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// SampleSink that forwards encoded H.264 to a WebRTC track. `write_sample` is
/// async, so we block on it via the runtime handle captured at construction.
struct TrackSampleSink {
    track: Arc<TrackLocalStaticSample>,
    handle: tokio::runtime::Handle,
}
impl SampleSink for TrackSampleSink {
    fn write(&self, sample: EncodedSample) {
        let track = self.track.clone();
        let s = Sample { data: sample.data.into(), duration: sample.duration, ..Default::default() };
        // block_on is safe here: this runs on the pipeline's own OS thread,
        // never on a runtime worker.
        if let Err(e) = self.handle.block_on(track.write_sample(&s)) {
            tracing::warn!("write_sample failed: {e}");
        }
    }
}

/// Map the session's relay policy to an ICE transport policy. `force-relay`
/// restricts gathering to relayed (TURN) candidates only — for cross-NAT paths
/// where direct/STUN candidates can't connect. Everything else allows all
/// candidate types (host/srflx/relay).
pub(crate) fn ice_transport_policy(relay_policy: &str) -> RTCIceTransportPolicy {
    if relay_policy == "force-relay" {
        RTCIceTransportPolicy::Relay
    } else {
        RTCIceTransportPolicy::All
    }
}

pub(crate) fn to_rtc_ice(servers: Vec<IceServer>) -> Vec<RTCIceServer> {
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
                // TURN servers use long-term password credentials. webrtc-rs
                // rejects a turn: URL whose credential_type isn't Password/Oauth
                // ("invalid turn server credentials"), and Default is Unspecified,
                // so set it explicitly. STUN URLs ignore it.
                credential_type: RTCIceCredentialType::Password,
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

/// Wire the bidirectional "control" data channel: clipboard sync + live quality.
/// `bitrate_tx` forwards quality requests to the video pipeline; `last_clipboard`
/// is the shared echo-suppression state. The agent only reads+broadcasts its own
/// clipboard while a `both` subscription is active (privacy: no unsolicited reads).
fn wire_control(dc: Arc<RTCDataChannel>, bitrate_tx: Sender<u32>, last_clipboard: Arc<Mutex<String>>) {
    // Poller lifecycle: `poller_on` is flipped false to stop the current poller.
    let poller_on = Arc::new(AtomicBool::new(false));

    // Stop the clipboard poller when the control channel closes (session end /
    // tab close / network drop) — otherwise the spawned task runs forever,
    // keeps reading the clipboard, and pins the dead channel.
    let poller_on_close = poller_on.clone();
    dc.on_close(Box::new(move || {
        poller_on_close.store(false, Ordering::SeqCst);
        Box::pin(async {})
    }));

    let dc_for_msg = dc.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let bitrate_tx = bitrate_tx.clone();
        let last_clipboard = last_clipboard.clone();
        let poller_on = poller_on.clone();
        let dc = dc_for_msg.clone();
        Box::pin(async move {
            let text = match String::from_utf8(msg.data.to_vec()) {
                Ok(t) => t,
                Err(_) => {
                    tracing::warn!("dropping non-utf8 control frame");
                    return;
                }
            };
            let ctl: ControlMessage = match serde_json::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("dropping malformed control message: {e}");
                    return;
                }
            };
            match ctl {
                ControlMessage::Quality { bitrate_bps } => {
                    let clamped = bitrate_bps.clamp(250_000, 20_000_000);
                    let _ = bitrate_tx.send(clamped);
                }
                ControlMessage::ClipSet { text } => {
                    if text.len() <= clipboard::CLIP_MAX_BYTES {
                        if let Err(e) = clipboard::write_clipboard(&text) {
                            tracing::warn!("write_clipboard failed: {e}");
                        } else {
                            *last_clipboard.lock().unwrap() = text;
                        }
                    }
                }
                ControlMessage::ClipRequest => match clipboard::read_clipboard() {
                    Ok(current) => {
                        if current.len() <= clipboard::CLIP_MAX_BYTES {
                            *last_clipboard.lock().unwrap() = current.clone();
                            send_clip_set(&dc, current);
                        } else {
                            tracing::warn!(
                                "clipboard pull skipped: {} bytes exceeds cap {}",
                                current.len(),
                                clipboard::CLIP_MAX_BYTES
                            );
                        }
                    }
                    Err(e) => tracing::warn!("read_clipboard failed: {e}"),
                },
                ControlMessage::ClipMode { mode } => {
                    if mode == ClipMode::Both {
                        start_clipboard_poller(dc.clone(), last_clipboard.clone(), poller_on.clone());
                    } else {
                        poller_on.store(false, Ordering::SeqCst);
                    }
                }
            }
        })
    }));
}

/// Serialize + send a clip-set on the control channel (fire-and-forget).
fn send_clip_set(dc: &Arc<RTCDataChannel>, text: String) {
    let msg = ControlMessage::ClipSet { text };
    let json = match serde_json::to_string(&msg) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("failed to serialize clip-set: {e}");
            return;
        }
    };
    let dc = dc.clone();
    tokio::spawn(async move {
        if let Err(e) = dc.send_text(json).await {
            tracing::warn!("failed to send clip-set: {e}");
        }
    });
}

/// Start (or restart) the agent-side clipboard poller for `both` mode. Reads the
/// clipboard ~every 800ms; on a change vs the shared last-known value, pushes a
/// clip-set to the web端. Stops when `poller_on` is set false (mode left `both`)
/// or the channel closes.
fn start_clipboard_poller(dc: Arc<RTCDataChannel>, last_clipboard: Arc<Mutex<String>>, poller_on: Arc<AtomicBool>) {
    // Idempotent: if already running, leave it.
    if poller_on.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        while poller_on.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            if !poller_on.load(Ordering::SeqCst) {
                break;
            }
            let current = match clipboard::read_clipboard() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let to_send = {
                let last = last_clipboard.lock().unwrap();
                clipboard::clipboard_to_send(&current, &last, clipboard::CLIP_MAX_BYTES)
            };
            if let Some(text) = to_send {
                *last_clipboard.lock().unwrap() = text.clone();
                send_clip_set(&dc, text);
            }
        }
    });
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
        relay_policy: &str,
        local_ice_tx: UnboundedSender<serde_json::Value>,
    ) -> Result<PeerSession> {
        let injector = InputInjector::start();
        let tx = injector.sender();
        let mut session = Self::build(ice_servers, relay_policy, local_ice_tx, tx).await?;
        session._injector = Some(injector);
        Ok(session)
    }

    /// Test seam: forward parsed input events to a caller-provided sink instead
    /// of a real injector (no display/permission needed).
    pub async fn new_with_input_sink(
        ice_servers: Vec<IceServer>,
        relay_policy: &str,
        local_ice_tx: UnboundedSender<serde_json::Value>,
        input_tx: Sender<InputEvent>,
    ) -> Result<PeerSession> {
        Self::build(ice_servers, relay_policy, local_ice_tx, input_tx).await
    }

    async fn build(
        ice_servers: Vec<IceServer>,
        relay_policy: &str,
        local_ice_tx: UnboundedSender<serde_json::Value>,
        input_tx: Sender<InputEvent>,
    ) -> Result<PeerSession> {
        // Wake + hold the display awake first, so it is available before we
        // query its size and start ScreenCaptureKit (a slept display would make
        // capture fail with "no display found"). WebRTC setup below gives the
        // wake a moment to take effect before capture starts.
        let keep_awake = KeepDisplayAwake::start();

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
            ice_transport_policy: ice_transport_policy(relay_policy),
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

        // agent is the answerer: the remote side creates the data channels;
        // we pick them up here in on_data_channel and dispatch by label.
        let (bitrate_tx, bitrate_rx) = std::sync::mpsc::channel::<u32>();

        // Shared "last clipboard value we set or saw", so the agent's poller does
        // not echo back a value the web端 just pushed (and vice-versa).
        let last_clipboard: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        let dc_input_tx = input_tx.clone();
        let ctl_bitrate_tx = bitrate_tx.clone();
        let ctl_last_clip = last_clipboard.clone();
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            match dc.label() {
                "input" => wire_input(dc, dc_input_tx.clone()),
                "control" => wire_control(dc, ctl_bitrate_tx.clone(), ctl_last_clip.clone()),
                other => tracing::warn!("ignoring unknown data channel: {other}"),
            }
            Box::pin(async {})
        }));

        // Video: start the capture→encode pipeline and add a sendonly H264 track.
        // Capture at the display's aspect ratio (fit within 1280x720) so the
        // frame has no in-frame letterbox — otherwise remote cursor coordinates
        // drift horizontally (see video::target_capture_size).
        let fps = 30u32;
        let (dst_w, dst_h) = crate::video::target_capture_size(1280, 720);
        // Build the encoder BEFORE adding the track. If init fails we add NO
        // video track, so the remote negotiates input-only rather than a
        // sendonly video m-line that is never fed (black screen).
        let encoder: Box<dyn crate::video::VideoEncoder> =
            match Openh264Encoder::new(dst_w, dst_h, 3_000_000, fps as f32) {
                Ok(e) => Box::new(e),
                Err(e) => {
                    tracing::error!("H264 encoder init failed, video disabled: {e}");
                    // still return a session (input-only) — mirror the injector-fail path
                    return Self::finish(pc, input_tx, None, keep_awake);
                }
            };
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability { mime_type: "video/H264".to_owned(), clock_rate: 90000, ..Default::default() },
            "video".to_owned(),
            "rd-agent".to_owned(),
        ));
        pc.add_track(video_track.clone()).await?;
        let sink: Arc<dyn SampleSink> = Arc::new(TrackSampleSink {
            track: video_track,
            handle: tokio::runtime::Handle::current(),
        });
        let capturer = make_source(dst_w, dst_h, fps);
        let video =
            VideoPipeline::start(capturer, encoder, sink, dst_w as usize, dst_h as usize, 60, bitrate_rx);
        Self::finish(pc, input_tx, Some(video), keep_awake)
    }

    fn finish(
        pc: Arc<RTCPeerConnection>,
        _input_tx: Sender<InputEvent>,
        video: Option<VideoPipeline>,
        keep_awake: KeepDisplayAwake,
    ) -> Result<PeerSession> {
        Ok(PeerSession {
            pc,
            ice_buffer: Mutex::new(IceBuffer::new()),
            _injector: None,
            _video: video,
            _keep_awake: keep_awake,
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

    /// Number of RTP senders that currently have a local track attached — i.e.
    /// how many media tracks the agent actually added (0 without `add_track`).
    pub async fn video_sender_count(&self) -> usize {
        let mut n = 0;
        for s in self.pc.get_senders().await {
            if s.track().await.is_some() {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod to_rtc_ice_tests {
    use super::{to_rtc_ice, RTCIceCredentialType};
    use crate::protocol::IceServer;

    #[test]
    fn turn_server_gets_password_credential_type() {
        // Regression: webrtc-rs rejects a turn: URL whose credential_type is the
        // Default (Unspecified) with "invalid turn server credentials". The
        // mapping must set Password so real TURN relay works.
        let servers = vec![IceServer {
            urls: serde_json::json!("turn:192.168.5.122:3478"),
            username: Some("rduser".into()),
            credential: Some("rdpass".into()),
        }];
        let rtc = to_rtc_ice(servers);
        assert_eq!(rtc.len(), 1);
        assert_eq!(rtc[0].username, "rduser");
        assert_eq!(rtc[0].credential, "rdpass");
        assert_eq!(rtc[0].credential_type, RTCIceCredentialType::Password);
    }

    #[test]
    fn stun_and_turn_urls_parse_into_multiple_servers() {
        let servers = vec![
            IceServer { urls: serde_json::json!("stun:h:3478"), username: None, credential: None },
            IceServer {
                urls: serde_json::json!("turn:h:3478"),
                username: Some("u".into()),
                credential: Some("p".into()),
            },
        ];
        let rtc = to_rtc_ice(servers);
        assert_eq!(rtc.len(), 2);
        assert_eq!(rtc[0].urls, vec!["stun:h:3478"]);
        assert_eq!(rtc[1].urls, vec!["turn:h:3478"]);
    }
}

#[cfg(test)]
mod ice_transport_policy_tests {
    use super::ice_transport_policy;
    use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;

    #[test]
    fn force_relay_maps_to_relay_only() {
        assert_eq!(ice_transport_policy("force-relay"), RTCIceTransportPolicy::Relay);
    }

    #[test]
    fn other_policies_allow_all_candidate_types() {
        assert_eq!(ice_transport_policy("relay-fallback"), RTCIceTransportPolicy::All);
        assert_eq!(ice_transport_policy("direct-only"), RTCIceTransportPolicy::All);
        assert_eq!(ice_transport_policy(""), RTCIceTransportPolicy::All);
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
