# PortMate libssh-rs-sys fork

This directory contains `libssh-rs-sys` 0.2.8 from the published crates.io
archive. Its recorded upstream source commit is
`5cd2872a60a20cc84cea2c37a1388d2ef12b4653` from
<https://github.com/wez/libssh-rs>.

PortMate carries two related Windows ABI fixes:

- The generated Rust `socket_t` binding uses `RawSocket` on Windows, matching
  libssh's `typedef SOCKET socket_t` declaration instead of assuming `c_int`.
- The vendored libssh `SSH_OPTIONS_FD` implementation retains the complete
  socket value instead of masking it to 16 bits. This is required for proxy and
  Jump Host connections that pass a pre-connected socket to libssh.

The vendored build also reports the actual bundled libssh version, 0.11.4,
instead of the stale 0.8.90 value from the upstream Rust build script.

The Rust wrapper remains MIT licensed. The vendored libssh source is licensed
under LGPL-2.1; its `COPYING` and `BSD` notices are retained under `vendored/`.
