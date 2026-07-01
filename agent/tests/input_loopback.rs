use rd_agent::input::InputEvent;
use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// The agent (answerer) receives a data-channel frame, parses it into an
// InputEvent, and forwards it to the input sink. This test injects a test
// sink (std mpsc) instead of a real enigo injector, so no display is needed.
#[tokio::test]
async fn agent_forwards_parsed_input_events() {
    let (agent_ice_tx, _agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<InputEvent>();
    let agent = PeerSession::new_with_input_sink(vec![], agent_ice_tx, input_tx)
        .await
        .unwrap();

    let api = APIBuilder::new().build();
    let web = Arc::new(
        api.new_peer_connection(RTCConfiguration::default())
            .await
            .unwrap(),
    );
    let dc = web.create_data_channel("input", None).await.unwrap();

    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move {
            let _ = dc3
                .send_text(r#"{"t":"mdown","button":"left"}"#.to_string())
                .await;
        })
    }));

    let offer = web.create_offer(None).await.unwrap();
    let mut web_gather = web.gathering_complete_promise().await;
    web.set_local_description(offer).await.unwrap();
    let _ = web_gather.recv().await;
    let full_offer = web.local_description().await.unwrap();

    let answer_sdp = agent.accept_offer(&full_offer.sdp).await.unwrap();
    let answer = RTCSessionDescription::answer(answer_sdp).unwrap();
    web.set_remote_description(answer).await.unwrap();

    // Block for the parsed event on a worker thread (std mpsc), with a timeout.
    let got = tokio::task::spawn_blocking(move || {
        input_rx.recv_timeout(std::time::Duration::from_secs(10))
    })
    .await
    .unwrap()
    .expect("timed out waiting for input event");

    assert_eq!(got, InputEvent::MDown { button: rd_agent::input::MouseButton::Left });

    agent.close().await.unwrap();
    web.close().await.unwrap();
}
