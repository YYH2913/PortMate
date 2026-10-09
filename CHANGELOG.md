# Changelog

All notable PortMate changes are recorded here. PortMate is still alpha software; a source build
or an unsigned artifact is not a production release. The complete release gates are maintained in
[RELEASE.md](./RELEASE.md).

## [Unreleased]

### Changed

- Centralized frontend, script and Rust tests, probe binaries, compatibility runners,
  SDK client projects and fixtures under the workspace-root `test/` directory.
  Existing Cargo filters and npm test commands retain their behavior; test TypeScript
  and externally included Rust support code now have dedicated type/format checks.
  Local review reproducers under `tmp/` are excluded from the normal suite.

### Fixed

- Native diagnostic translation coverage no longer stops at the first `cfg(test)`
  hook in a production file. Previously hidden diagnostics now have all six locale
  translations, including credential recovery, terminal queues, libssh and Sysmon.

## [0.1.11] - 2026-10-07

### Added

- An explicit SSH `none` authentication preset supports devices that accept a
  connection without a password or public key. Host-key verification still applies.
  None-only profiles with a saved username skip password prompts and do not read
  unused password/private-key secrets from a locked vault.
- SSH/Tmux profiles now offer **Ignore fingerprint changes on auto-reconnect**,
  disabled by default. It temporarily accepts changed host keys for previously
  trusted endpoints, including every Jump Host, during automatic reconnect only.
  Saved trusted fingerprints remain unchanged and each bypass records a system event.

### Changed

- Consolidated the module-by-module defect review and post-fix rechecks into this
  patch version, including transport integrity, credential handling, storage,
  MCP lifecycle, UI state, localization, and build reliability improvements.
- Windows GNU portable build documentation now identifies the ZIP output and
  its required co-located executables, WebView2 loader, and license files.

### Fixed

- known_hosts imports now reject bracketed entries with invalid or zero ports instead of silently assigning port 22.
- Secret redaction now consumes complete quoted, comma-containing, and whitespace-containing credential values without leaving their suffixes in events or diagnostics.
- Remote Sysmon now recognizes MSYS2/Git Bash and Cygwin `uname` labels as Windows and reaches the PowerShell collector.
- Linux Sysmon now preserves interface ownership from `getifaddrs` and only uses global kernel address fallbacks when native enumeration has no usable address.
- Sysmon dialogs and the workspace sidebar now hide the last remote sample while an SSH/Tmux session is disconnected or reconnecting.
- MCP HTTP Accept negotiation now honors the most specific media range, including explicit `q=0` exclusions and type wildcards.
- Managed MCP HTTP readiness probes now enforce a cumulative read deadline and check the startup deadline before accepting `Running`.
- MCP approval responses now fail closed when the request has expired, and approval waits use the request's absolute expiry deadline.
- A managed-instance background task stops the MCP HTTP bridge and retires its
  bearer token when the bound grant expires, even with the dialog closed or
  hidden; failed token retirement is retried independently of UI polling.
- Failed log-retention pruning no longer suppresses retries for the full hourly check interval.
- Log shard append now checks the opened file's link count on Unix and Windows,
  and rejects Windows reparse points before writing through an unrelated inode.
- Legacy app-data migration now refuses to delete current-directory entries outside the explicit PortMate/bootstrap allowlist.
- Legacy app-data migration now removes only the explicit bootstrap entry and refuses to remove a current directory that gains new contents during the migration check.
- SQLite stores and locks now use no-follow native opens, retained directory/file
  handles and post-open identity checks instead of trusting a path preflight.
- Local transfer sources and resume files now reject hard links before reading, truncating, or appending shared inodes.
- Remote SCP/SSH-copy and SFTP resume paths now fail closed around unverifiable hard-linked partial files; SFTP recreates an exclusive partial file before restarting a transfer.
- Local transfer sources now bind the opened handle to its original file identity, and completed transfers replace the destination atomically without deleting it first.
- Windows transfer file identity now uses stable Win32 handle queries instead of
  unstable Rust metadata APIs; checked handles are retained across source opens,
  and multiply-linked or reparse-point files are rejected.
- ZModem receives now use the same parent-component and temporary-file protections as other local transfer protocols.
- Local no-overwrite moves now use platform-specific atomic exclusive rename primitives, and fail closed where the platform cannot provide them.
- Remote batch conflict checks now treat only a confirmed missing target as absent; permission, timeout, and transport errors abort planning.
- SCP and remote-copy commands now reject symbolic links in every path component and use no-clobber hard-link commits for completed remote files.
- Remote SCP/copy shell helpers no longer overwrite zsh's `path`/`PATH` or
  assign its read-only `status` variable, restoring transfers with zsh login shells.
- SFTP transfer and file-rename commits now reject symlinked path components and use atomic no-replace hard-link commits for regular files; directory moves fail closed when the server cannot provide that guarantee.
- SFTP remote-copy sources now reject symlinked parent path components before reading, matching the download and destination-path guards.
- SFTP async file reads now reject server data packets larger than the requested buffer instead of panicking on malformed responses.
- Remote file-manager mutations now carry the connection generation observed with the listing and are rejected before a reconnect can retarget the operation.
- Local batch enumeration is bound to directory handles, including Windows
  reparse-point and directory-identity checks. Remote batch enumeration and
  recursive deletion use POSIX descriptor-relative tree operations over SSH
  instead of race-prone path-based SFTP recursion.
- Manual SSH opens now enforce the same profile snapshot used by reconnects, so endpoint and host-key policy edits during a handshake fail before trust data is persisted.
- Multi-hop SSH establishment accepts exact canonical host-key mirror additions
  from successful TOFU hops without weakening endpoint/policy/key-change checks.
- Synchronized input batches now retain each target's terminal epoch and skip queued data after reconnect; delayed clipboard reads also verify the observed connection before injecting text.
- Trigger command output-reader failures now terminate the command process group before returning, preventing background descendants from surviving a failed capture.
- Archive and signed-bundle finalization now use atomic no-replace installs, while overwrite exports preserve and restore the previous payload/checksum pair if either commit step fails.
- MCP UDP tunnel requests now apply one absolute deadline across DNS, bind, connect, send, and receive phases.
- SSH tunnel pipes preserve responses after a TCP/SSH write-half EOF, while
  a full channel close or failure promptly releases the opposite direction.
- MCP resumable-upload quota accounting now removes expired malformed upload directories and never charges invalid metadata as a full 512 MiB upload.
- SSH credential prompts now queue concurrent connection requests instead of silently cancelling the second request; disconnect and profile deletion remove only requests for the affected session.
- Proxy password drafts in session settings are now scoped to the Profile and protocol, so switching protocols preserves the correct draft without leaking it into another Profile.
- Quick commands are now disabled for non-connected sessions and report a connection prompt instead of silently dropping their input.
- Copy-on-select now resets its duplicate guard when the terminal selection is cleared, allowing the same text to be copied again after it changes externally.
- Startup keeps vault-dependent targets pending until Stronghold is unlocked,
  starts credential-free sessions immediately, and consumes each target only once.
- Tmux list parsing now uses a non-printable field separator excluded from valid names, preventing delimiter collisions from truncating names or shifting mutation targets.
- Tmux control watchers now stop and disappear when a session is renamed or removed, preventing a stale session name from orphaning live monitoring.
- Host scripts retain the validated 16 KiB parameter range on Windows Unicode
  process launches; the incorrectly applied ANSI/legacy total environment-block
  limit has been removed.
- Profile Vault private-key rotation now applies the same 1 MiB, NUL, and empty-secret validation as other secret writes before parsing or storing the key.
- Profile, private-key rotation, and OneKey saves now retain newly created secrets when Store persistence cannot be verified, preventing committed references from pointing to deleted credentials.
- Key Manager private-key import now preserves the generated secret when Profile persistence reports an unknown Store commit, preventing the UI cleanup path from deleting a possibly committed credential.
- OneKey sends and SSH logins now carry the observed connection generation through to the backend write, preventing credentials from crossing a reconnect or endpoint change.
- OneKey prompt completion now ignores current, old, and password-change prompts instead of offering the login password for those flows.
- libssh setup, SFTP, and channel blocking workers now retain ownership through timeout or cancellation and are reaped asynchronously instead of being silently detached.
- SSH-agent identity listing and signing now run as cancellable async operations with an independent deadline, so an unresponsive agent cannot leave detached threads behind.
- Native diagnostic templates now cover the file-transfer, Modem, OneKey, and Store-commit messages introduced by the reliability fixes, with Vitest limited to project-owned test roots.
- Native diagnostic catalogs now include the XModem ordering, ZModem cleanup, and SFTP resume-safety messages, keeping new reliability failures localized in every supported language.
- Log search now applies file and total byte budgets to the live reader, stopping safely when shards grow during the search.
- Profile import now invalidates pending file reads before rejecting oversized files or accepting manual edits, so stale content cannot replace the latest source.
- Terminal resize state now updates only after a successful backend request, retries transient failures a bounded number of times, and resubmits after reconnects.
- Local terminal profiles now carry a stable internal tag, so repeated actions and locale changes reuse the same shell session.
- Initial session-list IPC failures now remain visible and retry with a bounded startup timer instead of becoming a silent empty workspace.
- CI command logging now handles log-stream failures without unhandled errors and reaps the wrapped command tree before exiting.
- Sidecar preparation now selects the target-specific Cargo artifact when `CARGO_BUILD_TARGET` explicitly names the host target.
- Desktop builds, AppImage smoke/finalization, and Linux, Windows, and macOS package checks now honor relative and absolute `CARGO_TARGET_DIR` values.
- MCP SDK compatibility checks now resolve bridge binaries and per-SDK workspaces from configured `CARGO_TARGET_DIR` values and explicit `CARGO_BUILD_TARGET` output directories.
- Tmux compatibility checks now locate their Cargo probe under configured target directories and explicit target-triple output paths.
- Tmux browser workflow checks now pin the locale used by their selectors, so compatibility validation is deterministic on non-Chinese hosts.
- Workspace UI regression checks now include the explicit SSH `none` authentication preset in their expected settings matrix.
- macOS native keyring probes now attempt every cleanup step and report cleanup failures without leaving later keychain or temporary-directory cleanup unattempted.
- MCP grant management now preserves an unsaved grant draft when another window removes the grant being edited.
- YModem sends now propagate receiver cancellation and CRC handshake failures before sending more file data.
- Modem upload and download paths now reject trailing directory separators before starting the remote protocol.
- ZModem receive now removes its local partial file when the session is cancelled, times out, or fails before file finalization.
- XModem receives now reject out-of-order blocks instead of acknowledging and finalizing a truncated file.
- YModem receives now refuse to finalize files shorter than the sender's declared size.
- SSH terminal setup now waits for PTY and shell acceptance; tmux attach uses a
  dedicated PTY exec channel instead of typing commands into the foreground app.
- TCP readers stop after local disconnect even when the peer keeps its socket open.
  Shell writes run outside async workers with cancellation/deadlines, and serial
  control operations release the global registry before waiting for device I/O.
- Telnet protocol negotiation no longer waits for long-running user transfers;
  queued text is encoded under the current negotiated mode at send time, and
  oversized subnegotiations are bounded and discarded until their end marker.
- All terminal transports retain fragmented UTF-8 characters across reads. MCP
  screen snapshots now reflect terminal cursor, erase and alternate-screen state,
  exclude outbound/system logs, and reset on reconnect or terminal replacement.
- Restored Telnet CRLF submission for direct terminal input, private login input,
  detached panes, OneKey credentials, and commands after BINARY negotiation.
  Telnet option acknowledgements now settle without negotiation loops, and received
  carriage returns render immediately while NVT padding is filtered across packets.

### Security

- Enabling the reconnect fingerprint option permits server impersonation or
  man-in-the-middle attacks during automatic reconnect. Initial/manual connections,
  host-key scans, and unseen endpoints retain the configured verification policy.
- Closed credential-redaction suffix leaks, strengthened transfer/storage path
  and hard-link checks, and preserved generated secrets after indeterminate Store
  commits. MCP expiry checks and connection-generation fences prevent stale
  authorization or credentials from reaching a replacement connection.

### Migration

- Existing SSH/Tmux profiles and cached sessions default the new
  `reconnectIgnoreHostKeyChanges` preference to `false`.
- This patch requires no manual Store or vault format migration. Legacy app-data
  cleanup now preserves unexpected or concurrently created files.
- Remote SFTP uploads and copies restart an existing partial file through exclusive
  creation because SFTP metadata cannot prove that the partial inode has one link.
  Local downloads and SCP retain their separately verified resume behavior.

### Known Limitations

- Safe remote tree enumeration/deletion requires Python 3 and POSIX `dir_fd`
  primitives on the remote host. SFTP-only peers or unsupported remote operating
  systems fail closed; single-file transfers still use the selected protocol.

- The Linux cross-build produces an unsigned Windows x86-64 portable ZIP.
  Native Windows installation, WebView2 startup, OS-keyring behavior, and signed
  MSI/NSIS release acceptance still require Windows runners.

## [0.1.10] - 2026-09-17

### Changed

- Stronghold unlock controls now provide password visibility, verification progress,
  and focused retry feedback in the key manager and lock screen. Failed create,
  unlock, and password-change attempts retain their drafts for correction.
- Reorganized Session, Terminal, Workspace, and Tools menus by workflow. New
  workspace windows now belong to the Workspace menu; connection actions and
  management tools are grouped separately.
- Session settings use protocol tabs and a searchable navigation tree, with stable
  dialog dimensions and protocol-specific authentication pages. Terminal settings
  use a searchable sidebar with a compact layout on narrow windows.
- Expanded the About page with supported capabilities, profile-isolated trust,
  optional MCP access, maintainer links, and component information, with matching
  translations across all six interface languages.
- HTTP client JSON now defaults its Server ID to `portmate-<client-id>` for the
  selected authorization identity. Explicitly edited names remain unchanged during
  refresh and binding changes; the import name never changes the authorization itself.
- Separated MCP client authorization from HTTP transport management. The Grants page
  owns permissions, session scope, expiration, and revocation; the HTTP page owns the
  explicit client binding, network settings, Token, connection JSON, and managed service.
- HTTP setup now saves the selected binding before generating a missing Token and starting
  the service. Copying connection JSON no longer changes authorization or HTTP settings.
  Removed legacy automatic Client ID adoption: even a single active grant must be selected
  explicitly, and revocation never switches the bridge to another client.

### Added

- Connection credential prompts can unlock an existing Stronghold vault inline.
  Vault status updates keep save controls in sync, and opening the key manager
  no longer cancels the pending connection prompt.
- File manager listings now sort independently by name, type, size, or modification
  time, with directories first and natural filename order. Added detail columns,
  file/selection size summaries, localized timestamps, and readable permissions;
  range selection follows the displayed sort order.
- Added system-language detection, a persistent UI language selector shared by windows,
  English fallback, and Arabic RTL layout. Arabic, Chinese, English, French, Russian, and
  Spanish have matching catalog coverage across controls, advanced settings, host scripts,
  permissions, command help, and registered native diagnostics. Language changes do not remount terminal sessions
  or translate commands, device output, paths, or user-authored content.
- Added presentation-only native diagnostic localization, complete catalog/placeholder
  checks, and translated-label regressions for serial enum values. Error state, protocol
  identifiers, and matching logic remain unchanged; unrecognized diagnostics stay verbatim.
- Added two-step MCP revocation confirmation, including exact Client ID entry, cancellation,
  and protection against duplicate submissions while revocation is pending.
- Command history now captures commands finalized after serial-side line
  editing (cursor movement, backspace, delete and Enter) and successful MCP
  `run_command` submissions. MCP `send_text` records only complete CR/LF-terminated
  lines in the current payload, excluding unfinished fragments and editor controls.
  Desktop and MCP writers share a persisted enabled/limit/retention policy;
  history persistence failures do not turn a successful send into an error.
- Serial terminals passively detect repeated wrapped-line redraws and offer a
  suggested device column width. Applying the suggestion changes only the
  current terminal view and never sends a probe command to the device. Two
  matching Readline-style observations are required; historical replay and
  alternate-screen output are excluded. Hints can be dismissed, applied widths
  can be reverted, and reconnect/profile changes reset detection and adaptation.

### Fixed

- Unified localized Stronghold error presentation across vault operations, credential
  prompts, and screen unlocking, with separate messages for incomplete vault files,
  existing vault state, and master-password verification failures.
- Localized Tools menu identifiers and restored the Session reconnect action.
  Removed shortcut hints without matching bindings, corrected the Find shortcut,
  and disabled selection-only export and synchronized input when unavailable.
- Desktop and backend OneKey prompt detection now retain the same last 160
  Unicode characters of a prompt line, avoiding inconsistent identity validation
  for long interactive SSH prompts.
- Modem path-ready upload destinations remain device paths instead of being
  expanded under the profile's local transfer directory. Transfer hints preserve
  the literal `remote:` prefix in every language.
- Internal workspace tab and panel dragging now uses pointer gestures so it works
  alongside Windows native file drops. Fixed hidden-panel insertion positions,
  RTL dock ordering, cancellation cleanup, and viewport-limited resize keyboard
  dead zones; real mouse regressions cover docking, grouping, and splitting.
- SFTP local target validation no longer probes incomplete Windows drive prefixes
  such as `\\?\F:` while traversing a full path. Root and ancestor link checks
  remain in place, including junction protection. File-manager parent navigation
  and renaming preserve drive/UNC share roots and remote POSIX filename separators.
- File mutations and transfers now target the successfully loaded directory, not
  an unsubmitted address draft. Failed listings clear stale selection and disable
  mutations; reconnects reject old directory results, and refreshes retain address
  drafts. Permission editing rejects partially valid octal values.
- Free-input editors in main and detached windows wait for transport acknowledgement,
  retain drafts on failure, prevent duplicate submissions, and only record successful
  submissions in command history. Their shared submission lifecycle cancels queued
  work and ignores stale results when the view or connection changes. Background
  terminal typing and paste cannot interfere with an in-flight editor submission.
- MCP HTTP copy actions read current access details; save, token rotation, and managed
  startup check the expected configuration before side effects, preventing stale
  windows from overwriting another binding or operating on the wrong client.
- Queued transfers can be cancelled individually or together with running transfers.
  Script deletion or refresh selecting another script clears the previous parameters
  and result. Delayed sysmon responses no longer replace newer samples.
- Transfer queue commits now recheck that the Profile still exists, so a session
  deletion racing with a transfer start cannot leave an orphan queued task.
- Asynchronously initialized detached terminals correctly publish their ready state
  in development without assuming a second Strict Mode initialization.
- Detached native windows no longer replace live backend session state with another
  window's stale or invalid browser cache, avoiding unintended terminal recreation.
- Removed repeated full-history scans from terminal timestamp insertion, lookup,
  redraw cleanup, and restoration. A sparse ordered index preserves timestamps
  and follows xterm marker movement without reducing retained terminal output.
- Prevented quadratic deduplication when more than 4,000 terminal events are pending.
  Synchronously completed events now drain iteratively in bounded bursts, avoiding
  stack overflow that could leave subsequent terminal input/output stuck.
- HTTP MCP requests load the current Store once per envelope instead of twice;
  recent log metadata updates search from the newest event rather than the oldest.
- Added 100,000-timestamp and 30,000-pending-event complexity regressions, plus a
  long-session browser check with 20,000 restored rows, continued output, prompt
  redraw, export, and a synchronous event backlog.
- Centralized command submission recording with explicit input sources. Completion
  only maintains suggestions, so Enter no longer records the same command twice.
- Middle-click paste now uses xterm's normal input path, including newline conversion,
  private-input checks and bracketed paste (which waits for a real Enter).
- Command-line tracking recognizes actual Delete CSI, fragmented CSI/SS3 sequences
  and non-BMP characters, and resets on connection replacement.
- Added browser regressions for removing revoked or missing script clients and
  saving the cleaned allowlist; removal controls now have explicit accessible names.
- HTTP binding, Token generation, and managed startup now require an active grant.
  Rebinding invalidates the old Token; revoking the bound identity stops the managed bridge
  and clears its Token. Cleanup failures are reported without restoring revoked permissions.
- Preserved unsaved HTTP drafts on failed saves, ignored stale runtime polls after mutations,
  and kept the authorization footer from obscuring the final instructions.

### Security

- Remote mutation checks also reject Windows-style backslash-separated `..`
  components. MCP content uploads reject local-only destinations before accepting
  chunks or reserving upload quota, using shared transfer endpoint classification.
- HTTP management checks the expected binding and network configuration under the runtime
  lock before token or process changes. Stale windows cannot silently act on another client.

### Migration

- No new Store format migration is introduced in 0.1.10. HTTP Bridge setup requires an
  explicitly selected active client; a missing or revoked binding must be replaced manually.
- UI routing and preference values use English identifiers or numbers, independent of
  display language. Completion preferences use numeric character/row counts and
  `none` / `input` / `top`; old translated preference values are not migrated.
- Profile-transfer JSON now uses version 2 with structured English warning codes and
  unchanged user labels. Version 1 transfer documents are not accepted; export them again
  from the updated application. This does not change the native Store format.

## [0.1.9] - 2026-09-09

### Changed

- Added per-view terminal font zoom controls, Ctrl/Cmd + plus/minus/0 and
  Ctrl/Cmd + wheel shortcuts, with persisted view sizes and profile-default reset.
  Detached terminals now report their resized character grid to the connection.
- Replaced terminal custom scripts with host-executed Python 3 and platform Shell skills
  (Unix sh / Windows PowerShell 7), with per-client exposure, typed JSON parameters,
  dynamic MCP tools, bounded execution and captured results. Breaking change:
  old terminal scripts are neither loaded nor migrated.
- Updated the verified toolchain and dependency baselines: Node.js 24.20.0 with npm 12.0.2,
  Rust 1.98.1, Tauri 2.11.x, React 19.2.8, Vite 8.2.2, Vitest 5.0.0, and compatible
  Rust ecosystem releases including base64 0.23, sha2 0.11, Argon2 0.6, libloading 0.9,
  Russh 0.63, rusqlite 0.40, zmodem2 0.7 and the vendored libssh wrapper's bitflags 2 /
  thiserror 2. Digest formatting and KDF regression vectors preserve existing stored data;
  ZModem uses the poll/submit API and unsupported SSH host certificates fail closed.
- Extended MCP compatibility coverage to Rust SDK 3.2.0, Ruby SDK 1.5.0 and Java SDK 2.0.1;
  updated the checksum-verified Maven distribution to 3.9.16 while retaining older SDK fixtures.
- Added a main-window MCP Bridge shortcut to start the saved HTTP service, show its
  runtime state, and open HTTP management when already running or startup fails.
  The shortcut does not generate tokens or change client grants.
- Extended the official Python MCP SDK matrix to 2.1.1 with a committed dependency snapshot,
  retaining the older SDK coverage and both stdio and Streamable HTTP checks.
- Desktop terminal input now uses a bounded, sequence-ordered IPC pipeline instead of waiting
  for a complete IPC round trip per keystroke. Stream bindings reject input from replaced
  connections, while acknowledged sends and binary mouse frames retain ordering barriers.
- Repeated backspace and line-editing operations defer completion UI refreshes while updating
  the tracked input immediately; submissions and navigation still refresh without a delay.
- Command completion keeps its panel mounted across text and candidate updates. Only panel-size,
  cursor-row, and terminal-size changes recalculate its placement, avoiding repeated entrance
  animations and terminal/timestamp shifts while preserving below-cursor suggestions.

### Fixed

- Fixed serial terminal wrapped-line erase corruption by preserving the
  configured serial column width across pane resize and font zoom.
- Reduced Windows serial input stalls by avoiding empty synchronous reads on
  cloned COM handles.
- Windows serial readers now read only bytes already queued and wait outside
  the synchronous driver while idle, avoiding an empty 100 ms read delaying
  keyboard writes through a cloned handle. Received CR/LF bytes remain intact.
- Serial terminals retain their configured column count across window resize,
  pane changes and font zoom. Narrow views scroll horizontally; viewport resize
  no longer overwrites serial geometry. This prevents wrapped-input erase
  sequences from moving into earlier output when the device uses that width.
- Serial Insert mode now sends Escape to the device instead of silently entering
  local Normal mode and swallowing Linux login input. Shift+Escape explicitly
  enters Normal; a visible resume-input action exits it. Local navigation rebases
  stale off-screen positions after incoming boot output, preventing Enter from
  jumping back to early log rows. Main and detached windows share this behavior.
- Repeated sender batches wait the full configured interval after transport acknowledgement,
  without blocking synchronized keyboard input between batches. Text and Hex share cancellable
  per-session queues and window-owned jobs pinned to their original connections. Stops cancel
  queued writes; disconnects and closed windows retire jobs without replaying them after reconnect.
  Open-pane targets are deduplicated and invalid/odd-length Hex input is rejected instead of rewritten.
- Custom script editing now rejects oversized/NUL-containing bodies without silently
  truncating or rewriting commands. Confirmed refresh can recover from concurrent-save
  conflicts while cancelled refresh preserves drafts, and initial load errors stay visible.
- Custom script version timestamps advance even when the system clock repeats or moves
  backwards, preserving stale-save/delete detection and execution version checks.
- `send_key` preserves literal character case and symbols and correctly sends Ctrl+_ (0x1F).
  Named key aliases remain case-insensitive; repeated Ctrl prefixes and arbitrary escape payloads
  are rejected consistently by the desktop and MCP input path.
- MCP Bridge now reapplies the requested recent-log limit after filtering desktop IPC responses,
  preventing an oversized or stale IPC result from bypassing the documented 1-1000 bound.
- Transfer retry now fails closed for queued, running, and completed tasks; only failed or cancelled
  tasks can be retried, matching the desktop UI and MCP contract.
- Destroyed windows release their pending input-stream buffers and staged SSH credentials without
  closing shared sessions or invalidating another window's input stream. Cancelled close requests
  do not discard this state.
- Application initialization failures now report their diagnostic and exit with status 1 inside
  Tauri's setup callback, avoiding Tauri's panic-on-setup-error path. Package smoke checks cover
  unreadable Stores after preflight as well as legacy-directory conflicts.
- Resuming a private free-input draft from goto-line preserves its protection, including privacy
  enabled while the draft was hidden. Mode switches retain drafts; explicit cancellation clears
  them, allowing the next editor to begin a fresh input boundary.
- Official MCP SDK compatibility checks now use isolated temporary Stores with explicit read-only
  grants instead of assuming unconfigured clients may read session resources. TypeScript stdio/HTTP
  checks additionally verify denied access before granting and after revocation.
- MCP log search now filters authorized sessions before applying the result limit, so activity in
  unrelated sessions cannot hide matching authorized logs. Recent-log reads clone only the returned
  events instead of copying a session's entire retained history.
- Tab and clicked completion candidates now resolve their suffix against the live input line,
  preventing duplicate characters when the preview is still waiting for a deferred refresh.
  Stale candidates no longer intercept native Tab, and keyboard selection keeps its rendered row.

### Security

- Updated `fast-uri` and `qs` in the root npm lock and pinned TypeScript SDK fixtures without
  changing the SDK matrix versions. Regression checks keep those parser versions aligned.
- Updated compatible Rust crypto dependencies (`aes`, `chacha20`, `der`, and `wnaf`) to non-yanked
  patch releases and removed the obsolete AES audit exception. The existing RSA mitigation and
  exact-match RustSec review policy remain in place.
- An explicit non-default MCP Client ID is no longer replaced by the desktop-selected identity
  when its grant is missing, expired, or revoked. Such requests now fail closed instead of borrowing
  another client's permissions; legacy default identity migration remains available.
- Private terminal input is excluded from command completion and command history, including free-input
  editor submissions. Once a line contains private input, turning off the toggle or losing automatic
  prompt detection no longer makes its remaining bytes public; submission, cancellation, or clearing
  the line releases that protection. The UI indicates when protection is retained for the current line.

## [0.1.8] - 2026-09-05

### Added

- Added regression coverage for shifted history windows, cached terminal restoration, partial
  control sequences, screen redraws, and timestamp alignment.

### Changed

- New Serial session setup now refreshes the host's available serial devices automatically when
  entering Serial and provides an explicit refresh control in both quick and advanced settings.
- Batched interactive input through microtasks and limited timestamp lookups to the visible range,
  reducing scheduling and scrollback work during rapid typing.
- Moved periodic stream checkpoints, idle flushes, and final persistence off transport readers.
  Live output can continue while the Store is busy.

### Fixed

- Prevented old polled logs and their cursor-control sequences from being replayed behind the live
  prompt after the event-ID cache rolls over or a terminal view is restored.
- Kept timestamps attached to redrawn content, preserved unchanged prompt times during typing,
  and stopped displaying misleading time labels on empty rows.
- Deferred terminal resizing until cached screen restoration finishes and kept Enter at the live
  output position without taking over subsequent manual scrolling.
- Converted saved serial script lines into actual device command submissions, including the final
  line, instead of sending newline-free text that remains at the prompt.

### Security

- Retained session-generation checks, bounded queues, secret redaction, and ordered audit
  persistence while optimizing the terminal data path. No MCP grant scope was broadened.

### Migration

- No Store schema migration is required from 0.1.7. Existing profiles, grants, scripts, logs,
  encrypted credential references, and workspace settings load in place.

### Known Limitations

- Browser fixtures and loopback tests do not replace physical serial-device or native Windows and
  macOS verification. Driver, network, and device echo latency can still affect interaction.
- This source version does not imply that a new signed or native-tested binary has been published.

## [0.1.7] - 2026-09-02

### Added

- Added an always-visible Stronghold status control and an explicit creation flow with password
  confirmation, so a new credential vault is easy to find and initialize.

### Changed

- Unlocking Stronghold now only opens an existing vault; initialization is a separate explicit
  operation and cannot happen accidentally while attempting to unlock.

- Reorganized the top-level Tools menu into connection, automation, and management sections with
  keyboard navigation and focus restoration, while preserving the existing command entry points.
- Added an always-visible MCP Bridge status summary and clearer authorization, listener, client,
  and service-process sections without changing the existing grant, HTTP, and audit workflows.
- Sender-panel atomic writes now acknowledge completion from the native per-session transport queue;
  configured repeat intervals are measured against actual writes while printable keyboard input keeps
  its asynchronous coalescing path.

### Fixed

- Locked or still-loading Stronghold state no longer offers unusable credential persistence in the
  SSH prompt. One-time connections remain available, with a direct route to Stronghold setup.

### Security

- Stronghold initialization remains local and master passwords are neither stored in SQLite nor
  exposed to MCP clients, logs, or release metadata.

### Migration

- No Store schema migration is required from 0.1.6. Existing sessions, credential references,
  grants, terminal history, transfers, scripts, host keys, and workspace state load in place.

### Known Limitations

- The Windows GNU portable archive is unsigned cross-build evidence and does not replace native
  Windows MSVC, WebView2, Credential Manager, MSI/NSIS, Authenticode, or clean-machine tests.

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
