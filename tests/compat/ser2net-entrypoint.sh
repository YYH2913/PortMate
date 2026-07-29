#!/bin/sh
set -eu

mkdir -p /run/portmate
rm -f /run/portmate/serial

socat PTY,link=/run/portmate/serial,raw,echo=0 EXEC:/bin/sh,pty,setsid,ctty,stderr &
bridge=$!
server=

cleanup() {
    trap - EXIT INT TERM
    if [ -n "$server" ]; then
        kill "$server" 2>/dev/null || :
        wait "$server" 2>/dev/null || :
    fi
    kill "$bridge" 2>/dev/null || :
    wait "$bridge" 2>/dev/null || :
}
trap cleanup EXIT INT TERM

attempt=0
while [ ! -e /run/portmate/serial ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
        echo "timed out waiting for ser2net PTY" >&2
        exit 1
    fi
    sleep 0.05
done

ser2net -n -c /etc/ser2net/ser2net.yaml &
server=$!
wait "$server"
