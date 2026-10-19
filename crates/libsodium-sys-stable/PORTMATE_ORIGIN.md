# PortMate libsodium-sys-stable build dependency patch

This directory contains the production sources and signed native source archive
from the published `libsodium-sys-stable` 1.24.0 crate:

- Upstream: <https://github.com/jedisct1/libsodium-sys-stable>
- Source commit: `b17dd40bd8ff878a308b79423efc0a086ee2e7d3`
- crates.io archive SHA-256:
  `72b04bf6da2c98b727af37ab62cb505f4d751b975b034a9b9ad491d333b0564e`
- Bundled `LATEST.tar.gz` SHA-256:
  `b20a92e7ec25b285eafa349d721a5bb27e3a8ba94c0816630a127883f1d1b3ab`

Only the Cargo manifest is changed: its ZIP build dependency is upgraded from
`8.5` to `9.0.5`, and automatic target discovery is disabled. No bindings,
cryptographic code, native archive, signature, or build script are modified.
`Cargo.toml.orig` retains the published manifest. Upstream examples, tests,
component interface files and its independent lockfile are not included; the
optional WASI/Wasmer component targets are not used by PortMate.

Upstream 1.24.3 still requires ZIP 8.x. A workspace-level crates.io patch is
therefore necessary to remove `RUSTSEC-2026-0367`, `RUSTSEC-2026-0368` and
`RUSTSEC-2026-0369` without adding audit exceptions. The ZIP API used by the
unchanged build script remains available in 9.x. Existing `SODIUM_LIB_DIR`,
vcpkg, signature verification and native build behavior are preserved.

The package is excluded from workspace membership and consumed as a patched
dependency, not as a new PortMate application crate. Regression coverage lives
under `test/`. Remove this patch after upstream adopts a patched ZIP version.
