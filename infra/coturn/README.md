# coturn (STUN/TURN) — infra

Deployable [coturn](https://github.com/coturn/coturn) STUN/TURN server for the
remote-desktop project's NAT traversal. Exposes STUN/TURN on port `3478`
(UDP + TCP) with a static long-term credential.

- **User:** `rduser:rdpass`
- **Realm:** `remote.desktop`
- **Port:** `3478` (udp/tcp)

## Start

```bash
docker compose -f infra/coturn/docker-compose.yml up -d
```

The compose file uses `network_mode: host`, so the container shares the host
network stack and coturn binds `3478` directly on the host. This is convenient
for local P2P testing (STUN/host candidates reach the server without extra port
mapping).

## Verify

Check the logs — you should see coturn listening on `3478`:

```bash
docker compose -f infra/coturn/docker-compose.yml logs -f
```

Look for lines confirming the listening port (e.g. `Listener opened on ... port 3478`)
and the configured realm.

Optional deeper checks:

- **`turnutils_uclient`** (ships with coturn) to exercise a TURN allocation:
  ```bash
  turnutils_uclient -u rduser -w rdpass -v 127.0.0.1
  ```
- A browser **Trickle ICE** page
  (<https://webrtc.github.io/samples/src/content/peerconnection/trickle-ice/>)
  configured with the STUN/TURN URLs below.

## Server alignment — `ICE_SERVERS`

The server's `ICE_SERVERS` must match this deployment. For local (same-host)
testing:

```
ICE_SERVERS='[{"urls":"stun:127.0.0.1:3478"},{"urls":"turn:127.0.0.1:3478","username":"rduser","credential":"rdpass"}]'
```

`network_mode: host` makes local P2P straightforward. For a real deployment,
replace `127.0.0.1` with the server's public IP / hostname.

## What TURN is for here

Plan 2b's automated e2e test connects on the **same host**, so STUN + host
candidates are enough to establish the peer connection locally — TURN is not
exercised by the milestone test. TURN (relayed candidates) is primarily there
for **real cross-NAT connections between physical machines**, where a direct P2P
path can't be found.

## Production notes

This config is intentionally minimal for local development. Before deploying to
the internet you should:

- **Auth:** Switch from the static `user=rduser:rdpass` (long-term credential
  mechanism) to `use-auth-secret` + `static-auth-secret` and issue short-lived,
  time-limited TURN credentials. Do **not** ship the hard-coded `rduser:rdpass`
  to production.
- **Public IP:** Configure a reachable public IP (`external-ip` / `relay-ip`) so
  relayed candidates are valid across the internet.
- **TLS/DTLS:** This config sets `no-tls` / `no-dtls`. Enable TLS/DTLS with a
  real certificate (`cert` / `pkey`) and expose the TLS listening port for
  `turns:` / `stuns:`.
- **Firewall:** Open `3478` (udp/tcp), the TLS port, and the relay port range to
  the internet as needed.
