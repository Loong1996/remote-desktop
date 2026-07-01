# rd-agent (被控端 Rust Agent)

`rd-agent` 是运行在被控设备上的原生代理：首次启动时交互式登录并配对，之后保存本地
凭证，长连信令服务器等待远程会话，并通过 WebRTC data channel 与控制端通信。

当前阶段（Plan 2a）已实现：配置读写、登录配对、信令 WebSocket 长连、WebRTC
data channel 回显（`echo:<msg>`）。抓屏 / 键鼠注入见 Plan 3/4；真实浏览器控制端
的端到端联调见 Plan 2b。

## 运行前置

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

1. 启动 Node 信令/REST 服务端（仓库根目录）：

   ```bash
   JWT_SECRET=dev npm run dev -w @rd/server
   ```

   默认监听 `http://127.0.0.1:8080`（可用 `PORT` 环境变量覆盖）。

2. 用同一账号在服务端注册一个用户（首次运行 agent 时需要输入这组邮箱/密码）：

   ```bash
   curl -X POST http://127.0.0.1:8080/register \
     -H 'Content-Type: application/json' \
     -d '{"email":"a@b.com","password":"pw123456"}'
   ```

## 首次运行（登录配对）

```bash
cargo run --manifest-path agent/Cargo.toml
```

流程：

1. 本地未发现配置文件 → 打印 `No config found. Log in to pair this device.`
2. 依次输入 `Email:` / `Password:`（密码使用 `rpassword`，不回显）。
3. Agent 用这组账号密码调用服务端 `/login` 换取 JWT，再调用
   `/devices/pair`（携带主机名作为设备名）完成配对，拿到 `deviceId` + 设备 token。
4. 配对结果写入本地配置文件，打印 `Paired as device <deviceId>`。
5. 立即用保存的配置连上信令 WebSocket 并上线，日志打印
   `agent online, waiting for sessions`。

## 后续运行（已有配置）

再次运行同一命令，会直接读取本地配置文件、跳过登录，日志打印
`loaded config for device <deviceId>`，然后上线等待远程会话。

```bash
cargo run --manifest-path agent/Cargo.toml
```

## 配置文件与环境变量

- 默认配置文件路径：`<系统配置目录>/rd-agent/config.json`
  （Windows 上通常是 `%APPDATA%\rd-agent\config.json`，由 `dirs::config_dir()` 决定）。
- `RD_AGENT_CONFIG`：覆盖配置文件的完整路径（测试/多实例场景常用）。
- `RD_SERVER_URL`：仅在**首次配对**（本地无配置文件）时生效，指定要连接的服务端
  REST 地址，默认为 `http://127.0.0.1:8080`。配对成功后，服务端地址会被写入配置
  文件，后续启动直接复用，不再读取该变量。

配置文件内容示例：

```json
{
  "server_url": "http://127.0.0.1:8080",
  "device_id": "dev-9",
  "device_token": "devtok-9"
}
```

删除该文件（或指向一个不存在的 `RD_AGENT_CONFIG` 路径）即可强制下一次启动重新走
登录配对流程。

## 手动端到端冒烟（Plan 2b 之前的临时验证）

真实浏览器控制端的完整链路要到 Plan 2b 才接入；在此之前可以用一个最小的 WebSocket
脚本模拟控制端，验证 agent 的信令 + WebRTC 回显是否工作：

1. 按上文完成服务端启动 + agent 配对上线，确认日志里出现
   `agent online, waiting for sessions`。
2. 编写一个最小 ws 客户端（Node/`wscat`/浏览器控制台均可）连接到同一信令端点，
   发送触发会话的 `connect` 消息（携带上一步配对得到的 `deviceId`），随后发送一个
   WebRTC offer。
3. 观察 agent 侧日志：应打印 `incoming session accepted, awaiting offer`，随后
   在收到 offer 后返回一条 `type: "sdp"`、`sdp.kind: "answer"` 的信令消息。
4. 若要验证 data channel 本身的回显行为，可直接运行 crate 自带的环回测试，无需
   手起真实进程：

   ```bash
   cargo test --manifest-path agent/Cargo.toml --test echo_loopback
   ```

   该测试在单个进程内建立两个 `PeerConnection` 直连，验证 agent 端对
   `hello` 消息回复 `echo:hello`。

完整的“真实浏览器 ↔ agent”端到端联调（含 React 控制端 UI、ICE 协商、TURN 兜底）
在 Plan 2b 中实现和验证。

## 测试

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --manifest-path agent/Cargo.toml
cargo test --manifest-path agent/Cargo.toml
```
