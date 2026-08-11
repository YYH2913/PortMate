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
| Interactive workflow | WindTerm-style command completion and parameter hints, semantic command coloring, Quick Commands, OneKeys, free input, and synchronized input |
| SSH security | Profile-scoped host keys, TOFU and key-change blocking, Host/Client Key Managers, multi-hop Jump Hosts, ssh-agent, password/public-key/keyboard-interactive, and Linux libssh GSSAPI authentication |
| Files and transfer | SFTP/SCP file management, drag and drop, queues, throttling, cancellation, retry, resumable transfers, and X/Y/ZModem |
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

See [PROGRESS.md](./PROGRESS.md) for the detailed implementation record and remaining boundaries. See [RELEASE.md](./RELEASE.md) for release requirements.

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
5. Open transfers, tunnels, Tmux, Sysmon, the serial analyzer, key management, logs, or MCP Bridge from `工具 (Tools)`.

Each session tab has a connection marker. Green means connected; unavailable states are red, with the precise connecting, reconnecting, blocked, disconnected, or error diagnosis available in the tooltip. Startup recovery silently tries credentials already available to the Profile and does not show a reconnect dialog every time the app opens.

SSH, Tmux, TCP, Telnet, and Serial reconnect workers reload the latest Profile before the next attempt. Changes to connection settings, authentication policy, reconnect delay, or the reconnect switch do not need to wait for an obsolete retry cycle to finish.

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
5. For HTTP clients, configure the listen IP, port, Origins, and Client ID on the HTTP page, generate a token, and start the managed service.

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
- A non-loopback listener requires explicit remote-access approval.
- Every request requires a Bearer token or `X-PortMate-MCP-Token`; requests with an Origin also pass the configured allowlist.
- The bridge does not terminate TLS. Expose it remotely only on a trusted network or behind a correctly configured TLS reverse proxy.

Never place the HTTP token in a README, startup command, issue, log, or public MCP client example.

### Grant Scopes

| Scope | Access |
| --- | --- |
| `read-sessions` | Read authorized sessions and runtime state |
| `read-logs` | Read and search logs for authorized sessions |
| `write-input` | Send text, keys, or commands to a terminal |
| `transfer` | Create file-transfer tasks |
| `tunnel` | Create SSH tunnels |
| `manage-sessions` | Open or close sessions |

Grants support expiration, revocation, allowed-session lists, and per-write confirmation. Every MCP write operation is audited.

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

- Profiles, runtime state, grants, and index data are stored in `portmate-store.sqlite3` under the platform application-data directory.
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
