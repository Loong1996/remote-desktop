use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

#[tokio::test]
async fn agent_answer_includes_video_when_offer_requests_it() {
    let (agent_ice_tx, _rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let agent = PeerSession::new(vec![], "relay-fallback", agent_ice_tx).await.unwrap();

    // web side: a media engine with default codecs (H264 included), offering
    // to RECEIVE video.
    let mut m = MediaEngine::default();
    m.register_default_codecs().unwrap();
    let mut reg = Registry::new();
    reg = register_default_interceptors(reg, &mut m).unwrap();
    let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(reg).build();
    let web = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await.unwrap());
    web.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, send_encodings: vec![] }),
    ).await.unwrap();

    let offer = web.create_offer(None).await.unwrap();
    let mut gather = web.gathering_complete_promise().await;
    web.set_local_description(offer).await.unwrap();
    let _ = gather.recv().await;
    let full_offer = web.local_description().await.unwrap();

    let answer_sdp = agent.accept_offer(&full_offer.sdp).await.unwrap();
    assert!(answer_sdp.contains("m=video"), "answer has no video m-line:\n{answer_sdp}");
    // agent sends video → its m-line is sendonly (or sendrecv)
    assert!(answer_sdp.contains("a=sendonly") || answer_sdp.contains("a=sendrecv"),
        "video not sendable in answer:\n{answer_sdp}");
    // REAL gate: SDP substrings (m=video/H264/sendonly) come from codec
    // registration + transceiver mirroring, so they pass even without a track.
    // Assert the agent actually attached a sendable video track — red if
    // pc.add_track is removed.
    assert_eq!(agent.video_sender_count().await, 1, "agent has no attached video sender track");

    let _ = RTCSessionDescription::answer(answer_sdp).unwrap();
    agent.close().await.unwrap();
    web.close().await.unwrap();
}
