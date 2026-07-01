import { useState } from "react";
import { login, register } from "../api.js";

export interface LoginPageProps {
  /** Called with the user JWT once login/register succeeds. */
  onAuthenticated: (token: string) => void;
}

export function LoginPage({ onAuthenticated }: LoginPageProps) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(mode: "login" | "register") {
    setBusy(true);
    setError(null);
    try {
      const token = mode === "login"
        ? await login(email, password)
        : await register(email, password);
      onAuthenticated(token);
    } catch (e) {
      setError(e instanceof Error ? e.message : "request failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ maxWidth: 360, margin: "10vh auto", fontFamily: "system-ui" }}>
      <h1>Remote Desktop</h1>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit("login");
        }}
      >
        <label style={{ display: "block", marginBottom: 8 }}>
          Email
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            style={{ display: "block", width: "100%" }}
          />
        </label>
        <label style={{ display: "block", marginBottom: 12 }}>
          Password
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            style={{ display: "block", width: "100%" }}
          />
        </label>
        <button type="submit" disabled={busy}>
          Log in
        </button>
        <button type="button" disabled={busy} onClick={() => void submit("register")} style={{ marginLeft: 8 }}>
          Register
        </button>
      </form>
      {error && <p style={{ color: "crimson" }} role="alert">{error}</p>}
    </div>
  );
}
