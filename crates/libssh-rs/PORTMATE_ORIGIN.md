# PortMate libssh-rs fork

This directory contains `libssh-rs` 0.3.8 from the published crates.io
archive. Its recorded upstream source commit is
`5cd2872a60a20cc84cea2c37a1388d2ef12b4653` from
<https://github.com/wez/libssh-rs>.

PortMate carries one API addition in `src/lib.rs`:

- `Session::userauth_gssapi()` safely locks the private session handle,
  calls the existing `libssh-rs-sys` `ssh_userauth_gssapi` binding, and maps
  the native result through the crate's existing `AuthStatus` conversion.

Source-level attributes also allow the finite set of newer Clippy lints emitted
by the otherwise unchanged 0.3.8 sources, so workspace `-D warnings` checks can
keep treating this vendored package as a primary member.

The upstream `libssh-rs-sys` vendored build does not compile GSSAPI support.
PortMate must separately prove that the linked system libssh has GSSAPI before
selecting this authentication path.
