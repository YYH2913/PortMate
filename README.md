# PortMate

PortMate is a Rust-first terminal workbench for serial, SSH, shell, Telnet, raw TCP, file-transfer, and MCP-controlled co-debugging workflows.

This repository currently contains the active desktop implementation slice:

- Tauri v2 + React/TypeScript desktop shell
- WindTerm-style workbench UI with resource/file/session/history/send panes and modal settings dialogs
- Shared Rust domain model for sessions, logs, transfers, triggers, Sysmon snapshots, MCP grants, SSH identity policy, and profile-scoped host keys
- Profile-level SSH host key isolation with `hostKeyAlias`, independent from system `~/.ssh/known_hosts`
- Real SSH, local Shell PTY, raw TCP, Telnet, serial, SFTP, SCP, and Tmux attach/list/pane inspection paths in the Tauri backend
- SSH password/public-key/keyboard-interactive/ssh-agent authentication, with profile-first identity ordering
- Profile-level HTTP CONNECT and SOCKS5 proxies for SSH, Tmux, TCP, and Telnet, with optional Basic or username/password authentication
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
   - `Mode`: remote mode, local mode, synchronized input, free type, lock screen
   - `Transfer`: SFTP, SCP, X/Y/ZModem
   - `Tools`: forwarding, Sysmon, triggers, logs, MCP bridge, key manager
   - `Preferences`: global settings, font, color scheme, tab color, transparency, mouse behavior

4. Open settings from any settings-oriented menu item, for example `会话 -> 会话设置` or `工具 -> 终端设置`.

5. A modal settings window opens. `会话 -> 新建会话` and `会话 -> 会话设置` open the session settings dialog; `工具 -> 终端设置` opens the terminal settings dialog.

   - Session name, group, tags
   - SSH host, port, username, `HostKeyAlias`, host key policy, trust scope, identity file, auth order, agent and forwarding behavior
   - SSH profile-vault private key import plus optional saved password/passphrase into the system keyring; saved profiles keep only generated `secretRef` values
   - SSH/Tmux/TCP/Telnet profile proxy settings for HTTP CONNECT or SOCKS5, with optional HTTP Basic or SOCKS5 username/password authentication. Proxy passwords are transactionally written to the native keyring or unlocked Stronghold fallback and only `secretRef` metadata is persisted. SSH host-key scans use the same route as the real connection; with Jump Hosts, the proxy carries only the first physical hop and later hops continue through SSH `direct-tcpip` channels.
   - Serial port, baud rate, parity, flow control, DTR/RTS, reconnect delay, and optional receive-idle timeout
   - Shell/TCP/Telnet/Tmux fields when those session types are selected
   - Telnet BINARY and NAWS negotiation switches. Both default on for legacy and new profiles; accepted BINARY directions independently control inbound NVT CR decoding and outbound newline conversion, while NAWS sends the latest terminal dimensions after negotiation and every later resize. TERMINAL-TYPE replies use the profile's configured terminal type.
   - SSH/TCP/Telnet/Serial reconnect keeps a disconnected runtime in `Reconnecting` and retries in the background until the user closes or manually reconnects the session. SSH/Tmux, TCP/Telnet, and Serial reload and normalize the latest saved profile before every retry; SSH endpoint/authentication/health edits, TCP/Telnet host/port or protocol edits, and Serial port/line/health edits are not frozen at disconnect time. A connection-setting change during an in-flight attempt invalidates that result, while disabling reconnect or changing the transport stops the pending worker. All three transports read the latest reconnect flag and 100-60,000 ms delay while waiting, so disabling reconnect or changing the delay affects the pending worker without waiting for an old deadline. SSH/Tmux profiles also persist a protocol KeepAlive toggle, 1-3,600 second interval, and 1-20 unanswered-message limit, defaulting to a 1,000 ms reconnect delay and 30/3 KeepAlive thresholds. TCP/Telnet profiles persist an OS keepalive toggle, idle time, probe interval, and retry count, defaulting to a 1,000 ms reconnect delay and 30/10/3 keepalive values. Serial profiles persist an optional 1-86,400 second receive-idle timeout, defaulting to a 1,000 ms reconnect delay and disabled/60 seconds. Serial idle monitoring observes incoming bytes without writing protocol-agnostic heartbeat data to the device. Legacy profiles receive all defaults automatically.
   - Trigger matching and actions for timeline marks, notifications, highlights, local commands, send-text automation, custom links, and sound
   - Terminal type, font, rows, cols, scrollback, theme
   - Synchronized-input protocol filters, newline mode, inter-target delay, and explicit batch-send prefix/suffix
   - Logging formats, redaction, path template
   - Transfer protocols
   - Workbench/global preferences modeled after WindTerm settings

6. Click `保存` or `保存并连接`. SSH credentials are requested in a separate connection dialog, so the same saved profile can be reused with a different username/password/passphrase on the next connection. SSH private-key paths are configured under `SSH/Tmux -> 公钥`; passphrases are requested only at connect time.

Saved sessions, runtime state, host-key trust decisions, audit rows, and recent logs are written to the desktop app data directory as `portmate-store.sqlite3`. The SQLite store keeps the original JSON snapshot for compatibility and mirrors sessions, runtimes, events, transfers, host keys, MCP grants, audit records, timeline marks, and Sysmon snapshots into normalized query tables. Event, audit, timeline, and Sysmon mirrors are synchronized incrementally by primary key inside the same transaction as the authoritative snapshot, including deletion of trimmed rows; small mutable tables are atomically rebuilt. A `portmate-store.json` compatibility export is also maintained for inspection and older tooling. Terminal/global preferences are stored locally by the frontend.

The frontend persists a versioned workspace snapshot containing horizontal/vertical pane bindings, the active session, and validated tab colors, while migrating the earlier split localStorage keys. Startup mode can connect no sessions, the last restored panes, or a configured session list after the first profile load; targets are deduplicated, stale IDs are discarded, and credential-requiring sessions are opened sequentially. `会话 -> 还原布局` reloads and reconciles the saved snapshot against the current profile set.

Synchronized input broadcasts each queued input batch from its source pane to the other connected pane sessions exactly once. `工具 -> 终端设置 -> 同步输入` filters additional targets by protocol and configures protocol-aware/preserved/LF/CRLF newlines, a bounded delay between targets, and bounded prefix/suffix text for explicit batch sends. Interactive XTerm input, including its native bracketed keyboard paste, remains an unframed stream; menu, context-menu, and middle-click paste each receive the prefix and suffix exactly once. The source is never filtered out, batches retain FIFO order, and disabling synchronization immediately cancels remaining extra targets without dropping or delaying subsequent source input. Settings persist locally, but the synchronization switch itself always starts disabled so reopening PortMate cannot unexpectedly broadcast keystrokes.

The session automation editor manages multiple contains/regex triggers and multiple ordered actions per trigger. Timeline marks, notifications, highlights, send-text, local commands, custom-link templates, and bell/chime/alert sounds share typed frontend/Rust models. Runtime visual effects are emitted to the desktop immediately; command and send-text actions remain on the existing backend dispatch paths, and all matches retain system-event/timeline diagnostics.

Terminal streams can also be appended to profile-configured `raw`, `txt`, and `jsonl` shards below the desktop data directory's `logs/` root. Raw logging stores exact pre-decoding SSH channel, PTY, TCP/Telnet socket, or serial bytes on input and exact post-encoding wire bytes for successful user text/byte sends, Telnet negotiation replies, and modem frames on output. Telnet IAC/subnegotiation/NVT bytes are retained once before filtering, while CRLF and IAC escaping are reflected in outbound references. A per-session lane keeps outbound transport writes and event offsets in the same order; each `bytesRef` is exact, while a shared shard does not claim cross-direction causal ordering during concurrent input and output. System/control diagnostics from lifecycle, trigger, reconnect, transfer, and tunnel paths use a non-blocking single-wakeup sink with a bounded 4,096-event outbox. It writes each queued event once to redacted Text/JSONL, emits it live without ever appending Raw, and drains and joins on normal shutdown; backlog overflow or worker disconnection is retained as `loggingError` instead of growing memory without a limit or silently losing later diagnostics. SQLite/Raw/Text/JSONL degradation after a successful write is reported through `loggingError` without changing the send into a failure that callers might retry, and structured binary events retain only payload length rather than reversible Hex. New profiles start with logging and Raw disabled because Raw bytes are intentionally not redacted. Raw appends are serialized per final shard path, and new `bytesRef` values include a SHA-256 digest so a deleted/recreated or modified shard cannot silently resolve an old reference to different bytes; legacy path/offset/length references remain readable. `工具 -> 日志管理` enumerates only regular shard files under that root, supports path/format filtering, reads a bounded tail preview (`raw` and invalid UTF-8 as Hex), and batch-deletes fully validated selections while pruning empty directories. Symlinks, traversal paths, unsupported extensions, oversized scans, previews, and delete batches are rejected or skipped.

Content search in the log manager scans persisted Text/JSONL shards, including history no longer present in the bounded event store. Searches can target all eligible shards or the selected subset and return path, line, byte offset, and bounded context. Query length, selected paths, matches, per-file bytes, and total scanned bytes are bounded; raw shards remain Hex-preview only and are never silently interpreted as text.

Selected raw, Text, and JSONL shards can be archived without deleting the source files. The log manager streams up to 1,000 validated paths and 512 MiB of source data into an atomic `.tar.gz`, writes a per-file SHA-256 manifest inside the archive, and finalizes a matching `.sha256` sidecar. Archive entries stay below `logs/`, and path traversal, symlinks, unsupported extensions, and truncated files are rejected.

Each profile can also set `retentionDays` from 0 (disabled) through 3,650 days. PortMate checks matching profile shards in the background at startup and at most hourly while logging, deletes only regular files whose modification time is older than the configured cutoff, and prunes empty directories. Retention-enabled custom paths must contain `{session}` or `{profile}` so one profile cannot claim an unscoped shared log path; legacy profiles default to disabled retention.

The same dialog exports a session handoff as an atomic `.tar.gz` plus `.sha256` sidecar. Each archive contains redacted bundle metadata, event JSONL, platform/store diagnostics, and a manifest with per-file SHA-256 by default. Raw `bytesRef` segments require both disabling redaction and explicitly enabling raw inclusion; segment and total uncompressed-size limits prevent unbounded exports. The existing MCP `export_session_bundle` JSON response remains compatible and uses the same `summary.lastLine` and event-text redaction boundary.

The `传输 -> SFTP/SCP 传输` dialog supports local file copy, protocol-native SFTP upload/download/remote copy, SCP upload/download, remote-to-remote SCP copy through SSH command channels, and X/Y/ZModem in-band transfers over connected runtimes. Use `remote:/path/file` or `ssh:/path/file` on either side to mark the remote path. Queue rows retain partial progress, show localized terminal states and failure details, and provide a copyable diagnostic for failed tasks; bounded failure summaries are also written to the session event stream. ZModem uses the `zmodem2` state machine. Automatic remote modem transfers use lrzsz `rx`/`sx`, `rb`/`sb`, or `rz`/`sz`, gate protocol input behind per-transfer READY markers, and switch SSH PTYs to raw mode for binary integrity. Cancelling a running modem transfer sends three CAN bytes immediately, silent marker/byte waits poll cancellation every 100 ms, and waits fail as soon as the owning session leaves `Connected` so an old worker cannot leak across runtime reconnection.

The left `文件管理器` panel can browse local directories and, when the active SSH/Tmux session is connected, remote directories through the SFTP subsystem. It supports local/remote dual panes, Ctrl/Command toggle selection, Shift range selection, select-all, recursive new-directory creation, guarded batch delete, single-item rename/chmod/properties, multi-file or directory upload/download, and pane-to-pane drag-and-drop. File and directory batches preserve empty directories, skip symbolic links, use the resumable per-session transfer queue, and apply one shared `fail`/`overwrite`/`skip`/`rename` conflict policy before modifying the target. Native desktop drops use the same policy and safety limits. Remote directory downloads are recursively planned through SFTP and reject root paths, unsafe relative paths, target type conflicts, and oversized batches.

The bottom sender supports text and real byte-array Hex sending. Hex mode uses a dedicated `send_bytes` backend command, so serial/TCP payloads such as `FF 00 80` are not rewritten as UTF-8 text. Telnet Hex/raw byte sends always escape `0xFF` as doubled IAC. Text uses NVT CRLF/`CR NUL` encoding until the server accepts client-side BINARY; inbound NVT CR decoding is independently disabled after PortMate accepts server-side BINARY. Incoming Telnet negotiation and decoding retain state across fragmented socket reads, including profile-driven terminal-type subnegotiation, NAWS with escaped `0xFF` dimension bytes, and EOF flushing of a pending NVT CR.

Serial sessions keep a bounded in-memory capture of the latest 512 RX/TX frames and at most 1 MiB of exact wire bytes. The history pane can filter by direction, Hex, or ASCII without reconstructing binary data from lossy terminal text. The capture survives automatic reconnect and remains available after disconnect, but is not persisted unless the user explicitly exports the visible frames. Export writes an atomic, unredacted JSONL file plus a SHA-256 sidecar under the app data `exports/` directory; oversized individual frames are visibly marked with captured and original lengths. Clearing the capture invalidates in-flight polling responses so removed frames cannot reappear.

Changing a saved profile from one protocol to another is rejected while the session is connecting, connected, or reconnecting. Disconnect the session first; this prevents new protocol encoding and status metadata from being applied to an older live transport. Settings within the current protocol remain editable while connected.

Long-running stores bound non-terminal history as well as session events. PortMate retains 5,000
session events per session, 5,000 audit records per session/global scope, 2,000 timeline marks,
1,024 Sysmon snapshots, and 1,000 terminal transfer tasks, with a small batched-trim allowance for
frequently appended histories. Queued and running transfers are never evicted. Desktop and
standalone MCP loading trim every oversized event/history scope to its exact limit and rebuild the
event-count cache before exposing an older snapshot.

The terminal canvas supports select-to-copy plus right-click/middle-click paste when the desktop webview has clipboard permission.

The `工具 -> 端口转发` dialog supports local forwarding, remote reverse forwarding, and dynamic SOCKS5 forwarding. Remote-forward health probes use Linux `/proc/net/tcp` or `ss`, FreeBSD `sockstat`, macOS `lsof`, and a successful `netstat -ltn` fallback. A present but incompatible probe tool is treated as unsupported instead of an empty listener table, preventing repeated rebind attempts on BSD-style systems.

The `工具 -> Tmux` dialog reads remote `tmux list-sessions` and `tmux list-panes` output through the connected SSH/Tmux runtime, then can attach or create a named tmux session in the active terminal. Shared SSH helper-command capture rejects stdout above 4 MiB and stderr above 64 KiB before appending the overflowing channel chunk, so abnormal Tmux, tunnel-probe, or Sysmon output cannot grow desktop memory without bound or be silently parsed as a truncated result.

The `工具 -> Sysmon` workspace samples the local Linux machine for non-SSH profiles and detects the operating system behind an active SSH/Tmux connection. Remote sampling uses bounded Linux `/proc`, macOS `top`/`vm_stat`, FreeBSD `sysctl`, or Windows PowerShell/CIM collectors; a failed Unix `uname` probe falls back to an encoded, fixed Windows probe without interpolating profile or user input. macOS and FreeBSD network rates use duplicate-safe `netstat -ibn` counters, while Windows consumes structured marker-delimited JSON and validates it again in Rust. Its compact summary shows CPU, memory, load average, aggregate RX/TX rate, and uptime; tabular views show up to eight CPU-ranked processes (name only, never command arguments), 16 mounted filesystems, and 32 deduplicated network interfaces with per-interface rates and totals. A fourth trend view plots CPU/memory utilization or aggregate RX/TX rates from the persisted session history. It loads the newest 120 samples by default through a bounded `1..=240` query, deduplicates them by session and timestamp, and merges a successful manual refresh into the visible series immediately. A checkable activity applet beside the current session status samples immediately and then every 10 seconds, exposes CPU/memory pressure without covering the terminal, opens the full workspace on demand, skips overlapping requests, and stops when its remote session disconnects or the active session changes. Refresh failures keep the last valid applet and workspace snapshot visible. Unsupported remote operating systems return an explicit error instead of a zero-filled snapshot. The structured details are retained in the session store, SQLite v4 `details_json` mirror, session bundle, and MCP Sysmon resource; legacy summary-only snapshots deserialize with empty detail lists.

The `搜索 -> 会话搜索` and `搜索 -> 日志搜索` dialogs search the current desktop session set and recent loaded logs. `工具 -> 密钥管理器` manages PortMate host keys with scope/profile filters, batch delete, batch copy-to-profile, host-key alias/host/port/scope/profile/label editing, and OpenSSH `known_hosts` import/export. Its Client Keys workspace searches and groups profile identities by profile or source, copies selected identities to another profile, reorders or removes references across profiles while protecting Jump Host references, imports private keys into profile-vault, and adds visible ssh-agent identities individually or in batches. A compact identity inspector edits label/source/path/fingerprint metadata, shows immutable ID and Jump Host/shared-secret impact, rotates profile-vault private keys, and can remove either only the reference or an unshared backing secret. Identity mutations persist the profile before orphan cleanup and never overwrite a shared secret in place. The same workspace can create, unlock, lock, and rotate the master password of an Argon2id-protected IOTA Stronghold portable vault; `stronghold:` references route Profile secrets to its encrypted snapshot, while automatic storage still prefers the native OS keyring and falls back only when Stronghold is already unlocked. Rotation requires the unlocked vault, verifies the current password, requires a different replacement of at least eight characters, and commits the encrypted snapshot with the replacement key before changing the in-memory provider, preserving existing secrets and references if the commit fails. Snapshot open/save/rekey operations also use a cross-process OS file lock and compare the last loaded SHA-256 version before writing, so a stale second PortMate instance cannot overwrite a rotated vault and must reopen it instead. An inline preflight can migrate all or one SSH/Tmux/TCP/Telnet profile's proxy password plus SSH target passwords, passphrases, Profile Vault private keys, and per-Jump-Host credentials in either direction between native keyring and Stronghold. One source reference is copied once even when shared by several selected fields; Stronghold destinations use one batch snapshot commit, native destinations are read back exactly, all profile references switch in one copy-on-write store commit, and source records are cleaned only after their global usage reaches zero. Reserved MCP HTTP/IPC token references are always excluded. Before any provider write, PortMate durably commits and reads back a `synchronous=FULL` SQLite migration journal containing credential-slot projections and generated references but no secret body or body hash. The Profile KV snapshot, mirror tables, store revision, and `profiles-committed` checkpoint form one transaction and the only commit point. After restart, an all-old projection can only roll back unreferenced target copies; an all-new projection verifies every target and exact source/target contents before continuing source cleanup. Mixed, missing, changed, corrupt, or unavailable evidence preserves both providers and freezes automatic changes. Key Manager exposes the pending state and an explicit `核对并恢复` action after the portable vault is unlocked; an active or corrupt journal blocks another migration and related Profile/secret/identity mutations. The same recovery panel exports an atomic JSON diagnostic plus SHA-256 sidecar with before/current/after credential slots, provider presence, reference counts, and a boolean content comparison. It never includes secret bodies, body hashes, or an unverified raw journal payload, including when the journal itself is corrupt. `工具 -> MCP Bridge` opens the grant manager for MCP client IDs, scopes, allowed sessions, and recent audit records.

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

The terminal runtime is loaded as a separate Vite chunk. The current production build emits an
approximately 425 kB main JS chunk and a 381 kB xterm chunk, with xterm CSS split alongside it;
the previous approximately 805 kB single-chunk warning is eliminated without raising the warning
threshold.

To run the MCP bridge:

```bash
cargo run -p portmate-mcp
```

The stdio transport accepts newline-delimited JSON messages up to 1 MiB, excluding the line
delimiter. Oversized input is discarded through its terminating newline, returns a JSON-RPC parse
error, and does not desynchronize the following request.

Both stdio and HTTP preserve an explicit JSON-RPC `null` ID, reject non-string/number/null IDs and
non-structured `params`, and cap a batch at 128 items before dispatch. This prevents a small request
from amplifying into an unbounded sequence of tool calls or responses.

JSON-RPC responses and SSE JSON data are serialized through a 64 MiB bounded writer shared with the
desktop IPC response contract. An oversized single response becomes a `-32603` error with the same
request ID; an oversized batch or SSE state update is replaced without emitting the original data.

To let the standalone stdio bridge read the desktop store directly, pass the SQLite store path:

```bash
PORTMATE_STORE_PATH=/path/to/portmate-store.sqlite3 cargo run -p portmate-mcp
```

When the desktop app is running, it writes `portmate-ipc.json` next to the store. The live IPC token is stored in the OS keyring and the endpoint file contains a `tokenRef` rather than the token itself when keyring access is available. The MCP bridge uses that endpoint to forward `send_text`, `send_key`, `run_command`, `open_session`, `close_session`, `start_transfer`, `create_tunnel`, `list_tmux_state`, and `attach_tmux` to the live desktop runtime. If the desktop IPC file is unavailable, read tools fall back to the store snapshot. Long-running stdio bridges reload the latest valid snapshot and atomically published endpoint before each JSON-RPC message, so desktop restarts and token/address rotation do not require restarting the bridge; removing the endpoint immediately disables live forwarding.

Endpoint publication uses a synced same-directory temporary file and atomic replacement. Unix files
are forced to mode `0600`, including the plaintext-token fallback used when native keyring storage
fails; replacement does not follow a pre-existing symlink, and a failed publish keeps the previous
endpoint intact.

The bridge only loads a regular endpoint file up to 64 KiB (private owner-only mode on Unix), and
requires its `storePath` to match `PORTMATE_STORE_PATH`, its address to be loopback, and its keyring
reference to use the dedicated `keychain:ipc-*` namespace. Desktop IPC requests and responses are
bounded to 1 MiB and 64 MiB, with bounded connect, write, and total response waits.

Write tools are denied by default. For a trusted local development run with an empty grant store:

```bash
PORTMATE_MCP_TRUSTED=1 PORTMATE_MCP_CLIENT_ID=portmate-local cargo run -p portmate-mcp
```

When live desktop IPC is available, the desktop store is the authoritative grant source. Every
authenticated write attempt is audited with the MCP client ID, exact tool, session, scope, and
`invalid`/`denied`/`authorized`/`succeeded`/`failed` outcome. Tool arguments, command text,
passwords, passphrases, and file-path bodies are not copied into audit details. An explicit grant
enables its configured scopes; `PORTMATE_MCP_TRUSTED=1` additionally enables the documented local
bootstrap only while the grant store is empty.

The desktop IPC reader caps each request at 1 MiB and requires the complete payload within five
seconds; response writes use the same timeout. Oversized, incomplete, malformed, and invalid-token
requests are rejected before command dispatch and do not create audit records.

The same bridge can expose JSON-RPC over local HTTP for clients that cannot spawn stdio servers. It only accepts loopback bind addresses, validates `Origin` when present, and requires either `Authorization: Bearer <token>` or `X-PortMate-MCP-Token: <token>`. If `PORTMATE_MCP_HTTP_TOKEN` is not set, the bridge creates or reuses `keychain:mcp-http-token` in the OS keyring.
The desktop `工具 -> MCP Bridge` dialog shows the default HTTP endpoint, Origin, startup command, tokenRef, and can generate or rotate the keyring token.
Streamable HTTP clients that send `Accept: application/json, text/event-stream` receive JSON-RPC responses with `MCP-Protocol-Version`. Clients that prefer SSE can open `GET /mcp` with `Accept: text/event-stream` for an authenticated event stream containing endpoint and PortMate state events; `POST /mcp` with only `Accept: text/event-stream` returns the JSON-RPC result as a `message` event.
POST requests require `Content-Type: application/json` (parameters such as `charset=utf-8` are
accepted). An explicit `MCP-Protocol-Version` must match the server's negotiated `2025-06-18`
version; the header remains optional for initialization and older clients, and is allowed by CORS
preflight responses.

The HTTP bridge allows at most 64 concurrent connections, including long-lived SSE streams. A
complete request must arrive within five seconds, each response/SSE write has a five-second socket
timeout, and excess connections receive `503 Service Unavailable`. Non-SSE responses explicitly
close the HTTP/1.1 connection. Request headers are parsed strictly with a 64 KiB/128-field limit;
ambiguous duplicate framing or authentication headers, unsupported `Transfer-Encoding`, malformed
headers, and bytes beyond the declared `Content-Length` are rejected before JSON-RPC dispatch.
Repeated list headers such as `Accept` remain supported, including standard quality values.

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
cargo test --workspace -- --test-threads=4
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

The current slice is usable but not yet a full terminal replacement. Implemented runtime paths include SSH PTY shell with password/public-key/keyboard-interactive/ssh-agent authentication, profile-level SSH reconnect delay and protocol KeepAlive thresholds, profile-level HTTP CONNECT/SOCKS5 routing with optional authentication for SSH/Tmux/TCP/Telnet, multi-hop SSH Jump Host backend connection chains with per-hop host-key verification, per-hop independent password/passphrase `secretRef`, per-hop identity selection, per-hop inherited or custom host-key mode/alias/trust scope/rotation/IP-check policy, target host-key scan over multi-hop chains, Jump Host host-key confirmation for the first untrusted or changed hop in the chain, session-settings editing, and initial SSH reconnect loops, local Shell PTY with resize, raw TCP, Telnet socket mode with directional BINARY, NAWS, profile TERMINAL-TYPE and NVT negotiation, profile-configurable reconnect delay and bounded OS TCP keepalive, Telnet CRLF/raw byte IAC handling plus Raw TCP byte preservation with loopback mock regressions, TCP/Telnet and Serial automatic reconnect with latest-profile reload and stale-attempt rejection, runtime `lastDisconnect` and `lastDisconnectReason` diagnostics surfaced in summaries, SQLite, and the desktop toolbar, serial open/read/write with runtime port enumeration, configurable reconnect delay and receive-idle detection, DTR/RTS, Break, real Hex byte sending, exact bounded Serial RX/TX capture with direction/Hex/ASCII filtering and atomic JSONL export, Tmux list/pane inspection and attach, local/remote/dynamic SSH tunnels with running-list, stop controls, connection counters, byte counters, last-error status, enabled-tunnel reconstruction after SSH reconnect, and passive remote-listener health checks that restore revoked forwards while preserving IDs, labels, and ports, bounded local Linux and remote Linux/macOS/FreeBSD/Windows Sysmon summaries with process/disk/interface detail views and CPU/memory/RX/TX history trends, SFTP-backed local/remote dual-pane file browsing, trigger timeline/notification/highlight/local-command/send-text actions, local/SFTP/SCP/X/Y/ZModem transfer queue tasks with retry/speed metadata, profile-level B/s rate limits, per-session background queued scheduling, `.portmate-part` resume for local copy plus SFTP upload/download/remote copy, SCP upload/download, and remote-command SCP copy, full session queue view, batch cancel/retry controls, and live progress/cancel for local/SFTP/SCP copy loops, remote-command SCP copy with target-size polling, and X/Y/ZModem block loops, append-only raw/text/jsonl log shards, profile-vault private key/password/passphrase/proxy-password storage through the OS keyring or Stronghold, MCP IPC token storage through the OS keyring, MCP grant management, profile persistence, profile-scoped host-key trust with connection-failure confirmation dialog and one-shot trust, host/client key manager workflows for known_hosts import/export, host-key scope/profile filtering, host-key field editing, batch host-key delete/copy-to-profile, grouped and filtered client identities, cross-profile client-key copy/reorder/reference removal, protected Jump Host references, profile-vault private-key import, and individual or batch ssh-agent identity addition, modal settings, MCP manifests/tools/resources, stdio/loopback HTTP MCP bridge, and live desktop IPC for trusted MCP control. Automated integration coverage includes isolated OpenSSH TOFU, same-endpoint host-key mismatch blocking, public-key/PTY/native-SFTP write and transfer/SCP workflows, live SSH reconnect-delay changes with tunnel restoration, authenticated and unauthenticated HTTP CONNECT/SOCKS5 transport forwarding and rejection handling, a real two-hop Jump Host direct-tcpip chain with per-endpoint TOFU and second-hop key-mismatch blocking, all three SSH tunnel modes, SOCKS5 protocol negotiation, `socat` virtual serial PTY exact binary capture/I/O, no-probe receive-idle detection, and reconnect migration to a different PTY path with live delay changes, plus Telnet/raw TCP loopback, configurable keepalive, and reconnect-delay behavior.

The OpenSSH integration matrix also exercises host-key mismatch blocking followed by explicit TOFU `allowRotation` history retention, `MaxAuthTries` identity ordering and per-key diagnostics, three independent identities across a two-hop Jump Host chain with hop/endpoint diagnostics for first-hop refusal, second-hop direct-tcpip refusal, stalled handshakes at both hops and the final target, per-hop identity rejection, and target identity exhaustion, a real isolated ssh-agent across disabled/unfiltered/`IdentitiesOnly`/fingerprint-filtered policies including protection against same-comment fingerprint bypass, local/dynamic/remote tunnel target rejection followed by recovery on the original tunnel, server-side remote-forward removal followed by passive detection and restoration, best-effort local cleanup after a repeated cancel is rejected, automatic tunnel reconstruction after SSH reconnect with preserved identity/port and per-tunnel bind-failure isolation, SFTP/SCP upload/download and SFTP remote-copy resume from pre-existing `.portmate-part` prefixes, cancellation of rate-limited SFTP and SCP uploads followed by resumable retries, rejected server-side writes reaching a failed terminal state, interrupted SFTP/SCP uploads failing cleanly and resuming after SSH reconnect, plus lrzsz X/Y/ZModem uploads and downloads over a raw PTY with per-transfer READY/DONE gating and exact XModem upload truncation. A mixed-server matrix adds user-space russh password and keyboard-interactive first hops followed by independent OpenSSH public-key hops and targets.

Still pending: real FreeBSD/macOS SSH hosts in the remote-forward integration matrix, a real Windows OpenSSH host for Sysmon, terminal compatibility test baselines, broader MCP HTTP client matrix testing, broader transfer/serial integration matrices, cross-platform file-path coverage, and native keyring/Stronghold fault-injection matrices on Windows/macOS/Linux. MCP IPC/HTTP tokens remain native-keyring records and are outside profile credential migration.
