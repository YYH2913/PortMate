# Changelog

All notable PortMate changes are recorded here. PortMate is still alpha software; a source build
or an unsigned artifact is not a production release. The complete release gates are maintained in
[RELEASE.md](./RELEASE.md).

## [0.1.1] - 2026-08-14

### Added

- Added resumable MCP content uploads for remote transfers. A client can upload an authenticated,
  ordered Base64 stream of up to 512 MiB into private staging and then start a normal authorized
  transfer without requiring the source file to exist on the desktop host.
- Exposed transfer and SSH forwarding lifecycle operations through MCP grants, including scoped
  status, cancellation, retry, route policy, and audit records.
- Added CC Switch JSON generation, configurable HTTP listeners, embedded HTTP tokens, random Client
  IDs, grant expiry editing, and managed MCP sidecar lifecycle controls.
- Added terminal row timestamps, configurable text export destinations, text/hex/split inspection,
  Insert/Normal interaction modes, semantic command coloring, parameter hints, and JetBrains Mono.
- Added the detached serial analyzer, common baud-rate selection, exact RX/TX capture, protocol
  framing, and transfer-driven device load commands.

### Changed

- Session creation now uses separate username, host/IP, and port fields. Jump Hosts are managed as
  explicit removable hops, while connection state is shown on each tab without forcing a reconnect
  dialog at startup.
- SFTP remote paths now preserve the server's default-directory semantics and validate drag/drop
  destinations at the operation boundary. Completed transfer notices can be dismissed or expired.
- SSH, SFTP, SCP, Telnet, raw TCP, Tmux, vttest, full-screen terminal programs, and nine official MCP
  SDK families now have broader compatibility and failure matrices.
- The Rust desktop backend and MCP bridge were split into transport, security, storage, monitoring,
  transfer, migration, and protocol owners while preserving public command and data contracts.

### Fixed

- Fixed VMware WebKitGTK blank windows by applying the detected software-rendering fallback before
  GTK initialization.
- Fixed WebGL cursor checks that sampled the hidden phase of a blinking bar cursor, and retained the
  correct bar/block cursor transition between Insert and Normal modes.
- Fixed SSH reconnect racing Local/Dynamic tunnel listener teardown, which could leave the previous
  port bound and fail restoration with `Address already in use`.
- Fixed serial analyzer close behavior and connected-state refresh, terminal input focus after MCP
  Client ID generation, modal interaction layering, and multiple path-preservation issues.
- Fixed native CI portability for Windows OpenSSL/NASM, macOS temporary paths and filesystem
  fixtures, SSH teardown, and current MCP SDK versions.

### Security

- New SSH/proxy passwords, private-key passphrases, OneKey secrets, and Profile Vault private keys
  are written only to the master-password-protected Stronghold vault. SQLite stores references, not
  plaintext, and legacy native-keyring user entries can only migrate toward Stronghold.
- Unsaved SSH credentials use a 30-second one-use backend handle bound to the requesting window,
  session, and SSH configuration digest. MCP cannot submit passwords, passphrases, or these handles.
- MCP content upload staging is private, quota-bound, integrity checked, client-owned, session-grant
  scoped, and excluded from logs. HTTP and desktop IPC retain bounded messages, concurrency,
  deadlines, origin/token checks, and fail-closed authorization.
- Saved custom-script bodies are sent only to the selected terminal transport. Structured events,
  screen summaries, text/JSONL logs, desktop command results, MCP responses, and audit records retain
  only a placeholder, transmitted byte count, and authorized script identifier as applicable.
- Dependency gates now reject moderate-or-higher npm advisories and unreviewed RustSec changes. The
  remaining RSA advisory and upstream warnings are documented with exact mitigations in
  [SECURITY.md](./SECURITY.md).

### Migration

- Application data migrates atomically from `dev.portmate.app` to `dev.portmate.desktop` when only
  the legacy directory contains state. If both directories contain state, startup fails closed and
  preserves both Stores for manual review.
- Existing workspace, panel, command-history, session cache, SQLite, and credential-journal formats
  are normalized on load. Legacy `keychain:` user credentials remain readable and deletable until
  the user performs the explicit one-way Stronghold migration.
- Keep a backup of the application-data directory before upgrading. Do not run an older binary
  against the only upgraded Store; test rollback on a copy as required by [RELEASE.md](./RELEASE.md).

### Known Limitations

- Windows MSI/NSIS and macOS app/DMG still require successful native-runner installation evidence;
  final Windows Authenticode and Apple signing/notarization have not been performed.
- Microsoft Active Directory GSSAPI/PAC, real Windows OpenSSH remote Sysmon, real macOS/FreeBSD SSH
  transfer and remote forwarding, and physical serial/modem fault matrices still require external
  hosts or hardware.
- This remains an alpha release. Do not use it unattended for production-critical access or file
  transfer until all applicable [RELEASE.md](./RELEASE.md) gates are complete.

## [0.1.0] - 2026-08-11

### Added

- Initial alpha implementation of the Tauri terminal workspace, SSH/Shell/Serial/Telnet/TCP/Tmux
  sessions, SFTP/SCP and modem transfers, tunnels, logs, Sysmon, and the permissioned MCP bridge.

### Known Limitations

- Initial alpha packages were development artifacts and did not satisfy the complete cross-platform
  release, signing, upgrade, or external hardware gates.
