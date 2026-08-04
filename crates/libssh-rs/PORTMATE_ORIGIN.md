# PortMate libssh-rs fork

This directory contains `libssh-rs` 0.3.8 from the published crates.io
archive. Its recorded upstream source commit is
`5cd2872a60a20cc84cea2c37a1388d2ef12b4653` from
<https://github.com/wez/libssh-rs>.

PortMate carries five API additions in `src/lib.rs`:

- `Session::userauth_gssapi()` safely locks the private session handle,
  calls the existing `libssh-rs-sys` `ssh_userauth_gssapi` binding, and maps
  the native result through the crate's existing `AuthStatus` conversion.
- `Session: Clone` shares the crate's existing synchronized session holder so
  blocking native calls can be owned by Tokio blocking tasks.
- `Session::send_keepalive()` safely exposes libssh's protocol keepalive call.
- `SshKey::key_type_name()` and `SshKey::export_public_key_base64()` expose the
  same host-key observation fields used by PortMate's existing trust store.

PortMate also corrects the argument order in `Channel::poll_timeout()` to match
libssh's `(channel, timeout, is_stderr)` C API. Channel reads preserve libssh's
distinct timeout, temporarily-unavailable, and EOF results, including the zero
value returned by some nonblocking libssh implementations when no data is ready.

`SftpFile::flush()` checks for `fsync@openssh.com` before sending the optional
extension request. This matches the russh backend and keeps uploads compatible
with SFTP servers that do not implement OpenSSH extensions.

Source-level attributes also allow the finite set of newer Clippy lints emitted
by the otherwise unchanged 0.3.8 sources, so workspace `-D warnings` checks can
keep treating this vendored package as a primary member.

The upstream `libssh-rs-sys` vendored build does not compile GSSAPI support.
Linux therefore prefers a system libssh and must separately prove that it has
GSSAPI before selecting this authentication path. Other targets enable the
vendored libssh and OpenSSL features so the workspace remains self-contained;
those builds do not claim GSSAPI support.
