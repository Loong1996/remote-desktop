import { randomUUID } from "node:crypto";

export interface Conn { send(data: string): void; close(): void; }

interface Session { id: string; web: Conn; agent: Conn; }

export class Registry {
  private agents = new Map<string, Conn>();        // deviceId -> agent conn
  private agentByConn = new Map<Conn, string>();   // reverse
  private sessions = new Map<string, Session>();   // sessionId -> session

  setAgentOnline(deviceId: string, conn: Conn): void {
    this.agents.set(deviceId, conn);
    this.agentByConn.set(conn, deviceId);
  }
  isOnline(deviceId: string): boolean { return this.agents.has(deviceId); }
  getAgent(deviceId: string): Conn | undefined { return this.agents.get(deviceId); }

  /** Remove a connection: drop its agent mapping and any sessions it was part
   *  of, returning the surviving peer of each removed session so callers can
   *  notify them (peer-left). */
  remove(conn: Conn): { sessionId: string; peer: Conn }[] {
    const deviceId = this.agentByConn.get(conn);
    if (deviceId) {
      this.agents.delete(deviceId);
      this.agentByConn.delete(conn);
    }
    const affected: { sessionId: string; peer: Conn }[] = [];
    for (const [sid, s] of this.sessions) {
      if (s.web === conn || s.agent === conn) {
        affected.push({ sessionId: sid, peer: s.web === conn ? s.agent : s.web });
        this.sessions.delete(sid);
      }
    }
    return affected;
  }

  createSession(web: Conn, agent: Conn): string {
    const id = randomUUID();
    this.sessions.set(id, { id, web, agent });
    return id;
  }
  peerOf(conn: Conn, sessionId: string): Conn | undefined {
    const s = this.sessions.get(sessionId);
    if (!s) return undefined;
    if (s.web === conn) return s.agent;
    if (s.agent === conn) return s.web;
    return undefined;
  }
  dropSession(sessionId: string): void { this.sessions.delete(sessionId); }
}
