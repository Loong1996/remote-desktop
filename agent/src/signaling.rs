use crate::config::AgentConfig;
use crate::protocol::{SdpDesc, SignalingMessage};
use crate::webrtc_peer::PeerSession;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
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

const BASE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const STABLE_RESET: Duration = Duration::from_secs(60);

/// Result of one signaling connection's lifetime.
enum SessionOutcome {
    /// Transient close/error — reconnect after a backoff.
    Retry,
    /// Permanent failure (e.g. the device token was rejected) — stop.
    Fatal(String),
}

/// Exponential backoff: double, capped at MAX_BACKOFF.
fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_BACKOFF)
}

pub async fn run_agent(config: AgentConfig) -> Result<()> {
    let mut backoff = BASE_BACKOFF;
    loop {
        let started = std::time::Instant::now();
        match run_session(&config).await {
            SessionOutcome::Fatal(msg) => {
                tracing::error!("agent stopped: {msg}. Re-pair the device (delete its config) and restart.");
                return Err(anyhow::anyhow!(msg));
            }
            SessionOutcome::Retry => {
                // A connection that stayed up a while resets the backoff.
                if started.elapsed() >= STABLE_RESET {
                    backoff = BASE_BACKOFF;
                }
                tracing::warn!("signaling disconnected; reconnecting in {:?}", backoff);
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
            }
        }
    }
}

async fn run_session(config: &AgentConfig) -> SessionOutcome {
    let url = ws_url_from(&config.server_url);
    let (ws, _) = match connect_async(&url).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("connect failed: {e}");
            return SessionOutcome::Retry;
        }
    };
    let (mut write, mut read) = ws.split();

    let online = match serde_json::to_string(&SignalingMessage::AgentOnline {
        token: config.device_token.clone(),
    }) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to serialize agent-online: {e}");
            return SessionOutcome::Retry;
        }
    };
    if let Err(e) = write.send(Message::Text(online)).await {
        tracing::warn!("failed to send agent-online: {e}");
        return SessionOutcome::Retry;
    }
    tracing::info!("agent online, waiting for sessions");

    let mut current: Option<(String, PeerSession)> = None;
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();

    loop {
        tokio::select! {
            Some(candidate) = ice_rx.recv() => {
                if let Some((sid, _)) = &current {
                    let out = SignalingMessage::Ice { session_id: sid.clone(), candidate };
                    match serde_json::to_string(&out) {
                        Ok(txt) => {
                            if let Err(e) = write.send(Message::Text(txt)).await {
                                tracing::error!("failed to send local ice candidate: {e}");
                                return SessionOutcome::Retry;
                            }
                        }
                        Err(e) => tracing::warn!("failed to serialize local ice candidate: {e}"),
                    }
                }
            }

            item = read.next() => {
                let item = match item {
                    Some(item) => item,
                    None => return SessionOutcome::Retry,
                };
                let msg = match item {
                    Ok(Message::Text(t)) => t,
                    Ok(Message::Close(_)) | Err(_) => return SessionOutcome::Retry,
                    _ => continue,
                };
                let parsed: SignalingMessage = match serde_json::from_str(&msg) {
                    Ok(m) => m,
                    Err(e) => { tracing::warn!("bad signaling msg: {e}"); continue; }
                };
                match parsed {
                    SignalingMessage::Incoming { session_id, relay_policy, ice_servers } => {
                        if current.is_some() {
                            tracing::info!("incoming session {session_id} supersedes existing current session");
                        }
                        let peer = match PeerSession::new(ice_servers, &relay_policy, ice_tx.clone()).await {
                            Ok(p) => p,
                            Err(e) => { tracing::error!("failed to construct peer session for {session_id}: {e}"); continue; }
                        };
                        current = Some((session_id, peer));
                        tracing::info!("incoming session accepted, awaiting offer");
                    }
                    SignalingMessage::Sdp { session_id, sdp } if sdp.kind == "offer" => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                let answer_sdp = match peer.accept_offer(&sdp.sdp).await {
                                    Ok(a) => a,
                                    Err(e) => { tracing::error!("failed to accept offer for session {session_id}: {e}"); continue; }
                                };
                                let reply = match serde_json::to_string(&SignalingMessage::Sdp {
                                    session_id,
                                    sdp: SdpDesc { kind: "answer".into(), sdp: answer_sdp },
                                }) {
                                    Ok(r) => r,
                                    Err(e) => { tracing::error!("failed to serialize answer: {e}"); continue; }
                                };
                                if let Err(e) = write.send(Message::Text(reply)).await {
                                    tracing::error!("failed to send answer: {e}");
                                    return SessionOutcome::Retry;
                                }
                            }
                        }
                    }
                    SignalingMessage::Ice { session_id, candidate } => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                if let Err(e) = peer.add_remote_ice(candidate).await {
                                    tracing::warn!("failed to add remote ice candidate for session {session_id}: {e}");
                                }
                            }
                        }
                    }
                    SignalingMessage::PeerLeft { session_id } => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                let _ = peer.close().await;
                                current = None;
                                tracing::info!("peer left session {session_id}; released");
                            }
                        }
                    }
                    SignalingMessage::Error { code, message } => {
                        tracing::error!("signaling error {code}: {message}");
                        if code == "bad-token" {
                            return SessionOutcome::Fatal(format!("device token rejected: {message}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ws_url_http_to_ws() {
        assert_eq!(ws_url_from("http://127.0.0.1:8080"), "ws://127.0.0.1:8080");
        assert_eq!(ws_url_from("https://x.example:443"), "wss://x.example:443");
    }

    use std::time::Duration;

    #[test]
    fn backoff_doubles_and_caps_at_30s() {
        assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
        assert_eq!(next_backoff(Duration::from_secs(16)), Duration::from_secs(30)); // 32 capped
        assert_eq!(next_backoff(Duration::from_secs(30)), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn run_session_retries_when_server_unreachable() {
        // A refused connection must yield Retry (not Fatal, not a panic).
        let cfg = AgentConfig {
            server_url: "http://127.0.0.1:9".to_string(), // discard port, refused
            device_id: "d".to_string(),
            device_token: "t".to_string(),
        };
        assert!(matches!(run_session(&cfg).await, SessionOutcome::Retry));
    }
}
