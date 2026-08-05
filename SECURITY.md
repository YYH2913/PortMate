# Security Policy

## Reporting

Report suspected vulnerabilities privately through the repository's GitHub security advisory
channel. Do not include credentials, private keys, production hostnames, or unredacted PortMate
Store data in a public issue.

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
