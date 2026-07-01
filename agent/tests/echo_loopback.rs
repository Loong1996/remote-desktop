use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// This test builds a "web side" PeerConnection in-process (creates the data
// channel + offer) and exchanges SDP directly with the PeerSession under
// test (agent, the answerer that echoes). Both sides wait for ICE gathering
// to complete before exchanging SDP (non-trickle), so no separate ICE
// candidate relay is needed.
#[tokio::test]
async fn agent_echoes_data_channel_messages() {
    // agent side (under test). This test uses non-trickle gathering, so the
    // local candidate sink is not exercised here; a dedicated trickle test
    // (tests/ice_trickle.rs) covers candidate emission and relay.
    let (agent_ice_tx, _agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let agent = PeerSession::new(vec![], agent_ice_tx).await.unwrap();

    // web side (built here in the test)
    let api = APIBuilder::new().build();
    let web = Arc::new(
        api.new_peer_connection(RTCConfiguration::default())
            .await
            .unwrap(),
    );
    let dc = web.create_data_channel("echo", None).await.unwrap();

    let (got_tx, mut got_rx) = mpsc::unbounded_channel::<String>();
    let got_tx2 = got_tx.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let s = String::from_utf8(msg.data.to_vec()).unwrap();
        let _ = got_tx2.send(s);
        Box::pin(async {})
    }));

    // data channel: send "hello" once open
    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move {
            let _ = dc3.send_text("hello".to_string()).await;
        })
    }));

    // handshake: web creates offer, waits for full ICE gathering, then
    // hands the complete offer SDP to agent.accept_offer, which itself
    // waits for its own gathering to complete before returning the answer.
    let offer = web.create_offer(None).await.unwrap();
    let mut web_gather_complete = web.gathering_complete_promise().await;
    web.set_local_description(offer).await.unwrap();
    let _ = web_gather_complete.recv().await;
    let full_offer = web.local_description().await.unwrap();

    let answer_sdp = agent.accept_offer(&full_offer.sdp).await.unwrap();
    let answer = RTCSessionDescription::answer(answer_sdp).unwrap();
    web.set_remote_description(answer).await.unwrap();

    // expect to receive echo:hello (timeout guards against hangs)
    let got = tokio::time::timeout(std::time::Duration::from_secs(10), got_rx.recv())
        .await
        .expect("timed out waiting for echo")
        .unwrap();
    assert_eq!(got, "echo:hello");

    agent.close().await.unwrap();
    web.close().await.unwrap();
}
