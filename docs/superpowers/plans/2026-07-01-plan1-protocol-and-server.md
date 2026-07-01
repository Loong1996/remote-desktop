# Plan 1: 共享协议 + Node 服务端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭好后端基座——一份 TypeScript 共享协议定义，加一个 Node/TS 服务端，提供账号注册/登录(JWT)、设备列表/配对、以及 WebSocket 信令转发，全部有测试覆盖。

**Architecture:** npm workspaces 单仓多包。`packages/protocol` 定义信令消息与输入事件的 TS 类型和运行时校验；`packages/server` 用 Fastify(REST) + ws(WebSocket) + better-sqlite3(存储) 实现业务与信令中转。服务端只转发 SDP/ICE，不解析媒体。

**Tech Stack:** Node.js ≥20、TypeScript、Fastify、`ws`、better-sqlite3、jsonwebtoken、bcryptjs、vitest、tsx。

## Global Constraints

- Node.js 版本 ≥ 20（使用内置 `node:test`? 否——统一用 vitest）。
- 语言：TypeScript，`strict: true`。
- 密码哈希：bcryptjs（纯 JS，避免 Windows 原生编译）。
- JWT：HS256，密钥从环境变量 `JWT_SECRET` 读取，测试用固定值。
- 存储：better-sqlite3；测试用 `:memory:` 数据库。
- 所有信令消息为 JSON，字段名与本计划 Task 2 定义**逐字一致**；服务端不解析媒体内容。
- 坐标在输入事件中用 0~1 相对值（`number`，含边界 0 和 1）。
- 每个 Task 结束必须测试通过后再 commit（项目纪律）。
- 提交信息用英文，结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## File Structure

```
remote-desktop/
  package.json                      # npm workspaces 根
  tsconfig.base.json                # 共享 TS 配置
  packages/
    protocol/
      package.json
      tsconfig.json
      src/index.ts                  # 导出所有类型 + 校验
      src/signaling.ts              # 信令消息类型 + parseSignalingMessage
      src/input.ts                  # 输入事件类型 + parseInputEvent
      test/signaling.test.ts
      test/input.test.ts
    server/
      package.json
      tsconfig.json
      vitest.config.ts
      src/config.ts                 # 环境配置(端口/JWT密钥/中继策略/TURN)
      src/db.ts                     # SQLite 连接 + schema 初始化
      src/repo/users.ts             # 用户仓储
      src/repo/devices.ts           # 设备仓储
      src/auth.ts                   # 密码哈希 + JWT 签发/校验
      src/routes/auth.ts            # /register /login
      src/routes/devices.ts         # /devices /devices/pair
      src/signaling/registry.ts     # 在线连接表 + sessionId 分配
      src/signaling/hub.ts          # WebSocket 信令转发核心
      src/app.ts                    # 组装 Fastify + ws
      src/index.ts                  # 进程入口
      test/*.test.ts
```

**责任边界：** `protocol` 无任何 Node 依赖，纯类型+校验，可被 web/agent 复用。`repo/*` 只碰数据库，`auth.ts` 只碰哈希/JWT，`routes/*` 只做 HTTP 编排，`signaling/*` 只做连接与转发——各自可独立测试。

---

### Task 1: 仓库与工具链脚手架

**Files:**
- Create: `package.json`, `tsconfig.base.json`
- Create: `packages/protocol/package.json`, `packages/protocol/tsconfig.json`
- Create: `packages/protocol/src/index.ts`
- Test: `packages/protocol/test/smoke.test.ts`

**Interfaces:**
- Produces: 可运行的 `npm test`（vitest）跨 workspace；`@rd/protocol` 包名。

- [ ] **Step 1: 写根 `package.json`**

```json
{
  "name": "remote-desktop",
  "private": true,
  "workspaces": ["packages/*"],
  "scripts": {
    "test": "vitest run",
    "typecheck": "tsc -b"
  },
  "devDependencies": {
    "typescript": "^5.5.0",
    "vitest": "^2.0.0",
    "tsx": "^4.16.0"
  }
}
```

- [ ] **Step 2: 写 `tsconfig.base.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "declaration": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true
  }
}
```

- [ ] **Step 3: 写 protocol 包脚手架**

`packages/protocol/package.json`:
```json
{
  "name": "@rd/protocol",
  "version": "0.0.0",
  "type": "module",
  "main": "src/index.ts",
  "exports": { ".": "./src/index.ts" }
}
```

`packages/protocol/tsconfig.json`:
```json
{ "extends": "../../tsconfig.base.json", "include": ["src", "test"] }
```

`packages/protocol/src/index.ts`:
```ts
export const PROTOCOL_VERSION = 1;
```

- [ ] **Step 4: 写冒烟测试**

`packages/protocol/test/smoke.test.ts`:
```ts
import { expect, test } from "vitest";
import { PROTOCOL_VERSION } from "../src/index.js";

test("protocol version is 1", () => {
  expect(PROTOCOL_VERSION).toBe(1);
});
```

- [ ] **Step 5: 安装依赖并跑测试**

Run: `npm install && npm test`
Expected: 1 passed（smoke test 通过）

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: scaffold npm workspaces + protocol package"
```

---

### Task 2: 信令消息类型与校验

**Files:**
- Create: `packages/protocol/src/signaling.ts`
- Modify: `packages/protocol/src/index.ts`
- Test: `packages/protocol/test/signaling.test.ts`

**Interfaces:**
- Produces:
  - 类型 `SignalingMessage`（下述可辨识联合）。
  - `parseSignalingMessage(raw: unknown): SignalingMessage`——非法输入抛 `Error`。
  - `RelayPolicy = "direct-only" | "relay-fallback" | "force-relay"`。

- [ ] **Step 1: 写失败测试**

`packages/protocol/test/signaling.test.ts`:
```ts
import { expect, test } from "vitest";
import { parseSignalingMessage } from "../src/signaling.js";

test("parses a connect message", () => {
  const msg = parseSignalingMessage({ type: "connect", deviceId: "dev-1" });
  expect(msg).toEqual({ type: "connect", deviceId: "dev-1" });
});

test("parses an sdp relay message", () => {
  const msg = parseSignalingMessage({
    type: "sdp", sessionId: "s1", sdp: { type: "offer", sdp: "v=0..." },
  });
  expect(msg.type).toBe("sdp");
});

test("rejects unknown type", () => {
  expect(() => parseSignalingMessage({ type: "nope" })).toThrow();
});

test("rejects connect without deviceId", () => {
  expect(() => parseSignalingMessage({ type: "connect" })).toThrow();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -w @rd/protocol`
Expected: FAIL（`parseSignalingMessage` 未定义 / 模块不存在）

- [ ] **Step 3: 实现 signaling.ts**

```ts
export type RelayPolicy = "direct-only" | "relay-fallback" | "force-relay";

export interface IceServer { urls: string | string[]; username?: string; credential?: string; }

/** Agent 上线：用 device token 认证 */
export interface AgentOnline { type: "agent-online"; token: string; }
/** Web 发起连接某设备 */
export interface Connect { type: "connect"; deviceId: string; }
/** 服务端通知 Agent 有入站会话 */
export interface Incoming { type: "incoming"; sessionId: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }
/** 服务端告知 Web 会话已建立、附 ICE 配置 */
export interface SessionReady { type: "session-ready"; sessionId: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }
/** SDP 转发（offer/answer） */
export interface Sdp { type: "sdp"; sessionId: string; sdp: { type: "offer" | "answer"; sdp: string }; }
/** ICE candidate 转发 */
export interface Ice { type: "ice"; sessionId: string; candidate: unknown; }
/** 对端离开 */
export interface PeerLeft { type: "peer-left"; sessionId: string; }
/** 错误 */
export interface ErrorMsg { type: "error"; code: string; message: string; }

export type SignalingMessage =
  | AgentOnline | Connect | Incoming | SessionReady | Sdp | Ice | PeerLeft | ErrorMsg;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}
function str(v: unknown, field: string): string {
  if (typeof v !== "string" || v.length === 0) throw new Error(`invalid field: ${field}`);
  return v;
}

export function parseSignalingMessage(raw: unknown): SignalingMessage {
  if (!isObj(raw)) throw new Error("message must be an object");
  const t = raw.type;
  switch (t) {
    case "agent-online": return { type: t, token: str(raw.token, "token") };
    case "connect": return { type: t, deviceId: str(raw.deviceId, "deviceId") };
    case "sdp": {
      const sdp = raw.sdp;
      if (!isObj(sdp) || (sdp.type !== "offer" && sdp.type !== "answer")) throw new Error("invalid sdp");
      return { type: t, sessionId: str(raw.sessionId, "sessionId"), sdp: { type: sdp.type, sdp: str(sdp.sdp, "sdp.sdp") } };
    }
    case "ice": return { type: t, sessionId: str(raw.sessionId, "sessionId"), candidate: raw.candidate };
    case "peer-left": return { type: t, sessionId: str(raw.sessionId, "sessionId") };
    default: throw new Error(`unknown signaling type: ${String(t)}`);
  }
}
```

> 注：`incoming`/`session-ready`/`error` 由服务端**产出**，不需要在入站解析里接受，故 `parseSignalingMessage` 只解析客户端可发送的类型。

- [ ] **Step 4: 从 index 导出**

`packages/protocol/src/index.ts` 追加：
```ts
export * from "./signaling.js";
```

- [ ] **Step 5: 跑测试确认通过**

Run: `npm test -w @rd/protocol`
Expected: PASS（4 个 signaling 用例 + smoke 通过）

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(protocol): signaling message types + parser"
```

---

### Task 3: 输入事件类型与校验

**Files:**
- Create: `packages/protocol/src/input.ts`
- Modify: `packages/protocol/src/index.ts`
- Test: `packages/protocol/test/input.test.ts`

**Interfaces:**
- Produces:
  - 类型 `InputEvent`（鼠标/键盘可辨识联合，`t` 为判别字段）。
  - `parseInputEvent(raw: unknown): InputEvent`——非法抛 `Error`。
  - 坐标 `x,y` ∈ [0,1]。

- [ ] **Step 1: 写失败测试**

`packages/protocol/test/input.test.ts`:
```ts
import { expect, test } from "vitest";
import { parseInputEvent } from "../src/input.js";

test("parses mouse move with relative coords", () => {
  expect(parseInputEvent({ t: "mmove", x: 0.5, y: 0.25 })).toEqual({ t: "mmove", x: 0.5, y: 0.25 });
});
test("parses mouse button", () => {
  expect(parseInputEvent({ t: "mdown", button: "left" }).t).toBe("mdown");
});
test("parses key event", () => {
  expect(parseInputEvent({ t: "kdown", code: "KeyA" })).toEqual({ t: "kdown", code: "KeyA" });
});
test("rejects out-of-range coord", () => {
  expect(() => parseInputEvent({ t: "mmove", x: 1.5, y: 0 })).toThrow();
});
test("rejects unknown button", () => {
  expect(() => parseInputEvent({ t: "mdown", button: "middle-left" })).toThrow();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -w @rd/protocol`
Expected: FAIL（`parseInputEvent` 未定义）

- [ ] **Step 3: 实现 input.ts**

```ts
export type MouseButton = "left" | "right" | "middle";

export interface MouseMove { t: "mmove"; x: number; y: number; }      // x,y ∈ [0,1]
export interface MouseDown { t: "mdown"; button: MouseButton; }
export interface MouseUp   { t: "mup";   button: MouseButton; }
export interface Wheel     { t: "wheel"; dx: number; dy: number; }
export interface KeyDown   { t: "kdown"; code: string; }              // KeyboardEvent.code
export interface KeyUp     { t: "kup";   code: string; }

export type InputEvent = MouseMove | MouseDown | MouseUp | Wheel | KeyDown | KeyUp;

const BUTTONS = new Set(["left", "right", "middle"]);

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}
function num(v: unknown, field: string): number {
  if (typeof v !== "number" || Number.isNaN(v)) throw new Error(`invalid number: ${field}`);
  return v;
}
function coord(v: unknown, field: string): number {
  const n = num(v, field);
  if (n < 0 || n > 1) throw new Error(`coord out of range [0,1]: ${field}`);
  return n;
}
function button(v: unknown): MouseButton {
  if (typeof v !== "string" || !BUTTONS.has(v)) throw new Error("invalid button");
  return v as MouseButton;
}
function str(v: unknown, field: string): string {
  if (typeof v !== "string" || v.length === 0) throw new Error(`invalid field: ${field}`);
  return v;
}

export function parseInputEvent(raw: unknown): InputEvent {
  if (!isObj(raw)) throw new Error("event must be an object");
  switch (raw.t) {
    case "mmove": return { t: "mmove", x: coord(raw.x, "x"), y: coord(raw.y, "y") };
    case "mdown": return { t: "mdown", button: button(raw.button) };
    case "mup":   return { t: "mup",   button: button(raw.button) };
    case "wheel": return { t: "wheel", dx: num(raw.dx, "dx"), dy: num(raw.dy, "dy") };
    case "kdown": return { t: "kdown", code: str(raw.code, "code") };
    case "kup":   return { t: "kup",   code: str(raw.code, "code") };
    default: throw new Error(`unknown input type: ${String(raw.t)}`);
  }
}
```

- [ ] **Step 4: 从 index 导出**

`packages/protocol/src/index.ts` 追加：
```ts
export * from "./input.js";
```

- [ ] **Step 5: 跑测试确认通过**

Run: `npm test -w @rd/protocol`
Expected: PASS（input 5 用例全过）

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(protocol): input event types + parser"
```

---

### Task 4: 服务端存储层（SQLite + 用户/设备仓储）

**Files:**
- Create: `packages/server/package.json`, `packages/server/tsconfig.json`, `packages/server/vitest.config.ts`
- Create: `packages/server/src/db.ts`
- Create: `packages/server/src/repo/users.ts`
- Create: `packages/server/src/repo/devices.ts`
- Test: `packages/server/test/repo.test.ts`

**Interfaces:**
- Produces:
  - `openDb(path: string): Database`（`:memory:` 建表）。
  - `UsersRepo`: `create(email, passwordHash): User`, `findByEmail(email): User | undefined`, `findById(id): User | undefined`。
  - `DevicesRepo`: `create(userId, name): Device`（生成 `id` 与 `token`）, `findByToken(token): Device | undefined`, `listByUser(userId): Device[]`, `findById(id): Device | undefined`。
  - 类型 `User { id, email, passwordHash, createdAt }`，`Device { id, userId, name, token, createdAt }`。

- [ ] **Step 1: 写包脚手架**

`packages/server/package.json`:
```json
{
  "name": "@rd/server",
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "tsx src/index.ts",
    "test": "vitest run"
  },
  "dependencies": {
    "@rd/protocol": "*",
    "fastify": "^4.28.0",
    "ws": "^8.18.0",
    "better-sqlite3": "^11.1.0",
    "jsonwebtoken": "^9.0.2",
    "bcryptjs": "^2.4.3"
  },
  "devDependencies": {
    "@types/ws": "^8.5.10",
    "@types/better-sqlite3": "^7.6.11",
    "@types/jsonwebtoken": "^9.0.6",
    "@types/bcryptjs": "^2.4.6"
  }
}
```

`packages/server/tsconfig.json`:
```json
{ "extends": "../../tsconfig.base.json", "include": ["src", "test"] }
```

`packages/server/vitest.config.ts`:
```ts
import { defineConfig } from "vitest/config";
export default defineConfig({ test: { environment: "node" } });
```

- [ ] **Step 2: 写失败测试**

`packages/server/test/repo.test.ts`:
```ts
import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";

let users: UsersRepo, devices: DevicesRepo;
beforeEach(() => {
  const db = openDb(":memory:");
  users = new UsersRepo(db);
  devices = new DevicesRepo(db);
});

test("create and find user by email", () => {
  const u = users.create("a@b.com", "hash");
  expect(u.id).toBeTruthy();
  expect(users.findByEmail("a@b.com")?.id).toBe(u.id);
});

test("duplicate email throws", () => {
  users.create("a@b.com", "hash");
  expect(() => users.create("a@b.com", "hash2")).toThrow();
});

test("device gets id + token, findable by token", () => {
  const u = users.create("a@b.com", "hash");
  const d = devices.create(u.id, "My PC");
  expect(d.token.length).toBeGreaterThan(16);
  expect(devices.findByToken(d.token)?.id).toBe(d.id);
  expect(devices.listByUser(u.id).map(x => x.id)).toContain(d.id);
});
```

- [ ] **Step 3: 跑测试确认失败**

Run: `npm test -w @rd/server`
Expected: FAIL（`openDb` 等未定义）

- [ ] **Step 4: 实现 db.ts**

```ts
import Database from "better-sqlite3";

export type Db = Database.Database;

export function openDb(path: string): Db {
  const db = new Database(path);
  db.pragma("journal_mode = WAL");
  db.exec(`
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      email TEXT UNIQUE NOT NULL,
      passwordHash TEXT NOT NULL,
      createdAt INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS devices (
      id TEXT PRIMARY KEY,
      userId TEXT NOT NULL REFERENCES users(id),
      name TEXT NOT NULL,
      token TEXT UNIQUE NOT NULL,
      createdAt INTEGER NOT NULL
    );
  `);
  return db;
}
```

- [ ] **Step 5: 实现 repo/users.ts**

```ts
import { randomUUID } from "node:crypto";
import type { Db } from "../db.js";

export interface User { id: string; email: string; passwordHash: string; createdAt: number; }

export class UsersRepo {
  constructor(private db: Db) {}
  create(email: string, passwordHash: string): User {
    const user: User = { id: randomUUID(), email, passwordHash, createdAt: Date.now() };
    this.db.prepare("INSERT INTO users (id,email,passwordHash,createdAt) VALUES (?,?,?,?)")
      .run(user.id, user.email, user.passwordHash, user.createdAt);
    return user;
  }
  findByEmail(email: string): User | undefined {
    return this.db.prepare("SELECT * FROM users WHERE email = ?").get(email) as User | undefined;
  }
  findById(id: string): User | undefined {
    return this.db.prepare("SELECT * FROM users WHERE id = ?").get(id) as User | undefined;
  }
}
```

- [ ] **Step 6: 实现 repo/devices.ts**

```ts
import { randomUUID, randomBytes } from "node:crypto";
import type { Db } from "../db.js";

export interface Device { id: string; userId: string; name: string; token: string; createdAt: number; }

export class DevicesRepo {
  constructor(private db: Db) {}
  create(userId: string, name: string): Device {
    const device: Device = {
      id: randomUUID(), userId, name,
      token: randomBytes(24).toString("hex"), createdAt: Date.now(),
    };
    this.db.prepare("INSERT INTO devices (id,userId,name,token,createdAt) VALUES (?,?,?,?,?)")
      .run(device.id, device.userId, device.name, device.token, device.createdAt);
    return device;
  }
  findByToken(token: string): Device | undefined {
    return this.db.prepare("SELECT * FROM devices WHERE token = ?").get(token) as Device | undefined;
  }
  findById(id: string): Device | undefined {
    return this.db.prepare("SELECT * FROM devices WHERE id = ?").get(id) as Device | undefined;
  }
  listByUser(userId: string): Device[] {
    return this.db.prepare("SELECT * FROM devices WHERE userId = ? ORDER BY createdAt").all(userId) as Device[];
  }
}
```

- [ ] **Step 7: 跑测试确认通过**

Run: `npm install && npm test -w @rd/server`
Expected: PASS（repo 3 用例通过）

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(server): sqlite store with users + devices repos"
```

---

### Task 5: 认证（哈希 + JWT）与 auth 路由

**Files:**
- Create: `packages/server/src/config.ts`
- Create: `packages/server/src/auth.ts`
- Create: `packages/server/src/routes/auth.ts`
- Create: `packages/server/src/app.ts`
- Test: `packages/server/test/auth.test.ts`

**Interfaces:**
- Consumes: `UsersRepo`（Task 4）。
- Produces:
  - `hashPassword(pw): Promise<string>`, `verifyPassword(pw, hash): Promise<boolean>`。
  - `signToken(userId): string`, `verifyToken(token): { userId: string }`（失败抛错）。
  - `buildApp(deps): FastifyInstance`——注册 `POST /register`、`POST /login`，返回 `{ token }`。
  - `loadConfig(): Config { port, jwtSecret, relayPolicy, iceServers }`。

- [ ] **Step 1: 写 config.ts**

```ts
import type { RelayPolicy, IceServer } from "@rd/protocol";

export interface Config { port: number; jwtSecret: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  return {
    port: Number(env.PORT ?? 8080),
    jwtSecret: env.JWT_SECRET ?? "dev-secret-change-me",
    relayPolicy: (env.RELAY_POLICY as RelayPolicy) ?? "relay-fallback",
    iceServers: env.ICE_SERVERS ? JSON.parse(env.ICE_SERVERS) : [{ urls: "stun:stun.l.google.com:19302" }],
  };
}
```

- [ ] **Step 2: 写失败测试**

`packages/server/test/auth.test.ts`:
```ts
import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { buildApp } from "../src/app.js";

function makeApp() {
  const db = openDb(":memory:");
  return buildApp({
    users: new UsersRepo(db), devices: new DevicesRepo(db),
    config: { port: 0, jwtSecret: "test-secret", relayPolicy: "relay-fallback", iceServers: [] },
  });
}

let app: ReturnType<typeof makeApp>;
beforeEach(() => { app = makeApp(); });

test("register returns a token", async () => {
  const res = await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  expect(res.statusCode).toBe(200);
  expect(JSON.parse(res.body).token).toBeTruthy();
});

test("login after register succeeds", async () => {
  await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  const res = await app.inject({ method: "POST", url: "/login", payload: { email: "a@b.com", password: "pw123456" } });
  expect(res.statusCode).toBe(200);
});

test("login with wrong password fails 401", async () => {
  await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  const res = await app.inject({ method: "POST", url: "/login", payload: { email: "a@b.com", password: "wrong" } });
  expect(res.statusCode).toBe(401);
});
```

- [ ] **Step 3: 跑测试确认失败**

Run: `npm test -w @rd/server`
Expected: FAIL（`buildApp` 未定义）

- [ ] **Step 4: 实现 auth.ts**

```ts
import bcrypt from "bcryptjs";
import jwt from "jsonwebtoken";

export async function hashPassword(pw: string): Promise<string> {
  return bcrypt.hash(pw, 10);
}
export async function verifyPassword(pw: string, hash: string): Promise<boolean> {
  return bcrypt.compare(pw, hash);
}
export function signToken(userId: string, secret: string): string {
  return jwt.sign({ sub: userId }, secret, { expiresIn: "7d" });
}
export function verifyToken(token: string, secret: string): { userId: string } {
  const payload = jwt.verify(token, secret) as { sub: string };
  return { userId: payload.sub };
}
```

- [ ] **Step 5: 实现 routes/auth.ts**

```ts
import type { FastifyInstance } from "fastify";
import type { UsersRepo } from "../repo/users.js";
import { hashPassword, verifyPassword, signToken } from "../auth.js";

interface Body { email?: string; password?: string; }

export function registerAuthRoutes(app: FastifyInstance, users: UsersRepo, jwtSecret: string) {
  app.post("/register", async (req, reply) => {
    const { email, password } = (req.body ?? {}) as Body;
    if (!email || !password || password.length < 6) return reply.code(400).send({ error: "invalid input" });
    if (users.findByEmail(email)) return reply.code(409).send({ error: "email exists" });
    const user = users.create(email, await hashPassword(password));
    return { token: signToken(user.id, jwtSecret) };
  });

  app.post("/login", async (req, reply) => {
    const { email, password } = (req.body ?? {}) as Body;
    if (!email || !password) return reply.code(400).send({ error: "invalid input" });
    const user = users.findByEmail(email);
    if (!user || !(await verifyPassword(password, user.passwordHash)))
      return reply.code(401).send({ error: "bad credentials" });
    return { token: signToken(user.id, jwtSecret) };
  });
}
```

- [ ] **Step 6: 实现 app.ts（先只挂 auth 路由）**

```ts
import Fastify, { type FastifyInstance } from "fastify";
import type { UsersRepo } from "./repo/users.js";
import type { DevicesRepo } from "./repo/devices.js";
import type { Config } from "./config.js";
import { registerAuthRoutes } from "./routes/auth.js";

export interface AppDeps { users: UsersRepo; devices: DevicesRepo; config: Config; }

export function buildApp(deps: AppDeps): FastifyInstance {
  const app = Fastify({ logger: false });
  registerAuthRoutes(app, deps.users, deps.config.jwtSecret);
  return app;
}
```

- [ ] **Step 7: 跑测试确认通过**

Run: `npm test -w @rd/server`
Expected: PASS（auth 3 用例通过）

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(server): password hashing, JWT, register/login routes"
```

---

### Task 6: 设备路由（列表 + 配对）与 JWT 鉴权

**Files:**
- Create: `packages/server/src/routes/devices.ts`
- Modify: `packages/server/src/app.ts`
- Test: `packages/server/test/devices.test.ts`

**Interfaces:**
- Consumes: `DevicesRepo`（Task 4）、`verifyToken`（Task 5）。
- Produces:
  - `GET /devices` →（需 `Authorization: Bearer <jwt>`）`{ devices: {id,name,online}[] }`（`online` 本任务恒为 false，Task 7 接入在线表后变真实）。
  - `POST /devices/pair` `{ name }` → `{ deviceId, token }`（该 token 供 Agent 使用）。
  - `authUser(req): string`——从 Bearer 头解析 userId，失败抛。

- [ ] **Step 1: 写失败测试**

`packages/server/test/devices.test.ts`:
```ts
import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { buildApp } from "../src/app.js";

function makeApp() {
  const db = openDb(":memory:");
  return buildApp({
    users: new UsersRepo(db), devices: new DevicesRepo(db),
    config: { port: 0, jwtSecret: "test-secret", relayPolicy: "relay-fallback", iceServers: [] },
  });
}
let app: ReturnType<typeof makeApp>;
beforeEach(() => { app = makeApp(); });

async function token() {
  const res = await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  return JSON.parse(res.body).token as string;
}

test("pair then list device", async () => {
  const jwt = await token();
  const pair = await app.inject({ method: "POST", url: "/devices/pair", headers: { authorization: `Bearer ${jwt}` }, payload: { name: "My PC" } });
  expect(pair.statusCode).toBe(200);
  const { deviceId, token: devToken } = JSON.parse(pair.body);
  expect(deviceId).toBeTruthy(); expect(devToken).toBeTruthy();

  const list = await app.inject({ method: "GET", url: "/devices", headers: { authorization: `Bearer ${jwt}` } });
  const { devices } = JSON.parse(list.body);
  expect(devices).toHaveLength(1);
  expect(devices[0]).toMatchObject({ id: deviceId, name: "My PC", online: false });
});

test("list without token → 401", async () => {
  const res = await app.inject({ method: "GET", url: "/devices" });
  expect(res.statusCode).toBe(401);
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -w @rd/server`
Expected: FAIL（/devices 路由不存在 → 404，断言失败）

- [ ] **Step 3: 实现 routes/devices.ts**

```ts
import type { FastifyInstance, FastifyRequest } from "fastify";
import type { DevicesRepo } from "../repo/devices.js";
import { verifyToken } from "../auth.js";

export function authUser(req: FastifyRequest, jwtSecret: string): string {
  const header = req.headers.authorization;
  if (!header?.startsWith("Bearer ")) throw new Error("no token");
  return verifyToken(header.slice(7), jwtSecret).userId;
}

/** isOnline: 由 Task 7 的在线表注入；本任务默认恒 false */
export function registerDeviceRoutes(
  app: FastifyInstance, devices: DevicesRepo, jwtSecret: string,
  isOnline: (deviceId: string) => boolean = () => false,
) {
  app.get("/devices", async (req, reply) => {
    let userId: string;
    try { userId = authUser(req, jwtSecret); } catch { return reply.code(401).send({ error: "unauthorized" }); }
    return { devices: devices.listByUser(userId).map(d => ({ id: d.id, name: d.name, online: isOnline(d.id) })) };
  });

  app.post("/devices/pair", async (req, reply) => {
    let userId: string;
    try { userId = authUser(req, jwtSecret); } catch { return reply.code(401).send({ error: "unauthorized" }); }
    const { name } = (req.body ?? {}) as { name?: string };
    if (!name) return reply.code(400).send({ error: "name required" });
    const d = devices.create(userId, name);
    return { deviceId: d.id, token: d.token };
  });
}
```

- [ ] **Step 4: 在 app.ts 挂载设备路由**

修改 `buildApp`，在 `registerAuthRoutes(...)` 之后加：
```ts
import { registerDeviceRoutes } from "./routes/devices.js";
// ...在 buildApp 内：
  registerDeviceRoutes(app, deps.devices, deps.config.jwtSecret, deps.isOnline);
```
并在 `AppDeps` 接口加可选字段：
```ts
export interface AppDeps { users: UsersRepo; devices: DevicesRepo; config: Config; isOnline?: (deviceId: string) => boolean; }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `npm test -w @rd/server`
Expected: PASS（devices 2 用例 + 之前全部通过）

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(server): device pair + list routes with JWT auth"
```

---

### Task 7: WebSocket 信令转发（在线表 + 会话中转）

**Files:**
- Create: `packages/server/src/signaling/registry.ts`
- Create: `packages/server/src/signaling/hub.ts`
- Modify: `packages/server/src/app.ts`
- Create: `packages/server/src/index.ts`
- Test: `packages/server/test/signaling.test.ts`

**Interfaces:**
- Consumes: `DevicesRepo.findByToken`（Task 4）、`verifyToken`（Task 5）、`parseSignalingMessage`（Task 2）、`config.relayPolicy/iceServers`（Task 5）。
- Produces:
  - `Registry`: `setAgentOnline(deviceId, conn)`, `isOnline(deviceId): boolean`, `getAgent(deviceId): Conn | undefined`, `remove(conn)`；`createSession(webConn, agentConn): sessionId`, `peerOf(conn, sessionId): Conn | undefined`, `dropSession(sessionId)`。
  - `attachSignaling(wss, deps)`——把 ws 服务器接到转发逻辑。
  - `Conn` 抽象：`{ send(msg): void; onMessage(cb); onClose(cb); }`（对 `ws.WebSocket` 的最小封装，便于测试注入假连接）。

- [ ] **Step 1: 写失败测试（用假 Conn，纯逻辑，无网络）**

`packages/server/test/signaling.test.ts`:
```ts
import { expect, test, beforeEach } from "vitest";
import { Registry } from "../src/signaling/registry.js";

let reg: Registry;
beforeEach(() => { reg = new Registry(); });

test("agent online/offline tracked", () => {
  const conn = { send() {}, close() {} };
  reg.setAgentOnline("dev-1", conn as any);
  expect(reg.isOnline("dev-1")).toBe(true);
  reg.remove(conn as any);
  expect(reg.isOnline("dev-1")).toBe(false);
});

test("session links web and agent as peers", () => {
  const web = { send() {}, close() {} } as any;
  const agent = { send() {}, close() {} } as any;
  reg.setAgentOnline("dev-1", agent);
  const sid = reg.createSession(web, agent);
  expect(reg.peerOf(web, sid)).toBe(agent);
  expect(reg.peerOf(agent, sid)).toBe(web);
  reg.dropSession(sid);
  expect(reg.peerOf(web, sid)).toBeUndefined();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -w @rd/server`
Expected: FAIL（`Registry` 未定义）

- [ ] **Step 3: 实现 registry.ts**

```ts
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

  remove(conn: Conn): void {
    const deviceId = this.agentByConn.get(conn);
    if (deviceId) { this.agents.delete(deviceId); this.agentByConn.delete(conn); }
    for (const [sid, s] of this.sessions) if (s.web === conn || s.agent === conn) this.sessions.delete(sid);
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
```

- [ ] **Step 4: 实现 hub.ts（转发逻辑；含集成测试用的接线函数）**

```ts
import type { WebSocketServer, WebSocket } from "ws";
import { parseSignalingMessage } from "@rd/protocol";
import type { DevicesRepo } from "../repo/devices.js";
import { verifyToken } from "../auth.js";
import type { Config } from "../config.js";
import { Registry, type Conn } from "./registry.js";

export interface HubDeps { devices: DevicesRepo; config: Config; registry: Registry; }

function wrap(ws: WebSocket): Conn {
  return { send: (d) => ws.send(d), close: () => ws.close() };
}

export function attachSignaling(wss: WebSocketServer, deps: HubDeps) {
  const { devices, config, registry } = deps;
  wss.on("connection", (ws, req) => {
    const conn = wrap(ws);
    // Web 端用 ?token=<jwt> 鉴权；Agent 端发 agent-online 携带 device token
    const url = new URL(req.url ?? "/", "http://localhost");
    const jwt = url.searchParams.get("token");
    let webUserId: string | undefined;
    if (jwt) { try { webUserId = verifyToken(jwt, config.jwtSecret).userId; } catch { ws.close(); return; } }

    ws.on("message", (raw) => {
      let msg;
      try { msg = parseSignalingMessage(JSON.parse(raw.toString())); }
      catch { conn.send(JSON.stringify({ type: "error", code: "bad-message", message: "unparseable" })); return; }

      switch (msg.type) {
        case "agent-online": {
          const dev = devices.findByToken(msg.token);
          if (!dev) { conn.send(JSON.stringify({ type: "error", code: "bad-token", message: "invalid device token" })); ws.close(); return; }
          registry.setAgentOnline(dev.id, conn);
          break;
        }
        case "connect": {
          const agent = registry.getAgent(msg.deviceId);
          if (!agent) { conn.send(JSON.stringify({ type: "error", code: "offline", message: "device offline" })); return; }
          const sessionId = registry.createSession(conn, agent);
          const ice = { relayPolicy: config.relayPolicy, iceServers: config.iceServers };
          agent.send(JSON.stringify({ type: "incoming", sessionId, ...ice }));
          conn.send(JSON.stringify({ type: "session-ready", sessionId, ...ice }));
          break;
        }
        case "sdp": case "ice": {
          const peer = registry.peerOf(conn, msg.sessionId);
          if (peer) peer.send(JSON.stringify(msg));
          break;
        }
        case "peer-left": {
          const peer = registry.peerOf(conn, msg.sessionId);
          if (peer) peer.send(JSON.stringify(msg));
          registry.dropSession(msg.sessionId);
          break;
        }
      }
    });
    ws.on("close", () => registry.remove(conn));
  });
}
```

- [ ] **Step 5: 集成测试（真实 ws，端到端转发）**

追加到 `packages/server/test/signaling.test.ts`:
```ts
import { WebSocketServer, WebSocket } from "ws";
import { createServer } from "node:http";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { attachSignaling } from "../src/signaling/hub.js";

async function waitMsg(ws: WebSocket): Promise<any> {
  return new Promise((res) => ws.once("message", (d) => res(JSON.parse(d.toString()))));
}

test("agent-online → connect → sdp relayed to agent", async () => {
  const db = openDb(":memory:");
  const users = new UsersRepo(db); const devices = new DevicesRepo(db);
  const u = users.create("a@b.com", "h"); const dev = devices.create(u.id, "PC");
  const registry = new Registry();
  const http = createServer(); const wss = new WebSocketServer({ server: http });
  attachSignaling(wss, { devices, config: { port: 0, jwtSecret: "s", relayPolicy: "relay-fallback", iceServers: [] }, registry });
  await new Promise<void>((r) => http.listen(0, r));
  const port = (http.address() as any).port;

  const agent = new WebSocket(`ws://localhost:${port}`);
  await new Promise((r) => agent.once("open", r));
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = new WebSocket(`ws://localhost:${port}`);
  await new Promise((r) => web.once("open", r));
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));

  const incoming = await waitMsg(agent);
  expect(incoming.type).toBe("incoming");
  const sessionId = incoming.sessionId;

  // web 发 sdp offer，应被转发到 agent
  web.send(JSON.stringify({ type: "sdp", sessionId, sdp: { type: "offer", sdp: "v=0" } }));
  const relayed = await waitMsg(agent);
  expect(relayed).toMatchObject({ type: "sdp", sessionId });

  agent.close(); web.close(); wss.close(); http.close();
});
```

- [ ] **Step 6: 实现进程入口 index.ts（把 HTTP+WS+在线表组装起来）**

```ts
import { createServer } from "node:http";
import { WebSocketServer } from "ws";
import { openDb } from "./db.js";
import { UsersRepo } from "./repo/users.js";
import { DevicesRepo } from "./repo/devices.js";
import { loadConfig } from "./config.js";
import { buildApp } from "./app.js";
import { Registry } from "./signaling/registry.js";
import { attachSignaling } from "./signaling/hub.js";

const config = loadConfig();
const db = openDb(process.env.DB_PATH ?? "remote-desktop.db");
const users = new UsersRepo(db);
const devices = new DevicesRepo(db);
const registry = new Registry();

const app = buildApp({ users, devices, config, isOnline: (id) => registry.isOnline(id) });
const server = createServer();
app.server.on("request", () => {}); // Fastify 有自己的 server；见下方说明
const wss = new WebSocketServer({ noServer: true });
attachSignaling(wss, { devices, config, registry });

// 把 WS upgrade 挂到 Fastify 的 server 上
app.ready().then(() => {
  app.server.on("upgrade", (req, socket, head) => {
    wss.handleUpgrade(req, socket as any, head, (ws) => wss.emit("connection", ws, req));
  });
  app.listen({ port: config.port, host: "0.0.0.0" });
  console.log(`server on :${config.port}`);
});
```

> 说明：Fastify 内部已持有一个 `http.Server`（`app.server`）；WS 复用它的 `upgrade` 事件，无需第二个 HTTP server。上面的 `createServer()`/`app.server.on("request")` 两行是多余的，实现时删掉，只保留 `WebSocketServer({ noServer: true })` + `app.server.on("upgrade", ...)`。

- [ ] **Step 7: 跑全部测试确认通过**

Run: `npm test -w @rd/server`
Expected: PASS（registry 2 用例 + 集成 1 用例 + 之前 auth/devices/repo 全部通过）

- [ ] **Step 8: 手动冒烟（可选但推荐）**

Run: `npm run dev -w @rd/server`，另开终端用 `curl` 注册/登录/配对，确认返回 token。
Expected: 各接口 200，日志打印 `server on :8080`。

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(server): websocket signaling relay + online registry + entrypoint"
```

---

## Self-Review

**Spec coverage（对照设计文档各节）：**
- §2 账号极简（邮箱+密码+JWT）→ Task 5 ✅
- §2 设备列表/配对 → Task 6 ✅
- §4.2 REST（register/login/devices/pair）→ Task 5、6 ✅
- §4.2 WebSocket 纯转发 SDP/ICE → Task 7 ✅
- §4.2 store SQLite → Task 4 ✅
- §4.2 turn-config 下发 → Task 7（`incoming`/`session-ready` 携带 `relayPolicy`+`iceServers`）✅
- §4.4 共享协议（信令+输入事件，TS 实现）→ Task 2、3 ✅
- §5 在线状态靠 Agent 长连存活 → Task 7（`remove` on close）✅
- §6 中继策略随会话下发 → Task 7 ✅
- **不在本计划**：WebRTC 媒体、Rust Agent、React 前端、coturn 部署（属 Plan 2~4）——设计 §9 开发顺序如此分层，无遗漏。

**Placeholder scan：** 无 TBD/TODO；Step 6 的多余两行已用说明标注删除方式（非占位符，是明确指令）。

**Type consistency：** `SignalingMessage`/`InputEvent`/`RelayPolicy` 全程一致；`Conn` 接口在 registry 定义、hub 复用；`AppDeps.isOnline` 在 Task 6 定义、Task 7 入口注入——一致。

## Execution Handoff

计划已保存到 `docs/superpowers/plans/2026-07-01-plan1-protocol-and-server.md`。
