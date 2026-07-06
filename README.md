# Remote Desktop

[English](./README.md) · [中文](./README.zh.md)

A cross-platform remote desktop you control from a **browser** — no client install. Open a page, log in, click an online device, and you're driving it. The controlled machine runs a small native **Rust agent**. Screen and input travel **peer-to-peer over WebRTC**; the server only brokers the connection (signaling) and never sees your media.

Because the control end is just a web page, it works anywhere a modern browser does (desktop today, mobile/iOS with no extra app). Native control clients can be added later.

## Features

- **Live screen streaming + mouse/keyboard control**, low latency over WebRTC.
- **Accounts + device list** — register, log in, see your devices, click an online one to connect.
- **Clipboard sync** — off / one-way / two-way, selectable per session (defaults to off for privacy).
- **Live quality control** — bitrate slider + presets (流畅/均衡/高清/超清/原画), applied **without reconnecting**.
- **Resolution hot-switch** — 720p / display-logical / Retina, switched live (~0.5 s stall, session stays up).
- **Special key combos** — Copy, Paste, Spotlight, App Switcher, Mission Control, Screenshot, sent as proper chords.
- **Connection stats overlay** — fps, kbps, RTT, and resolution.
- **Fullscreen / fill-window** viewing.
- **Hardware encoding on macOS** (VideoToolbox H.264) with an openh264 software fallback everywhere.
- **P2P transport** with NAT traversal (STUN/TURN via coturn). Configurable relay policy per session: `direct-only` | `relay-fallback` (default) | `force-relay`.

Not yet built: audio, file transfer, multi-monitor, Windows/Linux hardware encoding, VP9/AV1 codecs.

## Architecture

```
┌──────────────────┐        ┌────────────────────────┐        ┌──────────────────┐
│ React web (ctrl) │◄──────►│   Node/TS server        │◄──────►│  Rust agent       │
│ browser          │  HTTPS │  · REST (accounts/devs) │   WS   │  (mac / Win / …)  │
│                  │  + WS  │  · WebSocket signaling  │        │                  │
└────────┬─────────┘        └────────────────────────┘        └────────┬─────────┘
         │                                                              │
         │      WebRTC (P2P first, TURN relay on fallback)              │
         │   • video track: H.264 (VideoToolbox / openh264)            │
         │   • "input" data channel: mouse/keyboard events             │
         │   • "control" data channel: clipboard · quality · resolution │
         └──────────────────────────────────────────────────────────────┘
                                     │
                              ┌──────┴───────┐
                              │  coturn       │  STUN/TURN
                              └──────────────┘
```

Signaling (SDP/ICE exchange) goes through the server; once connected, screen + input are peer-to-peer. Full design docs live in [docs/superpowers/specs/](docs/superpowers/specs/) and plans in [docs/superpowers/plans/](docs/superpowers/plans/).

## Monorepo layout

```
packages/
  protocol/   # @rd/protocol — shared TS wire types + runtime validation
              #   signaling, input events, control messages (clipboard/quality/resolution)
  server/     # @rd/server — Fastify (REST) + ws (signaling) + better-sqlite3
              #   accounts/JWT, device list/pair, WebSocket SDP/ICE relay
  web/        # @rd/web — React (Vite) control end: login, device list, session view
agent/        # rd-agent — Rust: screen capture, H.264 encode, input injection, WebRTC answerer
infra/coturn/ # coturn (STUN/TURN) docker-compose + config
docs/         # design specs + implementation plans
```

## Tech stack

- **Protocol / server / web:** TypeScript (Node ≥ 20). Fastify, `ws`, better-sqlite3, bcryptjs, jsonwebtoken; React + Vite. Tests with Vitest.
- **Agent:** Rust (edition 2021). `webrtc-rs`, ScreenCaptureKit (macOS) / `xcap` (Windows) capture, `openh264` + VideoToolbox H.264 encode, `enigo` for input injection.
- **Transport / NAT:** WebRTC + coturn.

## Getting started

Prerequisites: **Node.js ≥ 20** and **Rust** (stable, via rustup). coturn is only needed for connections that can't go direct (WAN); LAN works without it.

```bash
npm install          # install workspace deps (protocol, server, web)
npm test             # all TS tests (Vitest)
npm run typecheck    # type-check protocol + server (web is checked via its build)
npm run -w @rd/web build   # type-check + build the web client
```

Agent (Rust):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test  --manifest-path agent/Cargo.toml
cargo build --release --manifest-path agent/Cargo.toml
```

### Run it (three processes)

1. **Server** — pick 5181 so the web client finds it by default:

   ```bash
   JWT_SECRET=change-me PORT=5181 npm run dev -w @rd/server
   ```

2. **Web control end:**

   ```bash
   npm run dev -w @rd/web        # Vite dev server on http://localhost:5173
   ```

   Open the page, **register an account**, and log in. (The web client calls the
   server at `<page-host>:5181` by default; override with `VITE_SERVER_URL`.)

3. **Agent** on the machine to be controlled. First run pairs it to your account:

   ```bash
   RD_SERVER_URL=http://127.0.0.1:5181 \
     ./agent/target/release/rd-agent          # prompts for your email/password, saves config
   ```

   After pairing, the device appears **online** in the web device list — click **Connect**.

   > **macOS:** grant the agent's terminal **Screen Recording** and **Accessibility**
   > (System Settings → Privacy & Security). Without them the agent still connects,
   > but video is blank and/or input injection is disabled (it logs a warning).

## Configuration

**Server** (env):

| Var | Default | Meaning |
|-----|---------|---------|
| `PORT` | `8080` | HTTP/WS port (use `5181` to match the web client's default) |
| `JWT_SECRET` | *(required outside tests)* | JWT signing secret (HS256) |
| `RELAY_POLICY` | `relay-fallback` | `direct-only` \| `relay-fallback` \| `force-relay` |
| `ICE_SERVERS` | Google STUN | JSON array of ICE servers |
| `DB_PATH` | `remote-desktop.db` | SQLite file path |

**Agent** (env):

| Var | Default | Meaning |
|-----|---------|---------|
| `RD_AGENT_CONFIG` | platform config dir | Path to the JSON config (`server_url`, `device_id`, `device_token`) |
| `RD_SERVER_URL` | `http://127.0.0.1:8080` | Server used for first-run pairing |
| `RD_VIDEO_SOURCE` | `screen` | `screen` (real capture) or `testpattern` (synthetic, no permissions) |
| `RD_VIDEO_ENCODER` | *(auto)* | Set to `openh264` to force software encoding (skip VideoToolbox) |
| `RUST_LOG` | — | Tracing filter, e.g. `info` |

**Web** (build-time env): `VITE_SERVER_URL` (full server URL, wins) or `VITE_SERVER_PORT` (default `5181`).

## Security

This is **not hardened for public exposure.** Auth is a basic email/password + JWT; there is no 2FA, rate limiting, or device-approval flow. Run it on a trusted LAN or behind a VPN. If you port-forward it to the internet, use a strong password and close the forward when you're done.

## Roadmap

| Milestone | Status |
|-----------|--------|
| Shared protocol (TS) + Node server (accounts, devices, signaling) | ✅ |
| WebRTC media + Rust agent + React web client (end-to-end connect) | ✅ |
| Mouse/keyboard injection (raw keycode mapping) | ✅ |
| Screen capture + H.264 video (openh264) | ✅ |
| Clipboard sync · live quality/bitrate · resolution hot-switch · combos · stats | ✅ |
| macOS hardware encoding (VideoToolbox) + real-time frame pacing + PLI keyframes | ✅ |
| Windows/Linux hardware encoding (NVENC/QSV/AMF) | ⏳ |
| VP9/AV1 codecs + codec negotiation | ⏳ |
| Audio · file transfer · multi-monitor | ⏳ |

## License

TBD.
