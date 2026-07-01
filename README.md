# Remote Desktop

[English](./README.md) · [中文](./README.zh.md)

A cross-platform remote desktop system for controlling a machine over the internet. A browser is the control end today (works everywhere, including future iOS); native clients can be added later. The controlled machine runs a native agent. Screen and input travel peer-to-peer over WebRTC; a signaling server only brokers the connection.

> **Status:** Plan 1 (shared protocol + Node signaling server) is complete and merged. WebRTC media, the Rust agent, and the React web client are in progress (Plan 2+).

## Features

- **MVP scope:** live screen streaming + mouse/keyboard control (built incrementally).
- **Account + device list:** log in, see your devices, click an online one to connect.
- **P2P transport:** WebRTC with NAT traversal (STUN/TURN via coturn). Media never passes through the server.
- **Configurable relay policy:** `direct-only` | `relay-fallback` (default) | `force-relay`, per session.
- **Cross-platform agent:** Windows / macOS / Linux (Rust).

Later iterations: multi-monitor, audio, file transfer, clipboard sync, multi-controller sessions.

## Architecture

```
┌─────────────────┐        ┌──────────────────────┐        ┌─────────────────┐
│  React Web (ctrl)│◄──────►│   Node/TS server       │◄──────►│  Rust agent      │
│  browser         │  HTTPS │  · REST (accounts/devs)│   WS   │  (Win/mac/Linux) │
│                 │  + WS  │  · WebSocket signaling  │        │                 │
└────────┬────────┘        └──────────────────────┘        └────────┬────────┘
         │                                                          │
         │           WebRTC (P2P first, TURN relay on fallback)     │
         └──────────── screen (video track) + input (data channel) ─┘
                                     │
                              ┌──────┴───────┐
                              │  coturn       │  STUN/TURN
                              └──────────────┘
```

Signaling goes through the server; media is peer-to-peer. See [docs/superpowers/specs/](docs/superpowers/specs/) for the full design.

## Monorepo layout

```
packages/
  protocol/   # @rd/protocol — shared TS types + runtime validation
              #   signaling messages, input events (language-neutral JSON contract)
  server/     # @rd/server — Fastify (REST) + ws (signaling) + better-sqlite3
              #   accounts/JWT, device list/pair, WebSocket SDP/ICE relay
docs/         # design specs + implementation plans
```

Planned: `packages/web` (React control end), `agent/` (Rust agent), `infra/` (coturn).

## Tech stack

- **Protocol / server / web:** TypeScript (Node ≥20). Fastify, `ws`, better-sqlite3, bcryptjs, jsonwebtoken. React (control end).
- **Agent:** Rust (`webrtc-rs`, screen capture + H.264, `enigo` for input injection).
- **Transport / NAT:** WebRTC + coturn.
- **Tests:** vitest.

## Getting started

Requires Node.js ≥ 20.

```bash
npm install          # install workspace deps
npm test             # run all tests (vitest)
npm run typecheck    # type-check every package (strict)
```

Run the server (dev):

```bash
# JWT_SECRET is required outside tests
JWT_SECRET=change-me npm run dev -w @rd/server
```

Server configuration (env):

| Var | Default | Meaning |
|-----|---------|---------|
| `PORT` | `8080` | HTTP/WS port |
| `JWT_SECRET` | *(required in prod)* | JWT signing secret (HS256) |
| `RELAY_POLICY` | `relay-fallback` | `direct-only` \| `relay-fallback` \| `force-relay` |
| `ICE_SERVERS` | Google STUN | JSON array of ICE servers |
| `DB_PATH` | `remote-desktop.db` | SQLite file path |

## Roadmap

| Plan | Milestone | Status |
|------|-----------|--------|
| 1 | Shared protocol (TS) + Node server (accounts, devices, signaling) | ✅ Done |
| 2 | WebRTC empty connection (minimal React ctrl + Rust agent + coturn), data-channel echo | 🚧 In progress |
| 3 | Mouse/keyboard injection | ⏳ |
| 4 | Screen capture + H.264 video | ⏳ |

## License

TBD.
