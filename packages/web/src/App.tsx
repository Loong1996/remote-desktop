import { useState } from "react";
import type { Device } from "@rd/protocol";
import { LoginPage } from "./pages/LoginPage.js";
import { DevicesPage } from "./pages/DevicesPage.js";
import { SessionView } from "./pages/SessionView.js";

const TOKEN_KEY = "rd.token";

export function App() {
  const [token, setToken] = useState<string | null>(() => localStorage.getItem(TOKEN_KEY));
  const [activeDevice, setActiveDevice] = useState<Device | null>(null);

  function authenticate(t: string) {
    localStorage.setItem(TOKEN_KEY, t);
    setToken(t);
  }

  function logout() {
    localStorage.removeItem(TOKEN_KEY);
    setActiveDevice(null);
    setToken(null);
  }

  if (!token) {
    return <LoginPage onAuthenticated={authenticate} />;
  }

  if (activeDevice) {
    return <SessionView token={token} device={activeDevice} onExit={() => setActiveDevice(null)} />;
  }

  return (
    <DevicesPage
      token={token}
      onSelectDevice={setActiveDevice}
      onLogout={logout}
    />
  );
}
