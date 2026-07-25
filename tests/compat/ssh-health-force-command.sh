#!/bin/sh
set -eu

mode="$(cat /run/portmate-ssh-health-fault)"
original="${SSH_ORIGINAL_COMMAND:-}"

case "$mode:$original" in
    exec-rejected:*PORTMATE_SSH_HEALTH_OK*)
        echo 'health exec rejected by fault server' >&2
        exit 73
        ;;
    exec-silent:*PORTMATE_SSH_HEALTH_OK*|runtime-replaced:*PORTMATE_SSH_HEALTH_OK*)
        sleep 30
        exit 0
        ;;
    sftp-rejected:internal-sftp)
        echo 'sftp rejected by fault server' >&2
        exit 74
        ;;
esac

if [ -z "$original" ]; then
    exec /bin/sh
fi
if [ "$original" = "internal-sftp" ]; then
    exec /usr/lib/ssh/sftp-server
fi
exec /bin/sh -c "$original"
