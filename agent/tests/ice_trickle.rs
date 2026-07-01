use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// Pure bidirectional trickle-ICE loopback.
//
// The agent (PeerSession, the answerer) and a "web side" PeerConnection built
// here exchange ONLY the SDP offer/answer up front (no gathering wait), then
// relay ICE candidates to each other as they are discovered. The agent emits
// its local candidates through the mpsc sender passed to PeerSession::new; the
// web side emits its candidates through on_ice_candidate. Each side feeds the
// other's candidates in via add_remote_ice / add_ice_candidate. Connectivity
// only succeeds if trickle works in both directions.
#[tokio::test]
async fn agent_trickles_ice_and_echoes() {
    // agent side (under test): local candidates come out on agent_ice_rx.
    let (agent_ice_tx, mut agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<rd_agent::input::InputEvent>();
    let agent = Arc::new(
        PeerSession::new_with_input_sink(vec![], agent_ice_tx, input_tx)
            .await
            .unwrap(),
    );

    // web side (built here in the test).
    let api = APIBuilder::new().build();
    let web = Arc::new(
        api.new_peer_connection(RTCConfiguration::default())
            .await
            .unwrap(),
    );

    // web -> agent candidate relay.
    let agent_for_ice = agent.clone();
    web.on_ice_candidate(Box::new(move |c| {
        let agent = agent_for_ice.clone();
        Box::pin(async move {
            if let Some(c) = c {
                if let Ok(init) = c.to_json() {
                    if let Ok(v) = serde_json::to_value(init) {
                        let _ = agent.add_remote_ice(v).await;
                    }
                }
            }
        })
    }));

    // agent -> web candidate relay (drain agent_ice_rx in a task).
    let web_for_ice = web.clone();
    tokio::spawn(async move {
        while let Some(v) = agent_ice_rx.recv().await {
            if let Ok(init) = serde_json::from_value::<RTCIceCandidateInit>(v) {
                let _ = web_for_ice.add_ice_candidate(init).await;
            }
        }
    });

    let dc = web.create_data_channel("input", None).await.unwrap();

    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move {
            let _ = dc3.send_text(r#"{"t":"kdown","code":"KeyA"}"#.to_string()).await;
        })
    }));

    // Trickle handshake: exchange SDP WITHOUT waiting for gathering.
    let offer = web.create_offer(None).await.unwrap();
    web.set_local_description(offer.clone()).await.unwrap();

    let answer_sdp = agent.accept_offer(&offer.sdp).await.unwrap();
    let answer = RTCSessionDescription::answer(answer_sdp).unwrap();
    web.set_remote_description(answer).await.unwrap();

    let got = tokio::task::spawn_blocking(move || {
        input_rx.recv_timeout(std::time::Duration::from_secs(15))
    })
    .await
    .unwrap()
    .expect("timed out waiting for input event over trickle ICE");
    assert_eq!(got, rd_agent::input::InputEvent::KDown { code: "KeyA".into() });

    agent.close().await.unwrap();
    web.close().await.unwrap();
}

#[tokio::test]
async fn pre_offer_remote_candidate_is_buffered_not_dropped() {
    let (agent_ice_tx, _rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let agent = rd_agent::webrtc_peer::PeerSession::new(vec![], agent_ice_tx)
        .await
        .unwrap();
    // A candidate arriving before accept_offer must be accepted (buffered), not error.
    let candidate = serde_json::json!({
        "candidate": "candidate:1 1 udp 2130706431 127.0.0.1 54321 typ host",
        "sdpMid": "0",
        "sdpMLineIndex": 0
    });
    agent.add_remote_ice(candidate).await.unwrap();
    agent.close().await.unwrap();
}
