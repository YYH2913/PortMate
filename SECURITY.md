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

## Reviewed Upstream Warnings

- GTK3/GLib warnings are confined to Tauri's Linux GTK3/WebKit dependency graph. PortMate does not
  call the affected `glib::VariantStrIter` API directly; migration depends on an upstream Tauri
  GTK4-capable runtime.
- `rand 0.7.3` is a build-only dependency through `phf_generator -> selectors -> tauri-utils`. The
  advisory requires a runtime custom logger recursively calling `thread_rng`, which is not part of
  this build path.
- Yanked `aes 0.9.0` is pinned by the current Russh/SSH-key release family. It has no RustSec
  vulnerability advisory; upgrades remain tied to a compatible Russh release.
- Remaining unmaintained crates are exact transitive dependencies of Tauri build/runtime support or
  IOTA Stronghold. Their full fingerprints remain in the audit script so additions, removals, or
  version changes cannot pass without review.
