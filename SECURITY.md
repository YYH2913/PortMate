# Security Policy

## Reporting

Report suspected vulnerabilities privately through the repository's GitHub security advisory
channel. Do not include credentials, private keys, production hostnames, or unredacted PortMate
Store data in a public issue.

## Credential Boundary

PortMate separates user login credentials from internal process credentials:

- SSH and proxy passwords, private-key passphrases, OneKey secrets, and Profile Vault private keys
  are written only to the master-password-protected IOTA Stronghold vault. SQLite stores opaque
  `stronghold:` references, not plaintext.
- The native OS keyring is reserved for internal material such as the persistent MCP HTTP token and
  bundle-signing identity. Legacy user `keychain:` references remain readable and deletable so an
  existing installation can connect and migrate them one way to Stronghold. General credential APIs
  cannot create or overwrite those entries.
- Unsaved credentials submitted by the trusted desktop prompt are exchanged for a 30-second,
  one-use handle bound to the requesting window, session ID, and current SSH configuration digest.
  Locking the vault, closing a session, or deleting a Profile clears applicable handles. MCP calls
  reject passwords, passphrases, and credential handles.
- Secret reads occur only inside the Rust desktop backend. The Tauri frontend has no command that
  returns stored plaintext credentials.

This boundary protects credentials at rest from ordinary filesystem readers and keeps saved
plaintext out of SQLite, logs, MCP, and long-lived frontend state. It does not protect against an
attacker who controls the current OS account, injects or debugs the PortMate process, replaces a
trusted PortMate binary, or controls the kernel. Use OS account isolation, full-disk encryption, and
trusted release artifacts for those threats.

## SSH Host-Key Boundary

SSH/Tmux host-key changes are blocked by default. The per-profile
`reconnectIgnoreHostKeyChanges` option defaults to `false`; enabling it temporarily
accepts changed keys for previously trusted endpoints during automatic reconnect,
including Jump Hosts, without replacing saved trust. Each bypass records its observed
fingerprint. Initial/manual connections, host-key scans, and unseen endpoints retain
the configured policy. This opt-in removes protection against server impersonation
and man-in-the-middle attacks for changed keys during automatic reconnect.

## MCP Transfer And Route Boundary

- File-transfer and forwarding mutations require an explicit session-scoped grant and are written
  to the MCP audit log. Grants may require desktop confirmation for every mutation.
- Transfer status returned through MCP removes source and destination paths. Transfer lifecycle
  events record only protocol, direction, and final status; they do not copy paths or backend error
  messages into terminal logs.
- MCP transfer requests must include at least one `remote:` or `ssh:` endpoint. This prevents the
  session-scoped transfer grant from becoming a general local filesystem copy primitive. SFTP/SCP
  remote-to-remote copy remains limited to the same authorized session.
- Cancel, retry, and stop requests are authorized against the session recorded for the backend task
  or live tunnel ID. A caller-supplied `sessionId` cannot reassign ownership.
- MCP forwarding supports SSH local/remote forwarding and SSH dynamic SOCKS5 through an authorized
  connected SSH/Tmux session. It also supports fixed TCP forwarding and route-scoped dynamic SOCKS5
  through the PortMate host's own reachable network, without any terminal session. Host egress is
  represented by `egress: "portmate-host"`, is isolated by normalized MCP client ID, and does not
  support SSH-style remote listeners. None of these modes modifies the host routing table or
  installs transparent system-wide proxy rules. Binding a listener outside loopback can expose it
  to the attached network and should retain desktop confirmation.

## Dependency Gates

`npm run test:dependency-audit` rejects moderate-or-higher npm advisories.
`npm run test:rust-dependency-audit` runs `cargo-audit` against the committed `Cargo.lock`, rejects
every unreviewed vulnerability or warning, and requires reviewed findings to retain their exact
advisory ID, package, and version. A new finding, a changed dependency version, or a finding that
disappears all require the reviewed set to be updated rather than passing silently.

The only allowed RustSec vulnerability is `RUSTSEC-2023-0071` for
`rsa 0.10.0-rc.18`. Patched RSA 0.10.0 is available, but Russh 0.65.1 and
its pinned `ssh-key 0.7.0-rc.11` still require the prerelease dependency family.
The existing, version-specific exception remains; no new vulnerability is exempted.
PortMate mitigates the affected private RSA
operation for local SSH identities by using Russh's external-signer API and
`RandomizedSigner::try_sign_with_rng` with `getrandom::SysRng`. RSA-SHA512, RSA-SHA256, and legacy
SSH-RSA signatures all request fresh blinding randomness. Ed25519/ECDSA identities retain Russh's
normal signer, ssh-agent identities are signed outside this process, and the libssh GSSAPI backend
uses libssh/OpenSSL rather than the affected Rust private-key operation.

The mitigation is covered by a random-source request regression and a real OpenSSH RSA login test.
The exception is intentionally treated as required evidence: when the advisory is withdrawn, the
dependency disappears, or a patched RSA release is adopted, the audit gate fails until the stale
exception and mitigation note are reviewed.

The Stronghold dependency uses a provenance-pinned local `iota-crypto 0.23.2`
patch to upgrade k256 to 0.14 and remove the vulnerable `crypto-bigint 0.5`
branch. The locked graph uses patched `crypto-bigint 0.7.11` and `ctutils 0.4.8`.
Only API/entropy-trait adapters change; snapshot encryption and key formats stay
unchanged. Upstream curve/SLIP-0010 vectors, entropy failure checks and a snapshot
generated by the unmodified old dependency graph are regression-tested. See
`crates/iota-crypto/PORTMATE_ORIGIN.md`; no timing advisory is ignored for this patch.

The security refresh also upgrades Russh, bundled SQLite, Tokio, rustix, libz-sys
and async-executor. Fresh npm and RustSec audits remain required; historical MCP
SDK client fixtures are intentionally isolated compatibility tests, not application
dependencies or exposed SDK servers.

The workspace patches `libsodium-sys-stable` 1.24.0 locally because even upstream
1.24.3 still requires the ZIP 8.x build dependency affected by
`RUSTSEC-2026-0367`, `RUSTSEC-2026-0368` and `RUSTSEC-2026-0369`. The patch only
upgrades that build dependency to ZIP 9.0.5; bindings, build logic and the signed
native source archive remain unchanged. Provenance and removal criteria are in
`crates/libsodium-sys-stable/PORTMATE_ORIGIN.md`. No ZIP vulnerability is exempted.

## Reviewed Upstream Warnings

- `RUSTSEC-2026-0543` reports maintenance status for `chrono 0.4.45`, which supplies
  serialized UTC timestamps across the app, Store, and MCP. The audit reviews that
  exact warning. A replacement must preserve the timestamp and date formats that
  existing sessions and logs use.

- The remaining GLib warning is confined to Tauri's Linux GTK3/WebKit dependency graph. PortMate does not
  call the affected `glib::VariantStrIter` API directly; migration depends on upstream runtime
  dependency updates. Withdrawn GTK3 unmaintained advisories are removed from the reviewed set.
- `zvariant 5.15.0`, `zvariant_derive 5.15.0` and `zbus_names 4.3.4` are unmaintained
  standalone crates after their APIs moved into zbus 6. They are confined to the
  FreeBSD keyring backend through `zbus-secret-service-keyring-store 1.0.1` and
  `secret-service 5.2.0`; no compatible upstream backend release uses zbus 6 yet.
  Linux uses the separate native libdbus backend, while Windows/macOS use their
  platform-native stores. These are exact maintenance-warning reviews, not
  vulnerability exceptions; migration must be reviewed when the backend updates.
- Yanked package warnings are not exempted. The lockfile uses compatible patch releases of
  `aes`, `chacha20`, `der`, `serialport`, and `wnaf`; reintroducing their yanked versions fails the audit gate.
- Remaining unmaintained crates are exact transitive dependencies of Tauri build/runtime support or
  IOTA Stronghold. Their full fingerprints remain in the audit script so additions, removals, or
  version changes cannot pass without review.

## Local terminal elevation / 本地终端提权

Elevation is a **per-connection desktop action**, not a saved permission. A Shell
profile's `elevated` flag defaults to false for older profiles. The dedicated desktop
command accepts only workspace windows and binds consent to the exact saved Shell
configuration. Generic/MCP/IPC opens and OneKey credentials cannot authorize it;
startup recovery skips it even after vault unlock. Live elevation-mode changes
require disconnecting first. PortMate itself remains at its launch integrity.

Windows desktop builds embed gsudo 2.6.1 from a SHA-256-pinned upstream archive.
There is no runtime download or PATH/system installation. A launch extracts the
helper into a fresh application-data directory, verifies its executable hash and
pins it and its directory against replacement/reparse traversal until child reap.
Shell executable paths must resolve to local `.exe`/`.com` files. Arguments are
passed directly to the helper, not interpolated into PowerShell or `Start-Process`.
UAC and any existing system/gsudo cache policies remain under OS/user control;
PortMate does not enable caching or alter global gsudo settings.

Linux/macOS uses system sudo with a fresh-authentication request, root target and
target HOME. Its fixed password prompt is recognized by existing private-input
handling (no synchronized password broadcast, input history or outbound password
payload logging). Passwords are entered at that prompt, not in profile fields.
Cancellation, missing dependencies or authentication failure never starts a normal
fallback shell. Closing the PTY and reaping the bridge follows the existing bounded
Shell cleanup; deliberately detached child processes are not globally terminated.

Once a user opens an elevated terminal, **already-authorized MCP writes and inputs
sent by scripts/triggers in that session inherit its privileges**. Review those
permissions before accepting the confirmation. The UI shield is a launch preference,
not an administrator-token assertion. Host-side custom scripts do not gain elevation
through this setting.

本地提权只针对用户手动确认的单个 Shell 连接，不提升整个应用，也不允许 MCP、
启动恢复或导入配置自行授权。Windows 内置组件有固定哈希校验与运行期间文件锁；
Linux/macOS 在终端中通过 sudo 认证。请在确认前检查该会话已有的 MCP 写入授权、
脚本和触发器；发送到已提权终端的输入同样具有高权限。拒绝或失败时不会自动降级。
