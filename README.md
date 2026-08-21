<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="PortMate logo">
</p>

<h1 align="center">PortMate</h1>

<p align="center">A cross-platform terminal workspace for SSH, serial, and remote operations, with a permissioned MCP session bridge.</p>

<p align="center">
  <strong>English</strong> |
  <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/YYH2913/PortMate/actions/workflows/native-ci.yml">Native CI</a> ·
  <a href="https://github.com/YYH2913/PortMate/actions/workflows/mcp-sdk-freshness.yml">MCP SDK Freshness</a> ·
  <a href="./LICENSE">Apache-2.0</a> ·
  <a href="./.nvmrc">Node.js 22.20.0</a>
</p>

> [!IMPORTANT]
> PortMate is currently **alpha** software. The implementation and compatibility tests reproducible on Linux, Docker, and browser environments are in place, but native Windows/macOS packages, real Microsoft AD, physical serial hardware, and release signing still require external validation. Do not use the current build unattended in a production-critical path.

## What is PortMate?

PortMate is a terminal-first Tauri v2 desktop application. It brings SSH, local Shell PTY, serial, Telnet, raw TCP, Tmux, file transfer, tunnels, logging, and system monitoring into one split-pane workspace.

One of its primary design goals is session-level identity and trust isolation. SSH host keys, client identities, authentication order, and Jump Host policies belong to a PortMate Profile instead of the global `~/.ssh/known_hosts`. Devices that share an IP and port, rebuilt systems, and lab boards can therefore keep separate, reviewable trust records.

PortMate does not embed an AI assistant. The packaged `portmate-mcp` bridge lets external MCP hosts inspect sessions, query logs, or perform control actions within grants approved by the user.

## Features

| Area | Capabilities |
| --- | --- |
| Sessions and protocols | SSH, local Shell PTY, Serial, Telnet, raw TCP, and Tmux; TLS for TCP/Telnet; HTTP CONNECT and SOCKS5 proxies for SSH/Tmux/TCP/Telnet |
| Terminal workspace | Tabs, recursive splits, cross-group drag and drop, detached windows, layout restore, Insert/Normal modes with matching cursors, search, line navigation, and text/hex/split views |
| Interactive workflow | WindTerm-style command completion and parameter hints, semantic command coloring, Quick Commands, scoped custom scripts, OneKeys, free input, and synchronized input |
| SSH security | Profile-scoped host keys, TOFU and key-change blocking, Host/Client Key Managers, multi-hop Jump Hosts, ssh-agent, password/public-key/keyboard-interactive, and Linux libssh GSSAPI authentication |
| Files and transfer | SFTP/SCP file management, one-shot TFTP, drag and drop, queues, throttling, cancellation, retry, resumable transfers, and X/Y/ZModem |
| Operations and diagnostics | SSH health checks, local/remote/dynamic tunnels, persistent Sysmon sidebar and trends, structured logs, triggers, and diagnostic session bundles |
| Serial tooling | Common baud rates, DTR/RTS/Break, text and hex I/O, exact byte capture, a detached analyzer, and SLIP/COBS/Modbus RTU decoding |
| MCP | stdio and Streamable HTTP, fine-grained grants, session scopes, write confirmation, audit history, token rotation, and managed sidecar lifecycle |

## Project Status

- Target desktop platforms: Linux, Windows, and macOS.
- Primary current development and native verification environment: Linux on VMware.
- The terminal uses `@xterm/xterm` 6 with Search, Serialize, Unicode 11, Clipboard, Web Links, Fit, and optional WebGL support.
- Automated compatibility matrices cover SSH/SFTP/SCP, Telnet/TCP, vttest, full-screen applications, and multiple Tmux versions.
- MCP compatibility covers the official TypeScript, Python, Go, Rust, Ruby, Java, Kotlin, C#, and Swift SDKs.
- Linux DEB, RPM, and AppImage packages have local package and lifecycle gates. Windows and macOS still require successful native-runner evidence.

See [CHANGELOG.md](./CHANGELOG.md) for versioned user-visible changes, [PROGRESS.md](./PROGRESS.md) for the detailed implementation record and remaining boundaries, and [RELEASE.md](./RELEASE.md) for release requirements.

## Quick Start

### Prerequisites

- Git
- Node.js `>= 22.12.0`; `.nvmrc` pins the verified `22.20.0` release
- A stable Rust toolchain
- The [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform

Building the Linux desktop application generally also requires WebKitGTK, GTK3, libssh, Kerberos, udev, and system-tray development packages. The complete Ubuntu dependency list is available in [.github/workflows/native-ci.yml](./.github/workflows/native-ci.yml).

### Run the Desktop App

```bash
git clone https://github.com/YYH2913/PortMate.git
cd PortMate

nvm use
npm ci
npm run desktop:clean
```

`desktop:clean` only clears a stale Vite listener owned by this checkout and known Snap-injected GTK/WebKit environment values. It does not remove Profiles, logs, or PortMate application data. When cleanup is unnecessary, run:

```bash
npm run desktop
```

The launcher builds the development MCP sidecar, starts Vite, and then starts the Tauri/Rust backend. The first Rust build can take a while.

### Browser Preview

```bash
npm run dev
```

Open <http://127.0.0.1:1420/>. Browser mode is useful for layout and frontend interaction checks. Real SSH, serial, transfer, keyring, and local IPC operations require the Tauri desktop backend.

## Basic Usage

1. Open `会话 (Session) -> 新建会话 (New Session)`.
2. Select Shell, SSH, Tmux, Telnet, TCP, or Serial.
3. Enter host/IP, port, and username in separate fields; a combined `username@host` value is not required.
4. On the first SSH connection, verify the server host-key fingerprint and choose one-time or persistent Profile/project trust.
5. Open transfers, tunnels, Tmux, Sysmon, custom scripts, the serial analyzer, key management, logs, or MCP Bridge from `工具 (Tools)`.

Each session tab has a connection marker. Green means connected; unavailable states are red, with the precise connecting, reconnecting, blocked, disconnected, or error diagnosis available in the tooltip. Startup recovery silently tries credentials already available to the Profile and does not show a reconnect dialog every time the app opens.

SSH, Tmux, TCP, Telnet, and Serial reconnect workers reload the latest Profile before the next attempt. Changes to connection settings, authentication policy, reconnect delay, or the reconnect switch do not need to wait for an obsolete retry cycle to finish.

## Desktop File Transfers

SFTP works directly in the desktop application and does not require MCP, an MCP grant, or a running MCP Bridge. Connect an SSH or Tmux session with SFTP enabled, then use either `Workspace -> File Manager` for two-pane browsing, drag and drop, and batch transfers, or `Tools -> Transfer Tasks` for an explicit source and destination.

For an upload, use a local source path and a destination such as `remote:/tmp/file.bin`. For a download, use a source such as `remote:/var/log/messages` and a local destination path. The desktop transfer queue supports cancellation, retry, progress, throttling, and resumable SFTP operations.

## SSH Trust and Credentials

PortMate manages two distinct types of keys:

- **Host keys** identify remote servers. They are stored in PortMate's trust store by default and are not automatically written to the system `known_hosts`.
- **Client identities** authenticate the user to a server. Their offer order can be constrained so unrelated keys do not exhaust the server's `MaxAuthTries` limit.

SSH passwords, private-key passphrases, proxy passwords, OneKey secrets, and Profile Vault private keys are stored in an IOTA Stronghold vault protected by the user's master password; SQLite stores only `stronghold:` references. The vault must be unlocked before these secrets can be saved or used. The native OS keyring is reserved for internal application material such as the persistent MCP HTTP token and bundle-signing identity. Existing user `keychain:` references remain readable and deletable and can be migrated one way into Stronghold, but PortMate no longer creates or overwrites user credentials in the native keyring. The short-lived desktop IPC token rotates on every launch and is written only to the atomic, owner-only `portmate-ipc.json` endpoint; it is never stored in SQLite.

During SSH setup, plaintext credentials stay inside the desktop backend after the trusted credential prompt. Frontend calls receive only a short-lived, one-use handle bound to the requesting window, session, and current SSH configuration; MCP requests cannot supply passwords, passphrases, or credential handles. See [SECURITY.md](./SECURITY.md) for the exact trust boundary and its limitations.

A changed host key blocks the connection by default. Use one-time trust, append, or replacement only after confirming a device replacement, OS rebuild, or legitimate key rotation. Do not disable verification merely to remove a warning.

## MCP Bridge

### Recommended Setup

1. Keep the PortMate desktop application running.
2. Open `工具 (Tools) -> MCP Bridge -> 授权 (Grants)` and create a distinct Client ID with the required scopes and allowed sessions.
3. Enable per-operation confirmation for write scopes so the desktop can approve or reject each request.
4. For stdio clients, use the exact bridge and Store paths displayed by the MCP Bridge UI.
5. For HTTP clients, configure the listen IP, client address, port, Origins, and Client ID on the HTTP page, generate a token, and start the managed service.
6. For CC Switch, generate or rotate the Token, then copy the generated JSON from the HTTP page. The copied JSON includes that Token and must be treated as a secret.

### stdio Example

Build the development sidecar first:

```bash
cargo build --locked -p portmate-mcp
```

Most MCP hosts use a configuration similar to the following. Replace both paths with the exact values shown in the MCP Bridge UI:

```json
{
  "mcpServers": {
    "portmate": {
      "command": "/absolute/path/to/portmate-mcp",
      "args": ["--stdio"],
      "env": {
        "PORTMATE_STORE_PATH": "/absolute/path/to/portmate-store.sqlite3",
        "PORTMATE_MCP_CLIENT_ID": "my-mcp-client"
      }
    }
  }
}
```

The bridge reloads the Store and desktop IPC endpoint before each JSON-RPC envelope. A desktop restart or IPC token rotation therefore normally does not require restarting a long-lived stdio client.

### HTTP Mode

- The default listener is `127.0.0.1:8787`, with MCP available at `/mcp`.
- Listener presets include `127.0.0.1`, `0.0.0.0`, `::1`, and `::`, plus a custom numeric IP.
- The client address is persisted separately from the bind address, so a wildcard listener never generates an unusable `0.0.0.0` or `::` client URL.
- A non-loopback listener requires explicit remote-access approval.
- Every request requires a Bearer token or `X-PortMate-MCP-Token`; requests with an Origin also pass the configured allowlist.
- The bridge does not terminate TLS. Expose it remotely only on a trusted network or behind a correctly configured TLS reverse proxy.

Never place the HTTP token in a README, startup command, issue, log, or public MCP client example.

### CC Switch

The HTTP page generates the flat single-server JSON accepted by the CC Switch editor. It intentionally omits the outer `mcpServers` object. After you explicitly generate or rotate a Token, the JSON includes that Bearer Token:

```json
{
  "portmate": {
    "type": "http",
    "url": "http://192.168.33.222:8787/mcp",
    "headers": {
      "Authorization": "Bearer <token returned by PortMate>"
    },
    "tool_timeout_sec": 180
  }
}
```

The JSON is empty until a Token is explicitly generated or rotated. Treat copied JSON as a password: keep it out of source control, logs, screenshots, and shared documents. Rotating the Token invalidates the previous value.

### Grant Scopes

| Scope | Access |
| --- | --- |
| `read-sessions` | Read authorized sessions and runtime state |
| `read-logs` | Read and search logs for authorized sessions |
| `read-transfers` | List and inspect redacted transfer status |
| `read-tunnels` | List active SSH and PortMate-host forwards and SOCKS5 proxies |
| `read-scripts` | List MCP-enabled script summaries for authorized sessions; script bodies are omitted |
| `write-input` | Send text, keys, or commands to a terminal |
| `transfer` | Start, cancel, and retry file-transfer tasks; also implies `read-transfers` |
| `tunnel` | Create and stop SSH or PortMate-host forwards and SOCKS5 proxies; also implies `read-tunnels` |
| `manage-sessions` | Open or close sessions |
| `run-scripts` | Run saved MCP-enabled scripts in authorized sessions; also implies `read-scripts` |

Grants support expiration, revocation, allowed-session lists, and per-write confirmation. Allowed-session lists constrain session tools; host-route tools have no session target and are controlled directly by the `tunnel`/`read-tunnels` scopes. Every MCP write operation is audited.

### Custom Script Tools

Create and edit saved terminal scripts from `Tools -> Custom Scripts`. Each script has its own all-session or explicit-session boundary and an independent `Expose to MCP` switch. Desktop runs target an allowed session that is currently connected; scripts are sent through the existing session input lane and never start an arbitrary process on the desktop host.

MCP uses `list_custom_scripts` to receive only `id`, `name`, `description`, and `updatedAt`. It runs a saved script with `run_custom_script` and exactly two selectors: `sessionId` and `scriptId`. MCP cannot submit, read, or replace the script body. Execution requires the Client grant's `run-scripts` scope, the script's MCP switch, and the script's session boundary; normal write confirmation, post-authorization revalidation, and audit recording still apply. `run-scripts` does not imply `write-input`.

Script bodies are stored in the PortMate application Store and may also appear in terminal logging after execution. Do not save passwords, tokens, private keys, or other secrets in scripts.

### MCP Serial Break

`serial_send_break` asserts the hardware Break condition on the currently connected serial port for approximately 250 ms and then clears it. This is not text input, a newline, or `Ctrl+C`; devices commonly use it to interrupt startup, enter a bootloader, or trigger a device-specific serial control path. The target `sessionId` must identify both a Serial profile and its current live serial connection, and the USB-to-serial adapter and driver must support Break.

```json
{
  "sessionId": "board-uart"
}
```

The tool uses the `write-input` scope and remains subject to the grant's session boundary, per-write confirmation, commit-time authorization revalidation, and audit recording. A successful result contains `sent: true` and records a system event without device data. It fails closed when desktop IPC is unavailable, the session is disconnected or reconnecting, the target is not serial-backed, or the driver rejects Break. The existing desktop Send Break control does not require MCP.

For protocol payloads that must bypass terminal text handling, call `send_bytes` with `encoding: "base64"` or `encoding: "hex"`. PortMate sends the decoded bytes directly without appending a newline, and returns only a redacted byte summary. This is suitable for binary serial frames, bootloader packets, and raw TCP/Telnet payloads; Telnet escaping is applied only when required by the negotiated transport. The payload is bounded to 4 MiB and still requires the `write-input` grant.

```json
{
  "sessionId": "board-uart",
  "encoding": "hex",
  "data": "55 aa 00 ff"
}
```

### File Transfer Tools

`list_transfers` and `get_transfer` expose task IDs, protocol, progress, status, and timing while replacing both paths with `<redacted-path>`. `start_transfer` is the single transfer-start tool for SFTP, SCP, TFTP, XModem, YModem, and ZModem. Each call selects exactly one source form: a string `source` path, a virtual MCP file in `source: { kind: "mcp", fileName, contentBase64 }`, legacy top-level `fileName` plus `contentBase64`, or `uploadId` for a completed resumable upload. The virtual form sends bytes in the MCP request and never resolves a client path or requires a user-selected folder on the PortMate desktop host. At least one endpoint must use `remote:`, `ssh:`, or the constrained `load:` device receiver form; unprefixed string paths are local to the PortMate desktop host, and pure local-to-local copy is not exposed through MCP.

Upload with SFTP:

```json
{
  "sessionId": "edge-router",
  "protocol": "sftp",
  "source": "/home/operator/firmware.bin",
  "destination": "remote:/tmp/firmware.bin"
}
```

Download by reversing the sides, for example `source: "remote:/var/log/messages"` and `destination: "/home/operator/messages"`. SFTP and SCP may also copy between two `remote:` paths on the same authorized session. Use the returned task ID with `get_transfer`, `cancel_transfer`, or `retry_transfer`.

For a U-Boot-style device receiver, upload a local file with the matching Modem protocol and set the destination to `load:loadx`, `load:loady`, or `load:loadz`. `loadx` and `loady` are standard U-Boot commands; use `loadz` only when the target firmware provides that command. Optional validated query parameters add the load address and serial transfer rate:

```json
{
  "sessionId": "board-uart",
  "protocol": "ymodem",
  "source": "/home/operator/firmware.bin",
  "destination": "load:loady?address=0x80000000&baud=115200"
}
```

PortMate sends the device command before starting the protocol. A `baud` parameter is accepted only for a connected serial session; PortMate switches the local port for the transfer and restores its original rate afterward. The endpoint grammar cannot contain an arbitrary shell command.

TFTP is also available directly from the desktop Transfer Tasks dialog. It starts a one-shot server on the PortMate host and drives U-Boot through any connected interactive session. MCP clients select `protocol: "tftp"` in `start_transfer` with either a local desktop `source` or inline content:

```json
{
  "sessionId": "board-uart",
  "protocol": "tftp",
  "source": {
    "kind": "mcp",
    "fileName": "firmware.bin",
    "contentBase64": "AAECAwQF..."
  },
  "destination": {
    "kind": "tftpboot",
    "address": "0x81800000",
    "deviceIp": "192.168.255.1",
    "serverIp": "192.168.255.2",
    "bindPort": 69
  }
}
```

`deviceIp` is required. `address`, `fileName`, `serverIp`, `bindHost`, `bindPort`, and `timeoutSeconds` are optional: the load address defaults to `${loadaddr}`, the requested name defaults to the local source name, `serverIp` can be inferred from the route to the device, the bind host defaults to the advertised server IP, the port defaults to 69, and the timeout defaults to 60 seconds. Explicit timeouts must be at least 5 seconds and have no application-defined upper limit. `bindPort=0` chooses a free port. The legacy `"destination":"load:tftpboot?deviceIp=..."` string remains supported. PortMate temporarily sends `setenv ipaddr`, `setenv serverip`, and `setenv tftpdstp` followed by `tftpboot`; it never sends `saveenv`.

The server accepts RRQ only from `deviceIp`, serves only the selected name, supports the common `blksize`, `tsize`, and `timeout` options, and closes on completion, cancellation, failure, or timeout. Ports below 1024 may require elevated privileges on Unix; `bindPort=0` or a port above 1023 avoids that requirement when the target U-Boot honors `tftpdstp`. Transfer starts are asynchronous. Use the returned ID with `get_transfer`; final `bytesDone`, `status`, and `message` report the transferred byte count, completion state, and any error.

For content produced on another MCP client, use the virtual source form of `start_transfer`. It sends one standard Base64 value without requiring the source to exist on the desktop host or granting access to a local folder. The decoded payload limit is 4 MiB. For larger files, use the resumable upload workflow below:

```json
{
  "sessionId": "board-uart",
  "protocol": "tftp",
  "source": {
    "kind": "mcp",
    "fileName": "firmware.bin",
    "contentBase64": "AAECAwQF..."
  },
  "destination": {
    "kind": "tftpboot",
    "deviceIp": "192.168.255.1",
    "serverIp": "192.168.255.2",
    "bindPort": 0
  }
}
```

For files up to 512 MiB, compute the whole-file SHA-256 and use the resumable upload workflow. `begin_content_upload` returns an upload ID and the maximum decoded chunk size:

```json
{
  "sessionId": "board-uart",
  "protocol": "tftp",
  "fileName": "firmware.bin",
  "sizeBytes": 8388608,
  "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "destination": {
    "kind": "tftpboot",
    "deviceIp": "192.168.255.1",
    "serverIp": "192.168.255.2",
    "bindPort": 0
  }
}
```

Append standard-Base64 chunks in order. `offset` is the decoded byte offset and must equal the previous response's `nextOffset`:

```json
{
  "uploadId": "the-begin-response-upload-id",
  "offset": 0,
  "contentBase64": "AAECAwQF..."
}
```

After the response reports `complete: true`, call `start_transfer` with `{"uploadId":"..."}`. TFTP destination options are validated and bound by `begin_content_upload`; do not add top-level `deviceIp` or related fields to the final start request. Call `cancel_content_upload` with the same argument to discard an incomplete upload. Active uploads share a 1 GiB declared-size quota and uploads older than 24 hours are removed when a new upload begins.

Both workflows stage content in the desktop application's private data directory, validate safe file names and transfer routes, and remove staged content after use. The bytes are not included in MCP audit records or returned as client-supplied paths. SFTP/SCP destinations use `remote:` or `ssh:`, for example `remote:/tmp/firmware.bin`; TFTP and Modem destinations use constrained `load:` endpoints. Only the final transfer start follows the normal MCP write approval policy, so individual chunks do not create approval prompts. Staged-content tasks cannot be retried after the staging file is removed; start a new upload instead.

### Route-Specific Forwarding And Proxy

`create_tunnel`, `list_tunnels`, and `stop_tunnel` expose both route boundaries. Use `egress: "ssh"` with a connected, authorized SSH or Tmux `sessionId`, or use `egress: "portmate-host"` without a session to reach TCP targets available directly from the machine running PortMate. Neither boundary modifies the operating system routing table.

Local forwarding sends one local listener to a fixed target through SSH:

```json
{
  "sessionId": "edge-router",
  "egress": "ssh",
  "mode": "local",
  "bindHost": "127.0.0.1",
  "bindPort": 15432,
  "targetHost": "db.internal",
  "targetPort": 5432,
  "label": "Database route"
}
```

Use `mode: "remote"` for server-side SSH forwarding. Use `mode: "dynamic"` for a route-specific SOCKS5 proxy; dynamic mode needs no fixed target and supports `bindPort: 0` to request an available local port. SSH dynamic routes may omit `routeRules` to allow every target; a non-empty list restricts exact domains, `*.example.com` suffixes, IP addresses, or IPv4/IPv6 CIDRs. A rule can also restrict one port.

```json
{
  "sessionId": "edge-router",
  "egress": "ssh",
  "mode": "dynamic",
  "bindHost": "127.0.0.1",
  "bindPort": 0,
  "routeRules": [
    { "host": "*.internal.example", "port": 443 },
    { "host": "10.20.0.0/16", "port": null },
    { "host": "2001:db8:42::/48", "port": 22 }
  ],
  "label": "Device network SOCKS5"
}
```

Binding a local listener to a non-loopback address exposes it to that interface. Keep per-write confirmation enabled unless the client and route policy are fully trusted.

For a fixed route reachable from the PortMate host, call the same tool with `egress: "portmate-host"`; omit `sessionId`. Host egress supports `local` and `dynamic`, but not SSH remote forwarding:

```json
{
  "egress": "portmate-host",
  "mode": "local",
  "bindHost": "127.0.0.1",
  "bindPort": 0,
  "targetHost": "192.168.33.222",
  "targetPort": 443,
  "allowRemoteBind": false,
  "label": "PortMate host route"
}
```

For a PortMate-host SOCKS5 proxy, `routeRules` must contain at least one allowed target. Non-loopback listeners such as `0.0.0.0` are rejected unless the call also sets `allowRemoteBind: true`; that flag is visible in the MCP approval target and audit record. Host routes are runtime-only and owned by the normalized MCP Client ID: another client cannot list or stop them. They survive terminal session disconnects and stop through `stop_tunnel` or when PortMate exits; they are never restored from a saved profile.

```json
{
  "egress": "portmate-host",
  "mode": "dynamic",
  "bindHost": "0.0.0.0",
  "bindPort": 1080,
  "routeRules": [
    { "host": "192.168.33.0/24", "port": null },
    { "host": "service.internal", "port": 443 }
  ],
  "allowRemoteBind": true,
  "label": "Host network SOCKS5"
}
```

MCP agents that run in a container or on a separate machine cannot reach listeners bound on the desktop host. `tunnel_request` is the MCP data plane for that case: it sends one bounded raw TCP request through an existing client-owned PortMate-host route from the desktop host and returns the response bytes as standard Base64, so no host-side listener is involved. Fixed local routes use their configured target; dynamic SOCKS5 routes require `targetHost` and `targetPort` and enforce the tunnel `routeRules`. Only the MCP client that created the host route can send requests through it, and each call is a `tunnel`-scope write with its own audit record and optional per-write confirmation:

```json
{
  "tunnelId": "2a24...route-id",
  "encoding": "base64",
  "data": "R0VUIC9oZWFsdGh6IEhUVFAvMS4xDQpIb3N0OiBkZXZpY2UuaW50ZXJuYWwNCg0K",
  "timeoutMs": 10000,
  "closeWrite": true
}
```

For a dynamic route, add `"targetHost"` and `"targetPort"` to select the SOCKS5 destination. `closeWrite` defaults to `true` and half-closes the request stream after writing, which lets request/response services such as HTTP detect the end of the request; set it to `false` for protocols that keep sending and rely on `timeoutMs`. The request payload is bounded to 4 MiB, the response is read until the remote closes or `timeoutMs` (100 ms to 30 s) elapses, and the result reports `sentBytes`, `receivedBytes`, `responseBase64`, `truncated`, and `timedOut`.

## Build

Build the production frontend:

```bash
npm run build
```

Build native desktop packages:

```bash
npm run desktop:build
```

Cross-build an unsigned Windows GNU x86_64 portable archive from Linux with a configured
MinGW-w64 toolchain and Windows GNU `libsodium.a`:

```bash
npm run desktop:build:windows-gnu
```

Bundles are written below `target/release/bundle/`. Before publishing, run the installation, upgrade, rollback, signing, and artifact gates in [RELEASE.md](./RELEASE.md) on each target platform. A successful local source build is not, by itself, a releasable package.

## Verification

Normal development gates:

```bash
npm test
npm run build
npm run test:release-source
npm run test:release-upgrade
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Focused compatibility gates:

```bash
npm run test:terminal-compat
npm run test:vttest-compat
npm run test:tmux-workflow
npm run test:tmux-version-compat
npm run test:workspace-ui
npm run test:ssh-server-compat
npm run test:ssh-gssapi-compat
npm run test:tcp-telnet-server-compat
```

MCP client matrices:

```bash
npm run test:mcp-stdio-client
npm run test:mcp-http-client
npm run test:mcp-typescript-client
npm run test:mcp-python-client
npm run test:mcp-go-client
npm run test:mcp-rust-client
npm run test:mcp-ruby-client
npm run test:mcp-java-client
npm run test:mcp-kotlin-client
npm run test:mcp-csharp-client
npm run test:mcp-swift-client
npm run test:mcp-sdk-freshness
```

Docker compatibility matrices, Chrome/Playwright checks, native keyring probes, and desktop packaging require additional tools or the corresponding operating system. [RELEASE.md](./RELEASE.md) is the source of truth for the complete release gate.

## Repository Layout

```text
PortMate/
├── src/                    React/TypeScript desktop workspace
├── src-tauri/              Tauri application, command adapters, platform integration
│   └── src/
│       ├── backend_application.rs
│       ├── backend_automation.rs
│       ├── backend_security.rs
│       ├── backend_storage.rs
│       └── backend_transport.rs
├── crates/
│   ├── portmate-core/      Shared models, Store, host keys, grant policy
│   ├── portmate-mcp/       MCP stdio/HTTP bridge
│   ├── portmate-kdf/       Portable Vault KDF boundary
│   ├── portmate-keyring/   Cross-platform native keyring boundary
│   └── russh-sftp/         SFTP compatibility implementation used by PortMate
├── scripts/                Build, packaging, and compatibility scripts
├── tests/                  External server and protocol fixtures
└── .github/workflows/      Native CI and SDK freshness workflows
```

The Tauri root `lib.rs` contains only module registration and public re-exports. Transport, security, storage, automation, and application logic remain with their owning boundaries instead of accumulating in the crate root.

## Data and Privacy

- Profiles, custom scripts, runtime state, grants, and index data are stored in `portmate-store.sqlite3` under the platform application-data directory.
- The JSON compatibility snapshot, logs, exports, and IPC endpoint use bounded, atomic, or private-permission write paths as appropriate.
- Raw terminal logs can contain sensitive operational data. Enable them only when needed and review redaction before sharing a diagnostic bundle.
- MCP read responses remove credential references and sensitive local paths. Writes still require a grant and record the source Client ID, action, session, and final result.
- Report vulnerabilities privately through GitHub Security Advisories. Do not place credentials, private keys, production hostnames, or unredacted Stores in a public issue.

See [SECURITY.md](./SECURITY.md) for the security policy and reviewed dependency exceptions.

## Known Limitations

- PortMate is not yet a complete WindTerm or Bitvise replacement and does not currently promise a stable release channel.
- Real Microsoft Active Directory GSSAPI/PAC evidence is still pending. The current Samba matrix proves AD-compatible protocol coverage only.
- Remote Windows OpenSSH Sysmon and real macOS/FreeBSD SSH/SFTP/SCP/remote-forward evidence require external hosts.
- Virtual PTYs cannot replace physical serial/USB serial and modem power-loss, unplug, and line-state testing.
- Final artifacts have not yet been signed with Windows Authenticode or Apple Developer ID and notarized.
- MCP HTTP has no built-in TLS termination.

PortMate does not substitute simulated results for these gates. Required environments and current boundaries are tracked in [PROGRESS.md](./PROGRESS.md#剩余外部验证门槛).

## Documentation

- [PLAN.md](./PLAN.md): product goals and key design decisions
- [PROGRESS.md](./PROGRESS.md): implementation record, compatibility matrices, and remaining gates
- [RELEASE.md](./RELEASE.md): release checklist
- [SECURITY.md](./SECURITY.md): security policy, dependency exceptions, and vulnerability reporting
- [README.zh-CN.md](./README.zh-CN.md): Simplified Chinese README

## Contributing

Before submitting a change:

1. Keep behavior within the existing ownership boundaries and avoid unrelated refactors.
2. Add tests for user-visible behavior and protocol or security boundaries.
3. Run frontend, Rust, and compatibility checks proportional to the change.
4. Never commit real credentials, private keys, tokens, production Stores, unredacted logs, or signing material.
5. Open an issue before a large protocol, storage-format, or security-policy change and describe its compatibility and migration impact.

## Acknowledgements

PortMate's workspace and interaction model is inspired by WindTerm. Its SSH trust and key-management model draws from Bitvise and OpenSSH. The terminal uses xterm.js, the desktop runtime uses Tauri, and the bundled default monospace font is JetBrains Mono.

## License

PortMate is licensed under the [Apache License 2.0](./LICENSE). The bundled JetBrains Mono font is distributed under the SIL Open Font License 1.1; see [THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt](./THIRD_PARTY_LICENSES/JetBrainsMono-OFL.txt).
