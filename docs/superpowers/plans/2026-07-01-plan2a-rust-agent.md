# Plan 2a: Rust Agent（信令 + WebRTC data channel 回显）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现被控端 Rust Agent，能：交互式登录换取并持久化 device token，连接 Node 信令服务端保持在线，收到入站会话后完成 WebRTC 握手，并在 data channel 上把控制端发来的消息原样回显（`echo:<msg>`）。这是"WebRTC 空连接打通"里程碑的被控端一半。

**Architecture:** 单个 Rust binary crate `rd-agent`（放在仓库 `agent/`）。分层模块：`config`（本地 token 持久化）、`provision`（HTTP 登录+配对）、`protocol`（信令消息的 serde 镜像，对齐 `@rd/protocol`）、`signaling`（tokio-tungstenite WS 客户端）、`webrtc_peer`（webrtc-rs 对端 + data channel 回显）。核心逻辑设计为可在进程内测试：WebRTC 回显用两个本地 PeerConnection 环回验证；provision 用 wiremock 假 HTTP 服务验证；protocol 用 serde 往返验证。

**Tech Stack:** Rust 2021、tokio、webrtc(webrtc-rs)、tokio-tungstenite、reqwest、serde/serde_json、anyhow、thiserror、rpassword、dirs、tracing/tracing-subscriber；测试用 wiremock、tokio-test。

## Global Constraints

- Rust toolchain 在 `~/.cargo/bin`，**每个 shell 命令前先** `export PATH="$HOME/.cargo/bin:$PATH"`（cargo/rustc 不在默认 PATH）。
- crate 位置：仓库根下的 `agent/`（binary crate，名 `rd-agent`，edition 2021）。
- 信令消息 JSON 必须与 `@rd/protocol`（`packages/protocol/src/signaling.ts`）逐字段一致：`type` 判别；类型有 `agent-online{token}`、`incoming{sessionId,relayPolicy,iceServers}`、`session-ready{...}`、`connect{deviceId}`、`sdp{sessionId,sdp:{type,sdp}}`、`ice{sessionId,candidate}`、`peer-left{sessionId}`、`error{code,message}`。serde 用 `#[serde(tag="type", rename_all="kebab-case")]` 并对字段名做 camelCase 映射（`sessionId`、`relayPolicy`、`iceServers`、`deviceId`）。
- Agent 通过 `agent-online{token}` 认证（不使用 `?token=` query，那是 web 端的）。
- data channel 回显语义：收到文本消息 `X` → 回发 `echo:X`。
- 配置文件默认路径：`dirs::config_dir()/rd-agent/config.json`；**测试与运行都支持用环境变量 `RD_AGENT_CONFIG` 覆盖路径**（测试用临时文件，绝不写用户真实配置）。
- 服务端地址：默认 `http://127.0.0.1:8080`（REST）与 `ws://127.0.0.1:8080`（WS），可用 `RD_SERVER_URL` 覆盖。
- 每个 Task 结束必须 `cargo test`（相关范围）通过后再 commit。首个 Task 会触发 webrtc-rs 等依赖首次编译，**较慢属正常**，不是失败。
- 提交信息英文，结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 不改动 `packages/` 下已有的 Node/TS 代码；本计划只新增 `agent/`。

## File Structure

```
agent/
  Cargo.toml
  .gitignore                # /target
  src/
    main.rs                 # 入口：初始化日志→加载或 provision 配置→连信令→运行
    config.rs               # AgentConfig 结构 + load/save（JSON 文件，路径可 env 覆盖）
    provision.rs            # 交互式登录：POST /login → JWT → POST /devices/pair → token
    protocol.rs             # 信令消息 serde 镜像（对齐 @rd/protocol）
    signaling.rs            # WS 客户端：agent-online、消息循环、把 sdp/ice 交给 webrtc_peer
    webrtc_peer.rs          # webrtc-rs 对端：应答 offer、data channel 回显
  tests/
    protocol_roundtrip.rs   # 协议 JSON 往返（集成测试，跨 crate 边界）
    echo_loopback.rs        # 进程内两 PeerConnection 环回，验证回显
```

**责任边界：** `config`/`provision`/`protocol` 纯逻辑无 WebRTC 依赖，可快速独立测。`webrtc_peer` 只管一个 PeerConnection 的生命周期与回显，暴露一个可被 `signaling` 驱动的接口。`signaling` 只管 WS 收发与消息路由，不含 WebRTC 细节。

---

### Task 1: Agent crate 脚手架 + 配置模块

**Files:**
- Create: `agent/Cargo.toml`, `agent/.gitignore`, `agent/src/main.rs`, `agent/src/config.rs`
- Test: `agent/src/config.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Produces:
  - `struct AgentConfig { server_url: String, device_id: String, device_token: String }`（`serde` 可序列化）。
  - `fn config_path() -> PathBuf`（尊重 `RD_AGENT_CONFIG` env，否则 `dirs::config_dir()/rd-agent/config.json`）。
  - `fn AgentConfig::load() -> anyhow::Result<Option<AgentConfig>>`（文件不存在返回 `Ok(None)`）。
  - `fn AgentConfig::save(&self) -> anyhow::Result<()>`（必要时创建父目录）。

- [ ] **Step 1: 写 Cargo.toml**

```toml
[package]
name = "rd-agent"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
webrtc = "0.11"
tokio-tungstenite = "0.23"
futures-util = "0.3"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "1"
rpassword = "7"
dirs = "5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
wiremock = "0.6"
tokio-test = "0.4"
tempfile = "3"
```

- [ ] **Step 2: 写 `.gitignore`**

```
/target
```

- [ ] **Step 3: 写失败测试（config round-trip，放在 config.rs 末尾）**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::env::set_var("RD_AGENT_CONFIG", &path);
        let cfg = AgentConfig {
            server_url: "http://127.0.0.1:8080".into(),
            device_id: "dev-1".into(),
            device_token: "tok-abc".into(),
        };
        cfg.save().unwrap();
        let loaded = AgentConfig::load().unwrap().expect("should exist");
        assert_eq!(loaded.device_id, "dev-1");
        assert_eq!(loaded.device_token, "tok-abc");
        std::env::remove_var("RD_AGENT_CONFIG");
    }
    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RD_AGENT_CONFIG", dir.path().join("nope.json"));
        assert!(AgentConfig::load().unwrap().is_none());
        std::env::remove_var("RD_AGENT_CONFIG");
    }
}
```

- [ ] **Step 4: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p rd-agent --manifest-path agent/Cargo.toml config`
Expected: 编译失败（`AgentConfig` 未定义）。首次会拉取并编译大量依赖，耗时较长属正常。

- [ ] **Step 5: 实现 config.rs**

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub server_url: String,
    pub device_id: String,
    pub device_token: String,
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("RD_AGENT_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("rd-agent").join("config.json")
}

impl AgentConfig {
    pub fn load() -> anyhow::Result<Option<AgentConfig>> {
        let path = config_path();
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&data)?))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
```

- [ ] **Step 6: 写最小 main.rs（能编译；后续 Task 逐步充实）**

```rust
mod config;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("rd-agent starting");
    // 实际启动逻辑在 Task 6 接入
    Ok(())
}
```

- [ ] **Step 7: 跑测试确认通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml config`
Expected: 2 个 config 测试通过；`cargo build --manifest-path agent/Cargo.toml` 成功。

- [ ] **Step 8: Commit**

```bash
git add agent/
git commit -m "feat(agent): scaffold rd-agent crate + config persistence"
```

---

### Task 2: 信令协议 serde 镜像

**Files:**
- Create: `agent/src/protocol.rs`
- Modify: `agent/src/main.rs`（加 `mod protocol;`）
- Test: `agent/tests/protocol_roundtrip.rs`

**Interfaces:**
- Consumes: 无（对齐 `@rd/protocol` 的 JSON 约定）。
- Produces:
  - `enum SignalingMessage`（serde `tag="type"`, `rename_all="kebab-case"`）含变体：`AgentOnline{token}`、`Connect{device_id}`、`Incoming{session_id,relay_policy,ice_servers}`、`SessionReady{session_id,relay_policy,ice_servers}`、`Sdp{session_id,sdp:SdpDesc}`、`Ice{session_id,candidate:serde_json::Value}`、`PeerLeft{session_id}`、`Error{code,message}`。
  - `struct SdpDesc{ #[serde(rename="type")] kind:String, sdp:String }`。
  - `struct IceServer{ urls:serde_json::Value, username:Option<String>, credential:Option<String> }`。
  - 字段用 `#[serde(rename_all="camelCase")]` 使 `session_id`↔`sessionId` 等对齐。

- [ ] **Step 1: 写失败测试**

`agent/tests/protocol_roundtrip.rs`:
```rust
use rd_agent::protocol::{SignalingMessage, SdpDesc};

#[test]
fn agent_online_serializes_kebab_type() {
    let m = SignalingMessage::AgentOnline { token: "t1".into() };
    let s = serde_json::to_string(&m).unwrap();
    assert_eq!(s, r#"{"type":"agent-online","token":"t1"}"#);
}

#[test]
fn incoming_deserializes_camelcase_fields() {
    let json = r#"{"type":"incoming","sessionId":"s1","relayPolicy":"relay-fallback","iceServers":[{"urls":"stun:x:1"}]}"#;
    let m: SignalingMessage = serde_json::from_str(json).unwrap();
    match m {
        SignalingMessage::Incoming { session_id, relay_policy, .. } => {
            assert_eq!(session_id, "s1");
            assert_eq!(relay_policy, "relay-fallback");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn sdp_roundtrip_preserves_inner_type() {
    let m = SignalingMessage::Sdp {
        session_id: "s1".into(),
        sdp: SdpDesc { kind: "answer".into(), sdp: "v=0".into() },
    };
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains(r#""type":"sdp""#));
    assert!(s.contains(r#""sdp":{"type":"answer","sdp":"v=0"}"#));
    let back: SignalingMessage = serde_json::from_str(&s).unwrap();
    matches!(back, SignalingMessage::Sdp { .. });
}
```

> 为让 `tests/` 能 `use rd_agent::...`，本 crate 需同时暴露 lib 目标。见 Step 3。

- [ ] **Step 2: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml --test protocol_roundtrip`
Expected: 编译失败（`rd_agent::protocol` 不存在 / 无 lib 目标）。

- [ ] **Step 3: 让 crate 兼具 lib + bin，并实现 protocol.rs**

在 `agent/Cargo.toml` 的 `[package]` 后加：
```toml
[lib]
name = "rd_agent"
path = "src/lib.rs"

[[bin]]
name = "rd-agent"
path = "src/main.rs"
```

新建 `agent/src/lib.rs`（导出模块给 bin 和 tests 复用）：
```rust
pub mod config;
pub mod protocol;
pub mod provision;
pub mod signaling;
pub mod webrtc_peer;
```

> 注意：`lib.rs` 一次性声明所有模块。后续 Task 创建 `provision.rs`/`signaling.rs`/`webrtc_peer.rs` 前，这些 `pub mod` 会导致编译失败——所以本 Task 先创建这三个文件的**空占位**（每个含 `//! placeholder` 或最小内容），随 Task 3/4/5 充实。`main.rs` 改为 `use rd_agent::config;` 等，去掉自身的 `mod` 声明。

创建占位文件（内容仅一行注释即可）：`agent/src/provision.rs`、`agent/src/signaling.rs`、`agent/src/webrtc_peer.rs` 各写 `//! filled in a later task`。

`agent/src/protocol.rs`:
```rust
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
```

`agent/src/main.rs` 改为：
```rust
use rd_agent::config;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("rd-agent starting");
    let _ = config::config_path();
    Ok(())
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml --test protocol_roundtrip`
Expected: 3 个 protocol 往返测试通过。

- [ ] **Step 5: Commit**

```bash
git add agent/
git commit -m "feat(agent): signaling protocol serde mirror + lib/bin split"
```

---

### Task 3: 交互式登录配对（provision）

**Files:**
- Modify: `agent/src/provision.rs`
- Test: `agent/src/provision.rs`（`#[cfg(test)] mod tests` 用 wiremock）

**Interfaces:**
- Consumes: `AgentConfig`（Task 1）。
- Produces:
  - `async fn provision(server_url:&str, email:&str, password:&str, device_name:&str) -> anyhow::Result<AgentConfig>`：
    - `POST {server_url}/login` body `{email,password}` → 取 `{token}`（JWT）。
    - `POST {server_url}/devices/pair` header `Authorization: Bearer <jwt>` body `{name:device_name}` → 取 `{deviceId,token}`。
    - 返回 `AgentConfig{ server_url, device_id, device_token }`。
  - `fn prompt_credentials() -> anyhow::Result<(String,String)>`：stdin 读 email，用 `rpassword` 读 password（**此函数不写单测**，交互式；由 main 调用）。

- [ ] **Step 1: 写失败测试（wiremock 假服务端）**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path, header};

    #[tokio::test]
    async fn provision_logs_in_then_pairs() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token":"jwt-xyz"})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/devices/pair"))
            .and(header("authorization", "Bearer jwt-xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"deviceId":"dev-9","token":"devtok-9"})))
            .mount(&server).await;

        let cfg = provision(&server.uri(), "a@b.com", "pw123456", "MyPC").await.unwrap();
        assert_eq!(cfg.device_id, "dev-9");
        assert_eq!(cfg.device_token, "devtok-9");
        assert_eq!(cfg.server_url, server.uri());
    }

    #[tokio::test]
    async fn provision_errors_on_bad_login() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/login"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"error":"bad credentials"})))
            .mount(&server).await;
        let res = provision(&server.uri(), "a@b.com", "wrong", "MyPC").await;
        assert!(res.is_err());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml provision`
Expected: 编译失败（`provision` 未定义）。

- [ ] **Step 3: 实现 provision.rs**

```rust
use serde::Deserialize;
use crate::config::AgentConfig;

#[derive(Deserialize)]
struct LoginResp { token: String }
#[derive(Deserialize)]
struct PairResp {
    #[serde(rename = "deviceId")]
    device_id: String,
    token: String,
}

pub async fn provision(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> anyhow::Result<AgentConfig> {
    let client = reqwest::Client::new();

    let login = client
        .post(format!("{server_url}/login"))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await?;
    if !login.status().is_success() {
        anyhow::bail!("login failed: HTTP {}", login.status());
    }
    let jwt = login.json::<LoginResp>().await?.token;

    let pair = client
        .post(format!("{server_url}/devices/pair"))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({ "name": device_name }))
        .send()
        .await?;
    if !pair.status().is_success() {
        anyhow::bail!("pair failed: HTTP {}", pair.status());
    }
    let pair = pair.json::<PairResp>().await?;

    Ok(AgentConfig {
        server_url: server_url.to_string(),
        device_id: pair.device_id,
        device_token: pair.token,
    })
}

pub fn prompt_credentials() -> anyhow::Result<(String, String)> {
    use std::io::Write;
    print!("Email: ");
    std::io::stdout().flush()?;
    let mut email = String::new();
    std::io::stdin().read_line(&mut email)?;
    let password = rpassword::prompt_password("Password: ")?;
    Ok((email.trim().to_string(), password))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml provision`
Expected: 2 个 provision 测试通过。

- [ ] **Step 5: Commit**

```bash
git add agent/
git commit -m "feat(agent): interactive login + device pairing (provision)"
```

---

### Task 4: WebRTC 对端 + data channel 回显

> **高风险任务**：webrtc-rs API 细节多。核心目标：给定远端 offer（SDP），创建 answer，建立后对收到的 data channel 消息回显 `echo:<msg>`。用进程内两个 PeerConnection 环回做集成测试，不依赖信令服务端。

**Files:**
- Modify: `agent/src/webrtc_peer.rs`
- Test: `agent/tests/echo_loopback.rs`

**Interfaces:**
- Consumes: `IceServer`（Task 2）。
- Produces:
  - `struct PeerSession`（持有 `RTCPeerConnection`）。
  - `async fn PeerSession::new(ice_servers: Vec<IceServer>) -> anyhow::Result<PeerSession>`：建 PeerConnection，注册 `on_data_channel` 回显处理，注册 `on_ice_candidate` 回调把本地 candidate 通过传入的 sink 发出。
  - 为可测与可被信令驱动，`new` 接收一个 `on_local_ice: impl Fn(serde_json::Value)`（本地 ICE candidate 产生时回调）。
  - `async fn accept_offer(&self, offer_sdp: &str) -> anyhow::Result<String>`：set_remote_description(offer)，create_answer，set_local_description，返回 answer 的 SDP 串。
  - `async fn add_remote_ice(&self, candidate: serde_json::Value) -> anyhow::Result<()>`。
  - data channel 回显：收到 `msg` → `send_text("echo:"+msg)`。

- [ ] **Step 1: 写环回集成测试**

`agent/tests/echo_loopback.rs`:
```rust
use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// 这个测试构造一个“web 端”PeerConnection（创建 data channel + offer），
// 与被测的 PeerSession（answerer，回显）在进程内直接交换 SDP/ICE。
#[tokio::test]
async fn agent_echoes_data_channel_messages() {
    // agent 侧（被测）
    let (agent_ice_tx, mut agent_ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let agent = PeerSession::new(vec![]).await.unwrap();
    // 上面 new 内部已注册回显；但 new 的签名带 on_local_ice，见实现说明。

    // web 侧（测试内自建）
    let api = APIBuilder::new().build();
    let web = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await.unwrap());
    let dc = web.create_data_channel("echo", None).await.unwrap();

    let (got_tx, mut got_rx) = mpsc::unbounded_channel::<String>();
    let got_tx2 = got_tx.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let s = String::from_utf8(msg.data.to_vec()).unwrap();
        let _ = got_tx2.send(s);
        Box::pin(async {})
    }));

    // 交换 ICE：web→agent, agent→web
    // （测试里把 web 的 candidate 直接喂给 agent.add_remote_ice，反之亦然）
    // ... 详见实现说明：需在 web 上注册 on_ice_candidate 转发给 agent，
    //     并把 agent_ice_rx 收到的 candidate 加到 web。

    // 握手：web 造 offer → agent.accept_offer → answer 回 web
    let offer = web.create_offer(None).await.unwrap();
    web.set_local_description(offer.clone()).await.unwrap();
    let answer_sdp = agent.accept_offer(&offer.sdp).await.unwrap();
    let answer = RTCSessionDescription::answer(answer_sdp).unwrap();
    web.set_remote_description(answer).await.unwrap();

    // data channel 打开后发消息
    let dc2 = dc.clone();
    dc.on_open(Box::new(move || {
        let dc3 = dc2.clone();
        Box::pin(async move { let _ = dc3.send_text("hello".to_string()).await; })
    }));

    // 期望收到 echo:hello（设超时避免卡死）
    let got = tokio::time::timeout(std::time::Duration::from_secs(10), got_rx.recv())
        .await.expect("timed out").unwrap();
    assert_eq!(got, "echo:hello");

    let _ = agent_ice_tx; let _ = &mut agent_ice_rx;
}
```

> 实现说明（给实现者）：上面的测试骨架演示环回思路。实现 `PeerSession::new(ice_servers, on_local_ice)` 时，`on_local_ice` 用于把 agent 的本地 candidate 送出；测试里应把两端的 `on_ice_candidate` 互相转发（每端产生的 candidate 通过对端的 `add_ice_candidate` 加入）。webrtc-rs 里 `on_ice_candidate` 回调收到 `Option<RTCIceCandidate>`，`candidate.to_json()` 得到可序列化结构。允许实现者按 webrtc-rs 实际 API 微调测试的 ICE 转发接线，但**断言 `echo:hello` 不可改**。若纯环回下 trickle ICE 接线复杂，可在两端配置为非 trickle（等 `gathering complete` 后一次性交换完整 SDP，SDP 内已含 candidate），从而简化：那样测试无需单独转发 ICE，只交换 offer/answer 即可。**推荐用后者（等待 ICE gathering 完成）**。

- [ ] **Step 2: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml --test echo_loopback`
Expected: 编译失败（`PeerSession` 未定义）。

- [ ] **Step 3: 实现 webrtc_peer.rs**

```rust
use std::sync::Arc;
use anyhow::Result;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use crate::protocol::IceServer;

pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
}

fn to_rtc_ice(servers: Vec<IceServer>) -> Vec<RTCIceServer> {
    servers.into_iter().map(|s| {
        let urls = match s.urls {
            serde_json::Value::String(u) => vec![u],
            serde_json::Value::Array(a) => a.into_iter()
                .filter_map(|v| v.as_str().map(String::from)).collect(),
            _ => vec![],
        };
        RTCIceServer {
            urls,
            username: s.username.unwrap_or_default(),
            credential: s.credential.unwrap_or_default(),
            ..Default::default()
        }
    }).collect()
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
    pub async fn new(ice_servers: Vec<IceServer>) -> Result<PeerSession> {
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

        // agent 是 answerer：远端创建 data channel，这里在 on_data_channel 里接手并回显
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            wire_echo(dc);
            Box::pin(async {})
        }));

        Ok(PeerSession { pc })
    }

    /// 处理远端 offer，返回本地 answer 的 SDP。等待 ICE gathering 完成，
    /// 使 answer 内含全部 candidate（非 trickle），简化信令。
    pub async fn accept_offer(&self, offer_sdp: &str) -> Result<String> {
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
        self.pc.set_remote_description(offer).await?;
        let answer = self.pc.create_answer(None).await?;
        let mut gather_complete = self.pc.gathering_complete_promise().await;
        self.pc.set_local_description(answer).await?;
        let _ = gather_complete.recv().await;
        let local = self.pc.local_description().await
            .ok_or_else(|| anyhow::anyhow!("no local description after gathering"))?;
        Ok(local.sdp)
    }

    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }
}
```

> 若 Step 1 采用"等待 gathering 完成"的非 trickle 方案（推荐），测试里 web 端也要在 `set_local_description(offer)` 后等待其 `gathering_complete_promise`，再把含 candidate 的完整 `local_description().sdp` 作为 offer 传给 `accept_offer`；answer 同理已含 candidate。这样两端都不需要单独交换 ICE。实现者据此把 Step 1 测试的 ICE 转发部分删掉，改为 gathering 等待。

- [ ] **Step 4: 跑测试确认通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml --test echo_loopback`
Expected: `agent_echoes_data_channel_messages` 通过（收到 `echo:hello`）。可能需数秒建连。

- [ ] **Step 5: Commit**

```bash
git add agent/
git commit -m "feat(agent): webrtc peer with data-channel echo (loopback tested)"
```

---

### Task 5: 信令 WS 客户端

**Files:**
- Modify: `agent/src/signaling.rs`
- Test: `agent/src/signaling.rs`（`#[cfg(test)] mod tests`，用进程内 WS echo/stub server）

**Interfaces:**
- Consumes: `SignalingMessage`（Task 2）、`PeerSession`（Task 4）、`AgentConfig`（Task 1）。
- Produces:
  - `async fn run_agent(config: AgentConfig) -> anyhow::Result<()>`：连 `ws://<host>`（从 `server_url` 推导 ws scheme），发送 `agent-online{token}`，进入消息循环：
    - `incoming{session_id,ice_servers,..}` → 记住 session_id，建 `PeerSession`（暂存，等 offer）。
    - `sdp{session_id, sdp:{type:"offer",..}}` → `accept_offer` → 回发 `sdp{session_id, sdp:{type:"answer",..}}`。
    - `error{..}` → 记录日志。
  - 为可测：抽出纯函数 `fn ws_url_from(server_url:&str) -> String`（`http→ws`, `https→wss`）并单测；消息处理逻辑抽为 `async fn handle_message(msg, state, out_tx)`，用内存 channel 断言输出，无需真 WebRTC（对 offer 处理可在有真实 PeerSession 时于 Task 6 的 e2e 覆盖，本 Task 单测聚焦 url 推导 + agent-online 首发 + error 处理路径）。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ws_url_http_to_ws() {
        assert_eq!(ws_url_from("http://127.0.0.1:8080"), "ws://127.0.0.1:8080");
        assert_eq!(ws_url_from("https://x.example:443"), "wss://x.example:443");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml signaling`
Expected: 编译失败（`ws_url_from` 未定义）。

- [ ] **Step 3: 实现 signaling.rs**

```rust
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use crate::config::AgentConfig;
use crate::protocol::{SignalingMessage, SdpDesc};
use crate::webrtc_peer::PeerSession;

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

    while let Some(item) = read.next().await {
        let msg = match item {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };
        let parsed: SignalingMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => { tracing::warn!("bad signaling msg: {e}"); continue; }
        };
        match parsed {
            SignalingMessage::Incoming { session_id, ice_servers, .. } => {
                let peer = PeerSession::new(ice_servers).await?;
                current = Some((session_id, peer));
                tracing::info!("incoming session accepted, awaiting offer");
            }
            SignalingMessage::Sdp { session_id, sdp } if sdp.kind == "offer" => {
                if let Some((sid, peer)) = &current {
                    if *sid == session_id {
                        let answer_sdp = peer.accept_offer(&sdp.sdp).await?;
                        let reply = serde_json::to_string(&SignalingMessage::Sdp {
                            session_id,
                            sdp: SdpDesc { kind: "answer".into(), sdp: answer_sdp },
                        })?;
                        write.send(Message::Text(reply)).await?;
                    }
                }
            }
            SignalingMessage::Error { code, message } => {
                tracing::error!("signaling error {code}: {message}");
            }
            _ => {}
        }
    }
    Ok(())
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml signaling`
Expected: `ws_url_http_to_ws` 通过。

- [ ] **Step 5: Commit**

```bash
git add agent/
git commit -m "feat(agent): signaling websocket client + session loop"
```

---

### Task 6: main 接线 + 端到端手动冒烟

**Files:**
- Modify: `agent/src/main.rs`
- Create: `agent/README.md`（如何运行 + 手动冒烟步骤）

**Interfaces:**
- Consumes: `config`、`provision`、`signaling`（前序 Task）。
- Produces: 可运行的 `rd-agent` 二进制：有配置则直接上线；无配置则交互式登录配对并保存，再上线。

- [ ] **Step 1: 实现 main.rs（异步入口）**

```rust
use rd_agent::{config::AgentConfig, provision, signaling};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = match AgentConfig::load()? {
        Some(c) => {
            tracing::info!("loaded config for device {}", c.device_id);
            c
        }
        None => {
            let server = std::env::var("RD_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
            println!("No config found. Log in to pair this device.");
            let (email, password) = provision::prompt_credentials()?;
            let name = hostname_or("rd-agent");
            let cfg = provision::provision(&server, &email, &password, &name).await?;
            cfg.save()?;
            println!("Paired as device {}", cfg.device_id);
            cfg
        }
    };

    signaling::run_agent(cfg).await
}

fn hostname_or(fallback: &str) -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| fallback.to_string())
}
```

- [ ] **Step 2: 确认整体编译 + 全部测试通过**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --manifest-path agent/Cargo.toml && cargo test --manifest-path agent/Cargo.toml`
Expected: 编译通过；config(2) + protocol(3) + provision(2) + signaling(1) + echo_loopback(1) 全绿。

- [ ] **Step 3: 写 agent/README.md（运行 + 手动冒烟）**

内容要点（用实际命令）：
- 前置：`export PATH="$HOME/.cargo/bin:$PATH"`；先起 Node 服务端 `JWT_SECRET=dev npm run dev -w @rd/server`；先在 web/REST 用同一账号注册（`curl -XPOST .../register`）。
- 首次运行 `cargo run --manifest-path agent/Cargo.toml`：输入邮箱/密码 → 配对 → 保存配置 → 打印 "agent online"。
- 手动 e2e（与 Plan 2b 的 web 端联调前的临时验证）：可用一个最小 ws 脚本模拟 web 端发 `connect`+offer，确认 agent 回 `answer`；或直接留待 Plan 2b 用真 React 端联调。
- 说明配置文件位置与 `RD_AGENT_CONFIG`/`RD_SERVER_URL` 覆盖。

- [ ] **Step 4: Commit**

```bash
git add agent/
git commit -m "feat(agent): wire main (load-or-provision then run) + agent README"
```

---

## Self-Review

**Spec coverage（对照设计 §4.1 Rust Agent 与 §9 开发顺序）：**
- §4.1 `auth`（首次输入账号/密码换 device token 存本地）→ Task 3 + Task 1 + Task 6 ✅
- §4.1 `signaling`（长连 WS，收发 SDP/ICE）→ Task 5 ✅
- §4.1 `webrtc`（webrtc-rs，data channel）→ Task 4 ✅（回显；抓屏/键鼠属 Plan 3/4）
- §4.4 共享协议 Rust 侧实现 → Task 2 ✅
- Plan 2 里程碑"data channel 回显"→ Task 4（环回验证）+ Task 5（信令驱动）✅
- **不在本计划**：React 控制端、coturn、真浏览器 e2e → Plan 2b。`capture`/`input` → Plan 3/4。

**Placeholder scan：** Task 2 引入的 `provision.rs`/`signaling.rs`/`webrtc_peer.rs` 占位文件在 Task 3/4/5 被充实，非最终占位；已注明。无 TBD/TODO 遗留。

**Type consistency：** `SignalingMessage`/`SdpDesc`/`IceServer` 全程一致；`PeerSession::new`/`accept_offer` 在 Task 4 定义、Task 5 使用；`AgentConfig` 在 Task 1 定义、Task 3/5/6 使用；`run_agent(AgentConfig)` 签名一致。

**已知风险（给执行者）：**
- Task 4 webrtc-rs API 版本差异可能需要微调（`RTCIceServer` 字段、`on_data_channel`/`on_message` 闭包签名、`gathering_complete_promise`）。断言（`echo:hello`）与接口名不可变，接线细节允许按实际 API 调整并在报告说明。
- Task 5 的信令循环对 offer 的处理未在本 crate 单测（需真 PeerConnection），由 Task 6 README 的手动冒烟与 Plan 2b 的 e2e 覆盖；已在计划中显式说明，非遗漏。

## Execution Handoff

计划保存到 `docs/superpowers/plans/2026-07-01-plan2a-rust-agent.md`。
