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

/usr/sbin/krb5kdc -n &
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
  -o "$sftp_subsystem"
