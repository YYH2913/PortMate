#!/bin/sh
set -eu

cat > /run/proftpd/proftpd.conf <<'EOF'
ServerName "PortMate ProFTPD SFTP compatibility"
ServerType standalone
DefaultServer on
ModulePath /usr/lib/proftpd
Include /etc/proftpd/modules.d/
Port 22
UseIPv6 off
User root
Group root
RequireValidShell off
AuthOrder mod_auth_unix.c
MaxInstances 32
Umask 022
AllowOverwrite on
<IfModule mod_sftp.c>
    SFTPEngine on
    SFTPLog /var/log/proftpd/sftp.log
    SFTPHostKey /etc/proftpd/ssh_host_ed25519_key
    SFTPAuthMethods password
</IfModule>
<Global>
    RequireValidShell off
</Global>
EOF

exec /usr/sbin/proftpd --nodaemon -c /run/proftpd/proftpd.conf
