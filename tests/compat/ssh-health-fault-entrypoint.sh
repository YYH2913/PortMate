#!/bin/sh
set -eu

mode="${PORTMATE_SSH_HEALTH_FAULT:-normal}"
case "$mode" in
    normal|ping-unresponsive|transport-closed|exec-rejected|exec-silent|exec-wrong-marker|sftp-missing|sftp-rejected|sftp-silent|sftp-canonicalize-silent-once|sftp-opendir-silent-once|sftp-readdir-silent-once|sftp-canonicalize-missing|sftp-operation-denied|sftp-no-such-file|sftp-permission-denied|scp-rejected|runtime-replaced) ;;
    *)
        echo "unsupported SSH health fault mode: $mode" >&2
        exit 64
        ;;
esac

printf '%s\n' "$mode" > /run/portmate-ssh-health-fault

if [ "$mode" = "sftp-permission-denied" ]; then
    mkdir -p /home/portmate/portmate-readonly
    chown portmate:portmate /home/portmate/portmate-readonly
    chmod 0555 /home/portmate/portmate-readonly
fi

if [ "$mode" = "sftp-operation-denied" ]; then
    mkdir -p /home/portmate/portmate-sftp-blocked
    chown portmate:portmate /home/portmate/portmate-sftp-blocked
    # Allow sftp-server to enter the directory, but reject READDIR probes.
    chmod 0111 /home/portmate/portmate-sftp-blocked
fi

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
