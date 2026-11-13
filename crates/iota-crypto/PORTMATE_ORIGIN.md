# PortMate Stronghold crypto dependency patch

This is the published Apache-2.0 `iota-crypto` **0.23.2** source, not a new
upstream release. It is consumed through the workspace crates.io patch and
retains the upstream package version. `Cargo.toml.orig` is the upstream manifest.

- Upstream: <https://github.com/iotaledger/crypto.rs>
- Commit: `8c13125541e762206e2dc57b8bfde89c7f6ce8e3`
- crates.io archive SHA-256:
  `98a38db844c910d78825e173c083f2ef416b69cb091bba8ac1055763c6db065b`
- `PORTMATE_UPSTREAM.json` records the unmodified production-source hashes.
  Original source copyright and SPDX notices are retained; `LICENSE` contains Apache-2.0.

Stronghold 2.1.0 still enables this crate's `k256 0.13` support, which pulls
`crypto-bigint 0.5.5` affected by `RUSTSEC-2026-0387`. There is no compatible
patched release on that old branch. This patch updates **k256 to 0.14** and
uses the patched `crypto-bigint 0.7.11` / `ctutils 0.4.8` from the root lockfile.
It does not add an advisory exception or relabel a vulnerable package version.

Only these production API boundaries change:

- `src/signatures/secp256k1_ecdsa.rs`: adapt k256's renamed SEC1 encoding and
  infallible recoverable-signing APIs. Bridge the existing rand 0.8 CSPRNG
  interface to k256's rand_core 0.10 traits, forwarding the caller's bytes and
  preserving failure propagation. No fallback RNG or custom cryptographic math
  is introduced.
- `src/keys/slip10.rs`: use the renamed SEC1/curve-affine traits and checked
  byte-slice constructor. Derivation formulas and serialized key formats stay unchanged.

Snapshot encryption, key derivation, record storage and their production sources
are unchanged. Upstream inline unit tests are moved under `test/rust/iota-crypto/`
to follow the repository's test layout. Selected upstream secp256k1 and SLIP-0010
vectors from the same commit, plus the entropy-bridge regression, run through
`npm run test:stronghold-crypto`. The isolated harness owns its dependency lock
under `test/tooling/iota-crypto-check/`; it does not ship in the app. The portable
vault fault matrix and previous-snapshot regression cover storage compatibility.

Remove this patch when an upstream Stronghold/iota-crypto release uses the
patched elliptic-curve stack. Do not change the snapshot format to remove it.
