import { useCallback, useEffect, useState } from "react";
import type { Device } from "@rd/protocol";
import { listDevices, pairDevice } from "../api.js";

export interface DevicesPageProps {
  token: string;
  onSelectDevice: (device: Device) => void;
  onLogout: () => void;
}

export function DevicesPage({ token, onSelectDevice, onLogout }: DevicesPageProps) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pairName, setPairName] = useState("");
  const [pairedToken, setPairedToken] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setDevices(await listDevices(token));
    } catch (e) {
      setError(e instanceof Error ? e.message : "failed to load devices");
    } finally {
      setLoading(false);
    }
  }, [token]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function pair() {
    if (!pairName) return;
    setError(null);
    try {
      const { token: deviceToken } = await pairDevice(token, pairName);
      setPairedToken(deviceToken);
      setPairName("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "failed to pair device");
    }
  }

  return (
    <div style={{ maxWidth: 560, margin: "5vh auto", fontFamily: "system-ui" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <h1>Devices</h1>
        <button onClick={onLogout}>Log out</button>
      </div>

      {error && <p style={{ color: "crimson" }} role="alert">{error}</p>}
      {loading ? <p>Loading…</p> : (
        <ul style={{ listStyle: "none", padding: 0 }}>
          {devices.length === 0 && <li style={{ color: "#888" }}>No devices yet.</li>}
          {devices.map((d) => (
            <li key={d.id} style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 0" }}>
              <span
                aria-label={d.online ? "online" : "offline"}
                title={d.online ? "online" : "offline"}
                style={{
                  display: "inline-block", width: 10, height: 10, borderRadius: "50%",
                  background: d.online ? "#22c55e" : "#9ca3af",
                }}
              />
              <span style={{ flex: 1 }}>{d.name}</span>
              <button disabled={!d.online} onClick={() => onSelectDevice(d)}>
                Connect
              </button>
            </li>
          ))}
        </ul>
      )}

      <section style={{ marginTop: 24, borderTop: "1px solid #eee", paddingTop: 16 }}>
        <h2>Pair a new device</h2>
        <input
          placeholder="Device name"
          value={pairName}
          onChange={(e) => setPairName(e.target.value)}
        />
        <button onClick={() => void pair()} disabled={!pairName} style={{ marginLeft: 8 }}>
          Pair
        </button>
        {pairedToken && (
          <div style={{ marginTop: 12 }}>
            <p>Device token (enter this on the agent to bring it online):</p>
            <code style={{ userSelect: "all", wordBreak: "break-all" }}>{pairedToken}</code>
          </div>
        )}
      </section>
    </div>
  );
}
