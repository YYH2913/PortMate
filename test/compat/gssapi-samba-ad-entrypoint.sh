#!/bin/sh
set -eu

case "${PORTMATE_GSSAPI_AUTH:-yes}" in
  yes|no) ;;
  *)
    echo "PORTMATE_GSSAPI_AUTH must be yes or no" >&2
    exit 2
    ;;
esac

case "${PORTMATE_GSSAPI_SFTP:-normal}" in
  normal)
    sftp_subsystem='Subsystem=sftp internal-sftp'
    ;;
  rejected)
    sftp_subsystem='Subsystem=sftp /bin/false'
    ;;
  operation-denied)
    mkdir -p /home/portmate/portmate-sftp-blocked
    chown portmate:portmate /home/portmate/portmate-sftp-blocked
    chmod 0111 /home/portmate/portmate-sftp-blocked
    sftp_subsystem='Subsystem=sftp internal-sftp -d /home/portmate/portmate-sftp-blocked'
    ;;
  *)
    echo "PORTMATE_GSSAPI_SFTP must be normal, rejected, or operation-denied" >&2
    exit 2
    ;;
esac

if [ "$(hostname)" != "localhost" ]; then
  echo "Samba AD-compatible GSSAPI tests require --hostname localhost" >&2
  exit 2
fi

rm -f /etc/samba/smb.conf
samba-tool domain provision \
  --realm=PORTMATE.TEST \
  --domain=PORTMATE \
  --server-role=dc \
  --dns-backend=NONE \
  --host-name=localhost \
  --function-level=2008_R2 \
  --use-rfc2307 \
  --adminpass='Portmate-Admin-42' \
  --quiet
samba-tool user create portmate 'Portmate-User-42'
/usr/local/bin/portmate-configure-samba-ad
samba-tool domain exportkeytab /etc/krb5.keytab \
  --principal=host/localhost@PORTMATE.TEST
chmod 0600 /etc/krb5.keytab

export PORTMATE_GSSAPI_SFTP_OPTION="$sftp_subsystem"
exec setpriv --bounding-set=-sys_admin /bin/sh -c '
  samba --foreground --no-process-group --debug-stdout &
  exec /usr/sbin/sshd -D -e -f /dev/null \
    -o "GSSAPIAuthentication=${PORTMATE_GSSAPI_AUTH:-yes}" \
    -o GSSAPICleanupCredentials=yes \
    -o GSSAPIStrictAcceptorCheck=yes \
    -o LogLevel=VERBOSE \
    -o PasswordAuthentication=yes \
    -o KbdInteractiveAuthentication=no \
    -o PubkeyAuthentication=no \
    -o PermitRootLogin=no \
    -o UsePAM=no \
    -o AllowUsers=portmate \
    -o "$PORTMATE_GSSAPI_SFTP_OPTION"
'
