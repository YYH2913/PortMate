#!/bin/sh
set -eu

mode="${PORTMATE_SSH_HEALTH_FAULT:-normal}"
case "$mode" in
    normal|ping-unresponsive|exec-rejected|exec-silent|sftp-missing|sftp-rejected|scp-rejected|runtime-replaced) ;;
    *)
        echo "unsupported SSH health fault mode: $mode" >&2
        exit 64
        ;;
esac

printf '%s\n' "$mode" > /run/portmate-ssh-health-fault

cat > /run/portmate-sshd-config <<'EOF'
Port 22
ListenAddress 0.0.0.0
PidFile /run/sshd.pid
HostKey /etc/ssh/ssh_host_ed25519_key
PasswordAuthentication yes
KbdInteractiveAuthentication yes
PermitRootLogin no
UsePAM no
AllowUsers portmate
PrintMotd no
PermitUserEnvironment no
EOF

if [ "$mode" != "sftp-missing" ]; then
    printf '%s\n' 'Subsystem sftp internal-sftp' >> /run/portmate-sshd-config
fi

cat >> /run/portmate-sshd-config <<'EOF'
Match User portmate
    ForceCommand /usr/local/bin/portmate-ssh-health-force-command
EOF

exec /usr/sbin/sshd -D -e -f /run/portmate-sshd-config
