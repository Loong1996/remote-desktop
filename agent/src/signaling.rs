use crate::config::AgentConfig;
use crate::protocol::{SdpDesc, SignalingMessage};
use crate::webrtc_peer::PeerSession;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub fn ws_url_from(server_url: &str) -> String {
    if let Some(rest) = server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        server_url.to_string()
    }
}

pub async fn run_agent(config: AgentConfig) -> Result<()> {
    let url = ws_url_from(&config.server_url);
    let (ws, _) = connect_async(&url).await?;
    let (mut write, mut read) = ws.split();

    // 上线
    let online = serde_json::to_string(&SignalingMessage::AgentOnline {
        token: config.device_token.clone(),
    })?;
    write.send(Message::Text(online)).await?;
    tracing::info!("agent online, waiting for sessions");

    let mut current: Option<(String, PeerSession)> = None;

    // Channel carrying locally-gathered ICE candidates out of each PeerSession.
    // A clone of the sender is handed to every PeerSession::new; the receiver is
    // drained below and each candidate is trickled to the remote peer, tagged
    // with the current session id.
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();

    loop {
        tokio::select! {
            // Locally-gathered ICE candidate → trickle it to the remote peer.
            Some(candidate) = ice_rx.recv() => {
                if let Some((sid, _)) = &current {
                    let out = SignalingMessage::Ice {
                        session_id: sid.clone(),
                        candidate,
                    };
                    match serde_json::to_string(&out) {
                        Ok(txt) => {
                            if let Err(e) = write.send(Message::Text(txt)).await {
                                tracing::error!("failed to send local ice candidate: {e}");
                            }
                        }
                        Err(e) => tracing::warn!("failed to serialize local ice candidate: {e}"),
                    }
                }
            }

            // Inbound signaling message from the server.
            item = read.next() => {
                let item = match item {
                    Some(item) => item,
                    None => break,
                };
                let msg = match item {
                    Ok(Message::Text(t)) => t,
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => continue,
                };
                let parsed: SignalingMessage = match serde_json::from_str(&msg) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("bad signaling msg: {e}");
                        continue;
                    }
                };
                match parsed {
                    SignalingMessage::Incoming {
                        session_id,
                        ice_servers,
                        ..
                    } => {
                        if current.is_some() {
                            tracing::info!(
                                "incoming session {session_id} supersedes existing current session"
                            );
                        }
                        let peer = match PeerSession::new(ice_servers, ice_tx.clone()).await {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::error!(
                                    "failed to construct peer session for {session_id}: {e}"
                                );
                                continue;
                            }
                        };
                        current = Some((session_id, peer));
                        tracing::info!("incoming session accepted, awaiting offer");
                    }
                    SignalingMessage::Sdp { session_id, sdp } if sdp.kind == "offer" => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                let answer_sdp = match peer.accept_offer(&sdp.sdp).await {
                                    Ok(a) => a,
                                    Err(e) => {
                                        tracing::error!(
                                            "failed to accept offer for session {session_id}: {e}"
                                        );
                                        continue;
                                    }
                                };
                                let reply = serde_json::to_string(&SignalingMessage::Sdp {
                                    session_id,
                                    sdp: SdpDesc {
                                        kind: "answer".into(),
                                        sdp: answer_sdp,
                                    },
                                })?;
                                if let Err(e) = write.send(Message::Text(reply)).await {
                                    tracing::error!("failed to send answer: {e}");
                                    continue;
                                }
                            }
                        }
                    }
                    SignalingMessage::Ice { session_id, candidate } => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                if let Err(e) = peer.add_remote_ice(candidate).await {
                                    tracing::warn!(
                                        "failed to add remote ice candidate for session {session_id}: {e}"
                                    );
                                }
                            }
                        }
                    }
                    SignalingMessage::Error { code, message } => {
                        tracing::error!("signaling error {code}: {message}");
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ws_url_http_to_ws() {
        assert_eq!(ws_url_from("http://127.0.0.1:8080"), "ws://127.0.0.1:8080");
        assert_eq!(ws_url_from("https://x.example:443"), "wss://x.example:443");
    }
}
