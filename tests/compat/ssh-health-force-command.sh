#!/bin/sh
set -eu

mode="$(cat /run/portmate-ssh-health-fault)"
original="${SSH_ORIGINAL_COMMAND:-}"

run_sftp_fault_once() {
    fault_mode="$1"
    if mkdir /tmp/portmate-sftp-health-fault-fired 2>/dev/null; then
        exec /usr/local/bin/portmate-sftp-health-fault-server "$fault_mode"
    fi
    exec /usr/lib/ssh/sftp-server
}

case "$mode:$original" in
    exec-rejected:*PORTMATE_SSH_HEALTH_OK*)
        echo 'health exec rejected by fault server' >&2
        exit 73
        ;;
    exec-silent:*PORTMATE_SSH_HEALTH_OK*|runtime-replaced:*PORTMATE_SSH_HEALTH_OK*)
        sleep 30
        exit 0
        ;;
    exec-wrong-marker:*PORTMATE_SSH_HEALTH_OK*)
        printf '%s\n' 'PORTMATE_SSH_HEALTH_WRONG'
        exit 0
        ;;
    sftp-rejected:internal-sftp)
        echo 'sftp rejected by fault server' >&2
        exit 74
        ;;
    sftp-silent:internal-sftp)
        run_sftp_fault_once init
        ;;
    sftp-canonicalize-silent-once:internal-sftp)
        run_sftp_fault_once canonicalize
        ;;
    sftp-opendir-silent-once:internal-sftp)
        run_sftp_fault_once opendir
        ;;
    sftp-readdir-silent-once:internal-sftp)
        run_sftp_fault_once readdir
        ;;
    sftp-canonicalize-missing:internal-sftp)
        vanished="$(mktemp -d /tmp/portmate-sftp-vanished.XXXXXX)"
        cd "$vanished"
        rmdir "$vanished"
        exec /usr/lib/ssh/sftp-server
        ;;
    sftp-operation-denied:internal-sftp)
        exec /usr/lib/ssh/sftp-server -d /home/portmate/portmate-sftp-blocked
        ;;
    scp-rejected:dst=*)
        echo 'scp rejected by fault server' >&2
        exit 75
        ;;
esac

if [ -z "$original" ]; then
    exec /bin/sh
fi
if [ "$original" = "internal-sftp" ]; then
    exec /usr/lib/ssh/sftp-server
fi
exec /bin/sh -c "$original"
