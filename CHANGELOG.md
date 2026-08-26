# Changelog

All notable PortMate changes are recorded here. PortMate is still alpha software; a source build
or an unsigned artifact is not a production release. The complete release gates are maintained in
[RELEASE.md](./RELEASE.md).

## [Unreleased]

## [0.1.6] - 2026-08-27

### Added

- Added regression coverage for low-latency terminal output, batched byte-cache updates,
  and cross-window input/rendering behavior.

### Changed

- Reduced the cross-channel terminal event grace period from 250 ms to 32 ms now that canonical
  live packets are emitted first.
- Batched Hex/byte inspector cache commits and replaced repeated bounded-frame scans with indexed
  duplicate checks, keeping high-rate serial output off the interactive rendering path.

### Fixed

- Unified MCP HTTP sidecar Client ID resolution with saved grants. Legacy
  portmate-local configurations now adopt a single active grant automatically;
  explicit non-default IDs remain fail-closed when unauthorized, and standalone
  MCP sidecars refresh the same identity after Store changes.

- Stopped draining the physical serial driver after every interactive write, moved blocking serial
  writes off async runtime workers, and tracked them through session shutdown so XOFF, CTS stalls,
  USB driver faults, and login bursts cannot freeze the input queue or retain a Windows COM handle.

### Security

- Kept terminal live delivery and input ordering unchanged while bounding deferred byte-cache
  memory and retaining per-session duplicate and runtime-generation checks.

### Migration

- No Store schema migration is required from 0.1.5. Existing sessions, credentials, grants,
  terminal history, transfers, scripts, host keys, and workspace state load in place.

### Known Limitations

- The Windows GNU portable archive is unsigned cross-build evidence and does not replace native
  Windows MSVC, WebView2, Credential Manager, MSI/NSIS, Authenticode, or clean-machine tests.
- Physical serial drivers and remote endpoints can still add device or network latency that
  automated browser and loopback tests cannot reproduce completely.

## [0.1.5] - 2026-08-24

### Added

- Added regression coverage for Store-lock contention, exact printable/control-key ordering,
  detached-terminal input routing, prepared event identity, and delayed history ordering.

### Changed

- Unified printable keys, control keys, Enter, and paste requests under one bounded per-session
  native input queue with explicit coalescing barriers, and moved detached terminals onto the same
  frontend input pump as the main workspace.
- Coalesced short bursts of deferred desktop-input audit events before Store persistence, reducing
  per-character snapshot and log writes without moving persistence back onto the transport path.
- Moved inbound event recording, log shards, and trigger evaluation onto ordered per-session
  workers that drain ready bursts together, keeping SSH, serial, shell, TCP, and Telnet readers
  available while storage is busy.
- Routed live terminal bytes through one window-level listener, replayed the bounded pre-mount
  cache, and merged adjacent frames before handing them to xterm so split panes do not multiply
  native event work or serialize every transport read behind a separate parser callback.
- Reused the lazily loaded xterm WebGL module across terminal instances while retaining the DOM
  fallback for transparent terminals, Linux WebKitGTK, context loss, and incompatible GPU drivers.

### Fixed

- Published inbound transport bytes before event persistence and moved subsequent Store work out of
  transport readers, so remote echo and command output continue while audit or storage locks are busy.
- Removed persisted Store lookups from queued SSH, serial, shell, TCP, and Telnet keystroke
  enqueueing while retaining runtime-generation revalidation immediately before each write.
- Kept delayed inbound persistence in chronological event order when a later system or outbound
  event acquires the Store first, without regressing the session's last-activity timestamp.
- Preserved transport order when raw-byte and persisted text events arrive on different Tauri
  channels, preventing a new command prompt from rendering ahead of the preceding command output.
- Kept split UTF-8 and control-sequence bytes intact on the raw xterm path, validated live byte
  payloads before rendering, and stopped truncated frames from masquerading as complete output.
- Published Telnet application bytes after negotiation filtering, including bytes released when
  the negotiator finishes, instead of exposing protocol negotiation bytes or omitting the tail.

### Security

- Retained bounded per-session queues, runtime-generation revalidation, secret redaction, and
  ordered desktop audit persistence while moving terminal I/O off the shared Store lock.

### Migration

- No Store schema migration is required from 0.1.4. Existing sessions, credentials, grants,
  terminal history, transfers, scripts, host keys, and workspace state load in place.

### Known Limitations

- The Windows GNU portable archive remains unsigned cross-build evidence and does not replace
  native Windows MSVC, WebView2, Credential Manager, MSI/NSIS, Authenticode, or clean-machine tests.
- Physical serial devices and remote SSH/Telnet endpoints can add driver, network, or device echo
  latency that automated loopback and browser compatibility matrices cannot reproduce completely.

## [0.1.4] - 2026-08-24

### Added

- Added a PortMate local Shell session and the scoped run_local_command MCP tool without allowing
  MCP clients to choose an arbitrary host program, argument vector, or working directory.
- Added bounded UDP datagram request/response exchanges through client-owned PortMate-host routes,
  together with a complete Chinese MCP API reference for all tools, resources, prompts, scopes,
  parameters, transport modes, and removed legacy names.
- Added selective deletion of MCP audit history through the desktop authorization interface.
- Added semantic colors for unstyled terminal output, including status, severity, addresses, paths,
  values, and quoted strings, while preserving application-provided ANSI and TrueColor output.

### Changed

- Reworked terminal input into one session-level bridge path with bounded fast sends, explicit
  ordering boundaries for control sequences, and deferred persistence outside the transport hot
  path.
- Batched terminal byte notifications and isolated terminal rendering from unrelated workspace,
  transfer, and log updates to reduce interaction latency under sustained input and output.
- Cached semantic decorations per logical terminal line and avoided per-keystroke full-screen
  timestamp snapshots so unchanged output no longer rebuilds the visible terminal surface.
- Consolidated MCP transfer and host-route operations under start_transfer and the tunnel lifecycle,
  while retaining structured inline and resumable content sources.

### Fixed

- Released serial device handles before reconnect and serialized modem/TFTP command writes so a
  disconnected or bursty device does not leave the Windows COM port inaccessible or lose setup
  characters.
- Added U-Boot LWIP-compatible TFTP high-port commands, automatic port 69 fallback, and a structured
  destination contract that keeps deviceIp and other TFTP options at the correct boundary.
- Removed redundant per-keystroke serial capture refreshes and fixed several frontend/backend input
  queues that made interactive typing feel delayed.
- Reduced queued desktop input acknowledgements to a null result, removed a wire-byte clone, and
  retained one cancellable in-flight IPC boundary so rapid input merges without surviving session
  disconnect or deletion.

### Security

- Bounded terminal input and byte-event memory, UDP datagrams, TCP tunnel exchanges, MCP request
  bodies, and host-route targets while preserving per-client ownership, route rules, approval,
  commit-time revalidation, and audit records.
- Kept terminal input persistence asynchronous without weakening secret redaction or the ordered
  audit boundary for MCP and desktop writes.

### Migration

- No Store schema migration is required from 0.1.3. Existing sessions, grants, audit history,
  scripts, transfers, host keys, and encrypted credential references load in place.
- Preserve a backup of the application-data directory before upgrading and test rollback only on a
  copy of the Store, as required by RELEASE.md.

### Known Limitations

- The Windows GNU portable archive is unsigned cross-build evidence; it does not replace native
  Windows MSVC, WebView2, Credential Manager, MSI/NSIS, Authenticode, or clean-machine validation.
- Persistent UDP associations, SOCKS5 UDP ASSOCIATE, multicast, broadcast, DTLS, and QUIC session
  management are not implemented; udp_request carries one bounded datagram exchange.
- Physical serial hardware, U-Boot variants, real network routes, GSSAPI/Active Directory, and the
  complete native Windows/macOS compatibility matrix still require external validation.

## [0.1.3] - 2026-08-21

### Changed

- Removed the hard-coded 150-second ceiling from automatic TFTP transfers while retaining
  caller-configured operation deadlines.
- Stabilized terminal rendering and polling updates so interactive output remains responsive during
  long-running sessions and transfers.

### Fixed

- Preserved configured TFTP timeouts end to end instead of silently replacing them with a fixed
  upper bound.
- Reduced unnecessary terminal and workspace update churn that could delay visible input/output.

### Added

- Added the `tunnel_request` MCP tool, a bounded request/response data plane that sends raw bytes
  through an existing client-owned PortMate-host route from the desktop host and returns the
  response as standard Base64. Agents running in containers or on separate machines can now reach
  the route target without ever connecting to a listener bound on the desktop host. Fixed local
  routes use their configured target; dynamic SOCKS5 routes take `targetHost`/`targetPort` and
  enforce the route `routeRules`. Payloads and responses are bounded, reads honor a configurable
  timeout and optional write half-close, and each call is a `tunnel`-scope write with per-client
  ownership, approval, and audit coverage.

## [0.1.2] - 2026-08-17

### Added

- Added saved custom scripts with explicit desktop review, per-session targeting, version-bound
  execution, and an MCP capability that exposes only enabled scripts permitted by the grant.
- Added MCP host-route proxy capabilities so authorized clients can open, inspect, and close TCP
  routes reachable from the PortMate host without receiving desktop credentials.
- Added one-click CC Switch JSON for existing grants, including the reusable token already owned by
  that grant, and retained configurable remote HTTP listener settings.

### Changed

- Direct MCP content upload now supports larger resumable payloads and feeds the normal transfer
  pipeline, so remote load operations no longer require a source path on the desktop host.
- Terminal interaction keeps the active output anchored when Enter is pressed instead of moving the
  viewport to the oldest scrollback row.
- Native Rust test jobs now compile the backend with a mock runtime and without the desktop Wry
  feature, while production desktop builds retain the full Tauri/WebView runtime.

### Fixed

- Fixed existing MCP authorization tokens being unavailable for reuse in generated CC Switch
  configuration and made the complete configuration visible for previously created grants.
- Fixed transient libssh deadline tests by using an explicitly shared test cipher and preserving one
  total timeout across setup and authentication stages.
- Fixed packaged macOS conflict checks aborting inside AppKit by validating and migrating native
  smoke-test data directories before Tauri initializes the application runtime.
- Fixed Windows CI backend tests loading desktop-only runtime dependencies and failing before the
  Rust test harness with `STATUS_ENTRYPOINT_NOT_FOUND`.

### Security

- Custom-script MCP calls remain grant-scoped, require desktop approval where configured, bind the
  reviewed script version, and never expose saved script bodies through logs or MCP responses.
- Host-route proxy operations enforce explicit scope, bounded concurrency, target policy, audit
  records, and lifecycle ownership; they do not reveal stored SSH passwords or private keys.
- Reused MCP tokens are shown only in the local authorization UI that owns the grant and are embedded
  only when the user explicitly requests exportable CC Switch configuration.

### Migration

- No Store schema migration is required from `0.1.1`; existing sessions, grants, scripts, transfers,
  and encrypted credential references are loaded in place.
- Keep a backup of the application-data directory before upgrading and do not run an older build
  against the only upgraded Store, as described in [RELEASE.md](./RELEASE.md).

### Known Limitations

- The Windows GNU portable bundle is unsigned and provides cross-build and static package evidence;
  it does not replace Windows MSVC, WebView2, Credential Manager, MSI/NSIS, or signing smoke tests.
- GSSAPI/Active Directory, real remote Sysmon, physical serial/modem faults, and the complete
  cross-platform terminal/Tmux matrix still require external hosts, operating systems, or hardware.
- PortMate remains alpha software and should not be used unattended for production-critical access
  or transfer until all applicable [RELEASE.md](./RELEASE.md) gates pass.

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
- Fixed confirmed `run_custom_script` requests being dropped by the desktop approval event filter.
  The approval dialog now identifies the exact saved script by its trusted name and UUID.
- Fixed custom-script creation selecting a different script when another window saved concurrently.
  The backend now returns the exact committed script ID instead of requiring the UI to infer it.
- Fixed desktop custom-script execution accepting a body changed in another window after it was
  reviewed. Run requests now bind the displayed `updatedAt` version and fail before sending bytes.
- Fixed custom-script conflict recovery by adding an explicit refresh action that preserves the
  selected script, loads its current version, and cannot discard unsaved editor changes.
- Fixed custom-script drafts being discarded without confirmation when switching, creating, or
  closing, and added target-specific confirmation before deleting a saved script.
- Fixed OneKey drafts being discarded during navigation and prevented send actions from using an
  older saved credential while a different username or secret update is visible in the editor.
- Fixed unsaved MCP grant and HTTP settings being discarded on destructive navigation, added exact
  grant-revocation confirmation, and locked mutable settings while save responses are pending.
- Fixed the quick-command manager silently discarding unsaved additions, edits, deletions, and
  ordering changes when closed from the title bar, backdrop, or Cancel action.
- Fixed Key Manager drafts and pasted key material being discarded or overwritten by pending
  responses, added exact destructive-action confirmation, and stopped project/user Host Keys from
  being implicitly assigned to the currently selected Profile when opened for editing.
- Fixed the multi-page terminal settings dialog silently discarding unsaved preferences, sync input,
  and keymap changes, and isolated late terminal-export directory picker responses.
- Fixed duplicate transfer start/retry/cancel and tunnel create/stop submissions. Pending row actions
  are isolated by task ID, and responses from a closed dialog cannot update or close its replacement.
- Fixed duplicate Tmux attach, control-mode, pane synchronization, and layout/session mutations.
  Control watcher cleanup is now bound to its runtime ID and cannot stop a newer replacement.
- Fixed duplicate and conflicting log archive, session bundle export, and shard deletion actions.
  Closing the log manager now isolates late write results from a replacement dialog.
- Fixed duplicate Session Settings saves, staged-secret writes, SSH health checks, and Host Key
  scan/trust actions. Host Key results are now bound to the exact SSH draft that requested them.
- Fixed duplicate OpenSSH/PuTTY/Shell session imports and protected unsaved import drafts during
  format changes or close. Overlapping file reads can no longer submit or restore a stale preview.
- Fixed duplicate file-manager create, delete, rename, move, chmod, and transfer-planning operations.
  Stale listings and superseded remote sessions can no longer own a mutation.
- Fixed overlapping private-key file reads in Key Manager so an older file cannot replace or submit
  the latest selection. Generic Secret writes now reject NUL and payloads larger than 1 MiB.
- Fixed Serial Analyzer and main monitor capture clearing freezing or restoring stale frames. Capture
  reads, clearing, and exports now serialize so delayed polling and duplicate clicks cannot race mutations.
- Fixed MCP approval buttons submitting the same decision twice before the pending state rendered.
  Approval responses are now single-flight by request ID and expiry waits for an in-flight decision.
- Fixed duplicate SSH Host Key trust decisions and stale scan responses replacing a newer security
  prompt. Trust results now participate in the shared Host Key mutation ownership boundary.
- Fixed same-frame screen-lock submissions starting duplicate Portable Vault unlocks. Unlock and
  restore-lock operations are now single-flight, and stale unlocks fail closed behind a newer lock.
- Fixed duplicate disconnect and reconnect actions issuing multiple `close_session` requests for one
  session. Pending closes now lock conflicting controls and cannot restore a deleted Profile.
- Fixed same-frame OneKey save, delete, or send actions entering the backend more than once. OneKey
  operations now acquire a synchronous dialog gate before React's pending state is rendered.
- Fixed the Sender panel dispatching text or Hex twice when its button and keyboard shortcut fired
  in the same frame. A synchronous send gate now owns the complete configured send batch.
- Fixed duplicate serial DTR, RTS, and Break actions queueing repeated device operations. The three
  controls now share a per-session gate and ignore responses after their Profile is deleted.
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
- MCP custom-script approvals are bound to the script ID and `updatedAt` version captured before the
  prompt. Editing, disabling, deleting, or retargeting the script while approval is pending fails
  closed and requires a new review; approval events contain only the trusted script summary.
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
