#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "libssh GSSAPI build check currently supports Linux only" >&2
  exit 1
fi

for command in cargo jq ldd nm pkg-config readelf; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command is unavailable: $command" >&2
    exit 1
  fi
done

if ! pkg-config --atleast-version=0.9.7 libssh; then
  echo "libssh >= 0.9.7 development metadata is unavailable" >&2
  exit 1
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

binary="$({
  cargo test -p libssh-rs --no-run --message-format=json-render-diagnostics
} | jq -r '
  select(
    .reason == "compiler-artifact"
    and .target.name == "libssh_rs"
    and .profile.test == true
    and .executable != null
  )
  | .executable
' | tail -n 1)"

if [[ -z "$binary" || ! -x "$binary" ]]; then
  echo "cargo did not report the libssh-rs test executable" >&2
  exit 1
fi

if ! nm -D "$binary" | awk '$NF ~ /^ssh_userauth_gssapi(@|$)/ { found=1 } END { exit !found }'; then
  echo "libssh-rs test executable does not reference ssh_userauth_gssapi" >&2
  exit 1
fi

libssh_path="$(
  ldd "$binary" \
    | awk '$1 ~ /^libssh\.so/ && $2 == "=>" { print $3; exit }'
)"
if [[ -z "$libssh_path" || ! -f "$libssh_path" ]]; then
  echo "libssh-rs test executable did not resolve a shared libssh" >&2
  exit 1
fi

if ! readelf -d "$libssh_path" \
  | awk '/NEEDED/ && /libgssapi/ { found=1 } END { exit !found }'; then
  echo "linked libssh does not declare a GSSAPI runtime dependency" >&2
  exit 1
fi

echo "libssh GSSAPI build check passed: $libssh_path"
