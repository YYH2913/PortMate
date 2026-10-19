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
`rsa 0.10.0-rc.18`. No patched release is available. PortMate mitigates the affected private RSA
operation for local SSH identities by using Russh's external-signer API and
`RandomizedSigner::try_sign_with_rng` with `getrandom::SysRng`. RSA-SHA512, RSA-SHA256, and legacy
SSH-RSA signatures all request fresh blinding randomness. Ed25519/ECDSA identities retain Russh's
normal signer, ssh-agent identities are signed outside this process, and the libssh GSSAPI backend
uses libssh/OpenSSL rather than the affected Rust private-key operation.

The mitigation is covered by a random-source request regression and a real OpenSSH RSA login test.
The exception is intentionally treated as required evidence: when the advisory is withdrawn, the
dependency disappears, or a patched RSA release is adopted, the audit gate fails until the stale
exception and mitigation note are reviewed.

The workspace patches `libsodium-sys-stable` 1.24.0 locally because even upstream
1.24.3 still requires the ZIP 8.x build dependency affected by
`RUSTSEC-2026-0367`, `RUSTSEC-2026-0368` and `RUSTSEC-2026-0369`. The patch only
upgrades that build dependency to ZIP 9.0.5; bindings, build logic and the signed
native source archive remain unchanged. Provenance and removal criteria are in
`crates/libsodium-sys-stable/PORTMATE_ORIGIN.md`. No ZIP vulnerability is exempted.

## Reviewed Upstream Warnings

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
