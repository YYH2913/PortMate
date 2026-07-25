#!/bin/sh
set -eu

mode=${1:-${PORTMATE_TCP_MODE:-echo}}

case "$mode" in
  echo)
    ncat --listen --keep-open --broker 23 &
    listener=$!
    trap 'kill "$listener" 2>/dev/null || :' EXIT INT TERM
    while kill -0 "$listener" 2>/dev/null; do
      ncat 127.0.0.1 23 --exec /bin/cat || :
    done
    ;;
  burst-close)
    exec socat TCP-LISTEN:23,reuseaddr,fork EXEC:'/usr/local/bin/tcp-compat-server burst-client'
    ;;
  close)
    exec socat TCP-LISTEN:23,reuseaddr,fork EXEC:'/usr/local/bin/tcp-compat-server close-client'
    ;;
  burst-client)
    dd if=/dev/zero bs=4096 count=64 2>/dev/null | tr '\000' B
    printf '__PORTMATE_TCP_BURST__\n'
    ;;
  close-client)
    printf '__PORTMATE_TCP_CLOSE__\n'
    ;;
  *)
    printf 'unsupported PORTMATE_TCP_MODE: %s\n' "${PORTMATE_TCP_MODE}" >&2
    exit 64
    ;;
esac
