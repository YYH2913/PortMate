# PortMate

PortMate is a Rust-first terminal workbench for serial, SSH, shell, Telnet, raw TCP, file-transfer, and MCP-controlled co-debugging workflows.

This repository currently contains the active desktop implementation slice:

- Tauri v2 + React/TypeScript desktop shell
- WindTerm-style workbench UI with resource/file/session/history/send panes and modal settings dialogs
- Shared Rust domain model for sessions, logs, transfers, triggers, Sysmon snapshots, MCP grants, SSH identity policy, and profile-scoped host keys
- Profile-level SSH host key isolation with `hostKeyAlias`, independent from system `~/.ssh/known_hosts`
- Real SSH, local Shell PTY, raw TCP, Telnet, serial, SFTP, SCP, and Tmux attach/list/pane inspection paths in the Tauri backend
- SSH password/public-key/keyboard-interactive/ssh-agent authentication, with profile-first identity ordering
- Local, remote reverse, and dynamic SOCKS5 SSH tunnel runtime, local/remote Sysmon snapshots, SFTP-backed file manager, trigger actions, and transfer task tracking
- SSH `profile-vault` private keys, optional saved passwords/passphrases, and the live MCP IPC token stored in the OS keyring with only `secretRef` metadata persisted in files/SQLite
- SQLite-backed local session/profile/host-key persistence in the desktop app data directory, with a JSON compatibility export
- Standalone `portmate-mcp` stdio bridge exposing MCP resources, tools, prompts, and local IPC control over JSON-RPC

## Workspace

```text
crates/portmate-core   Shared domain model, SSH trust policy, tests
crates/portmate-mcp    MCP stdio bridge for external AI/MCP hosts
src                    React workbench UI
src-tauri              Tauri desktop shell and backend commands
```

## Development

```bash
npm install
npm run dev
```

The browser preview runs at `http://127.0.0.1:1420`. It uses empty local state outside Tauri and the Tauri command backend inside the desktop app.

## How To Use The Current Build

1. Start the desktop app:

   ```bash
   npm run desktop:clean
   ```

   Use `npm run desktop` if you are not launching from a snap-packaged VS Code environment.

2. The app starts empty. No sample sessions, logs, transfers, host keys, or MCP audit rows are injected.

3. Use the top menu bar for WindTerm-style entry points:

   - `Session`: new session, session settings, duplicate tab, startup sessions, layout restore
   - `Edit`: copy, paste, paste dialog, search, online search
   - `View`: session tree, explorer pane, shell pane, quick bar, split views, focus mode
   - `Terminal`: sync input, command sender, completion, free type, lock screen
   - `Transfer`: SFTP, SCP, X/Y/ZModem
   - `Tools`: forwarding, Sysmon, triggers, logs, MCP bridge, key manager
   - `Preferences`: global settings, font, color scheme, tab color, transparency, mouse behavior

4. Open settings from any settings-oriented menu item, for example `会话 -> 会话设置` or `工具 -> 终端设置`.

5. A modal settings window opens. `会话 -> 新建会话` and `会话 -> 会话设置` open the session settings dialog; `工具 -> 终端设置` opens the terminal settings dialog.

   - Session name, group, tags
   - SSH host, port, username, `HostKeyAlias`, host key policy, trust scope, identity file, auth order, agent and forwarding behavior
   - SSH profile-vault private key import plus optional saved password/passphrase into the system keyring; saved profiles keep only generated `secretRef` values
   - Serial port, baud rate, parity, flow control, DTR/RTS, reconnect
   - Shell/TCP/Telnet/Tmux fields when those session types are selected
   - SSH/TCP/Telnet/Serial reconnect keeps a disconnected runtime in `Reconnecting` and retries in the background until the user closes or manually reconnects the session
   - Trigger matching and actions for timeline marks, notifications, highlights, local commands, and send-text automation
   - Terminal type, font, rows, cols, scrollback, theme
   - Logging formats, redaction, path template
   - Transfer protocols
   - Workbench/global preferences modeled after WindTerm settings

6. Click `保存` or `保存并连接`. SSH credentials are requested in a separate connection dialog, so the same saved profile can be reused with a different username/password/passphrase on the next connection. SSH private-key paths are configured under `SSH/Tmux -> 公钥`; passphrases are requested only at connect time.

Saved sessions, runtime state, host-key trust decisions, audit rows, and recent logs are written to the desktop app data directory as `portmate-store.sqlite3`. The SQLite store keeps the original JSON snapshot for compatibility and mirrors sessions, runtimes, events, transfers, host keys, MCP grants, audit records, timeline marks, and Sysmon snapshots into normalized query tables. Event, audit, timeline, and Sysmon mirrors are synchronized incrementally by primary key inside the same transaction as the authoritative snapshot, including deletion of trimmed rows; small mutable tables are atomically rebuilt. A `portmate-store.json` compatibility export is also maintained for inspection and older tooling. Terminal/global preferences are stored locally by the frontend.

The `传输 -> SFTP/SCP 传输` dialog supports local file copy, protocol-native SFTP upload/download/remote copy, SCP upload/download, remote-to-remote SCP copy through SSH command channels, and X/Y/ZModem in-band transfers over connected runtimes. Use `remote:/path/file` or `ssh:/path/file` on either side to mark the remote path. ZModem uses the `zmodem2` state machine. Automatic remote modem transfers use lrzsz `rx`/`sx`, `rb`/`sb`, or `rz`/`sz`, gate protocol input behind per-transfer READY markers, and switch SSH PTYs to raw mode for binary integrity. Cancelling a running modem transfer sends three CAN bytes immediately, silent marker/byte waits poll cancellation every 100 ms, and waits fail as soon as the owning session leaves `Connected` so an old worker cannot leak across runtime reconnection.

The left `文件管理器` panel can browse local directories and, when the active SSH/Tmux session is connected, remote directories through the SFTP subsystem. It supports local/remote dual panes, refresh, parent navigation, recursive new-directory creation, recursive delete with root/current-directory guards, rename, chmod, file properties, local-to-remote upload, remote-to-local download, and pane-to-pane file drag-and-drop. Native desktop file drops can copy external files or recursively expand directory trees into either the local pane or a connected remote pane through the same resumable transfer queue; empty directories are preserved, symbolic links are skipped, and self-copy/target-collision/oversized-batch guards run before the target is modified.

The bottom sender supports text and real byte-array Hex sending. Hex mode uses a dedicated `send_bytes` backend command, so serial/TCP payloads such as `FF 00 80` are not rewritten as UTF-8 text. Telnet Hex/raw byte sends escape `0xFF` as doubled IAC on the wire, while text uses NVT CRLF/`CR NUL` encoding. Incoming Telnet negotiation and NVT decoding retain state across fragmented socket reads, including terminal-type subnegotiation and EOF flushing of a pending CR.

The terminal canvas supports select-to-copy plus right-click/middle-click paste when the desktop webview has clipboard permission.

The `工具 -> 端口转发` dialog supports local forwarding, remote reverse forwarding, and dynamic SOCKS5 forwarding.

The `工具 -> Tmux` dialog reads remote `tmux list-sessions` and `tmux list-panes` output through the connected SSH/Tmux runtime, then can attach or create a named tmux session in the active terminal.

The `工具 -> Sysmon` action samples the local machine for Shell/Serial/TCP sessions and executes a Linux `/proc` sampler through the active SSH/Tmux connection for remote sessions.

The `搜索 -> 会话搜索` and `搜索 -> 日志搜索` dialogs search the current desktop session set and recent loaded logs. `工具 -> 密钥管理器` manages PortMate host keys with scope/profile filters, batch delete, batch copy-to-profile, host-key alias/host/port/scope/profile/label editing, and OpenSSH `known_hosts` import/export. Its Client Keys workspace searches and groups profile identities by profile or source, copies selected identities to another profile, reorders or removes references across profiles while protecting Jump Host references, imports private keys into profile-vault, and adds visible ssh-agent identities individually or in batches. `工具 -> MCP Bridge` opens the grant manager for MCP client IDs, scopes, allowed sessions, and recent audit records.

Run the desktop application:

```bash
npm run desktop
```

If you launch from snap-packaged VS Code and GTK/WebKit loads `/snap/core20` libraries, use the sanitized launcher:

```bash
npm run desktop:clean
```

Build desktop bundles:

```bash
npm run desktop:build
```

The terminal renderer is pinned to `@xterm/xterm@6.0.0` with matching current `@xterm/addon-*` packages.

To run the MCP bridge:

```bash
cargo run -p portmate-mcp
```

To let the standalone stdio bridge read the desktop store directly, pass the SQLite store path:

```bash
PORTMATE_STORE_PATH=/path/to/portmate-store.sqlite3 cargo run -p portmate-mcp
```

When the desktop app is running, it writes `portmate-ipc.json` next to the store. The live IPC token is stored in the OS keyring and the endpoint file contains a `tokenRef` rather than the token itself when keyring access is available. The MCP bridge uses that endpoint to forward `send_text`, `send_key`, `run_command`, `open_session`, `close_session`, `start_transfer`, `create_tunnel`, `list_tmux_state`, and `attach_tmux` to the live desktop runtime. If the desktop IPC file is unavailable, read tools fall back to the store snapshot.

Write tools are denied by default. For a trusted local development run with an empty grant store:

```bash
PORTMATE_MCP_TRUSTED=1 PORTMATE_MCP_CLIENT_ID=portmate-local cargo run -p portmate-mcp
```

The same bridge can expose JSON-RPC over local HTTP for clients that cannot spawn stdio servers. It only accepts loopback bind addresses, validates `Origin` when present, and requires either `Authorization: Bearer <token>` or `X-PortMate-MCP-Token: <token>`. If `PORTMATE_MCP_HTTP_TOKEN` is not set, the bridge creates or reuses `keychain:mcp-http-token` in the OS keyring.
The desktop `工具 -> MCP Bridge` dialog shows the default HTTP endpoint, Origin, startup command, tokenRef, and can generate or rotate the keyring token.
Streamable HTTP clients that send `Accept: application/json, text/event-stream` receive JSON-RPC responses with `MCP-Protocol-Version`. Clients that prefer SSE can open `GET /mcp` with `Accept: text/event-stream` for an authenticated event stream containing endpoint and PortMate state events; `POST /mcp` with only `Accept: text/event-stream` returns the JSON-RPC result as a `message` event.

```bash
PORTMATE_STORE_PATH=/path/to/portmate-store.sqlite3 \
PORTMATE_MCP_HTTP=1 \
PORTMATE_MCP_HTTP_ADDR=127.0.0.1:8787 \
PORTMATE_MCP_HTTP_ORIGINS=http://127.0.0.1:8787 \
cargo run -p portmate-mcp -- --http
```

## Verification

```bash
npm test
npm run build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

On Linux, Tauri desktop compilation also requires WebKitGTK/GTK development packages. Debian/Ubuntu package names are typically:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libcairo2-dev libgdk-pixbuf-2.0-dev
```

Without those packages, `cargo check -p portmate` stops at pkg-config errors for libraries such as `cairo` or `gdk-pixbuf-2.0`.

## SSH Trust Model

PortMate intentionally does not use the system `known_hosts` file as the source of truth. Each SSH profile owns:

- `hostKeyPolicy.alias`, equivalent in purpose to OpenSSH `HostKeyAlias`
- profile/project/user scoped trusted host keys
- multiple trusted host key algorithms for the same profile
- `identityPolicy.identitiesOnly`, so the selected profile key is tried before broad agent enumeration
- `agentPolicy`, including agent forwarding and offer order

This prevents common embedded lab failures where several devices reuse `192.168.1.10:22` but have different host keys.

## Current Implementation Boundary

The current slice is usable but not yet a full terminal replacement. Implemented runtime paths include SSH PTY shell with password/public-key/keyboard-interactive/ssh-agent authentication, multi-hop SSH Jump Host backend connection chains with per-hop host-key verification, per-hop independent password/passphrase `secretRef`, per-hop identity selection, per-hop inherited or custom host-key mode/alias/trust scope/rotation/IP-check policy, target host-key scan over multi-hop chains, Jump Host host-key confirmation for the first untrusted or changed hop in the chain, session-settings editing, and initial SSH reconnect loops, local Shell PTY with resize, raw TCP, Telnet socket mode with basic option negotiation, Telnet CRLF/raw byte IAC handling plus Raw TCP byte preservation with loopback mock regressions, TCP/Telnet automatic reconnect with loopback state-transition coverage, Serial initial reconnect loops, runtime `lastDisconnect` and `lastDisconnectReason` diagnostics surfaced in summaries, SQLite, and the desktop toolbar, serial open/read/write with runtime port enumeration, DTR/RTS, Break, real Hex byte sending, recent serial RX/TX timestamp and Hex monitoring, Tmux list/pane inspection and attach, local/remote/dynamic SSH tunnels with running-list, stop controls, connection counters, byte counters, last-error status, enabled-tunnel reconstruction after SSH reconnect, and passive remote-listener health checks that restore revoked forwards while preserving IDs, labels, and ports, local and SSH remote Sysmon snapshots, SFTP-backed local/remote dual-pane file browsing, trigger timeline/notification/highlight/local-command/send-text actions, local/SFTP/SCP/X/Y/ZModem transfer queue tasks with retry/speed metadata, profile-level B/s rate limits, per-session background queued scheduling, `.portmate-part` resume for local copy plus SFTP upload/download/remote copy, SCP upload/download, and remote-command SCP copy, full session queue view, batch cancel/retry controls, and live progress/cancel for local/SFTP/SCP copy loops, remote-command SCP copy with target-size polling, and X/Y/ZModem block loops, append-only raw/text/jsonl log shards, profile-vault private key/password/passphrase storage through the OS keyring, MCP IPC token storage through the OS keyring, MCP grant management, profile persistence, profile-scoped host-key trust with connection-failure confirmation dialog and one-shot trust, host/client key manager workflows for known_hosts import/export, host-key scope/profile filtering, host-key field editing, batch host-key delete/copy-to-profile, grouped and filtered client identities, cross-profile client-key copy/reorder/reference removal, protected Jump Host references, profile-vault private-key import, and individual or batch ssh-agent identity addition, modal settings, MCP manifests/tools/resources, stdio/loopback HTTP MCP bridge, and live desktop IPC for trusted MCP control. Automated integration coverage includes isolated OpenSSH TOFU, same-endpoint host-key mismatch blocking, public-key/PTY/native-SFTP write and transfer/SCP workflows, a real two-hop Jump Host direct-tcpip chain with per-endpoint TOFU and second-hop key-mismatch blocking, all three SSH tunnel modes, SOCKS5 protocol negotiation, `socat` virtual serial PTY binary I/O and PTY replacement reconnect behavior, and Telnet/raw TCP loopback and reconnect behavior.

The OpenSSH integration matrix also exercises host-key mismatch blocking followed by explicit TOFU `allowRotation` history retention, `MaxAuthTries` identity ordering and per-key diagnostics, three independent identities across a two-hop Jump Host chain with hop/endpoint diagnostics for first-hop refusal, second-hop direct-tcpip refusal, stalled handshakes at both hops and the final target, per-hop identity rejection, and target identity exhaustion, a real isolated ssh-agent across disabled/unfiltered/`IdentitiesOnly`/fingerprint-filtered policies including protection against same-comment fingerprint bypass, local/dynamic/remote tunnel target rejection followed by recovery on the original tunnel, server-side remote-forward removal followed by passive detection and restoration, best-effort local cleanup after a repeated cancel is rejected, automatic tunnel reconstruction after SSH reconnect with preserved identity/port and per-tunnel bind-failure isolation, SFTP/SCP upload/download and SFTP remote-copy resume from pre-existing `.portmate-part` prefixes, cancellation of rate-limited SFTP and SCP uploads followed by resumable retries, rejected server-side writes reaching a failed terminal state, interrupted SFTP/SCP uploads failing cleanly and resuming after SSH reconnect, plus lrzsz X/Y/ZModem uploads and downloads over a raw PTY with per-transfer READY/DONE gating and exact XModem upload truncation. A mixed-server matrix adds user-space russh password and keyboard-interactive first hops followed by independent OpenSSH public-key hops and targets.

Still pending: remote-forward listener probes for non-Linux servers without `ss`/`netstat`, richer file conflict policies and remote-directory download workflows, terminal compatibility test baselines, broader MCP HTTP client matrix testing, broader transfer/serial integration matrices, and a portable Stronghold-style vault backend for environments where the native OS keyring is unavailable or intentionally disabled.
