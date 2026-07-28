#!/bin/sh
set -eu

case "${PORTMATE_GSSAPI_AUTH:-yes}" in
  yes|no) ;;
  *)
    echo "PORTMATE_GSSAPI_AUTH must be yes or no" >&2
    exit 2
    ;;
esac

/usr/sbin/krb5kdc -n &
exec /usr/sbin/sshd -D -e -f /dev/null \
  -o "GSSAPIAuthentication=${PORTMATE_GSSAPI_AUTH:-yes}" \
  -o GSSAPICleanupCredentials=yes \
  -o GSSAPIStrictAcceptorCheck=yes \
  -o LogLevel=VERBOSE \
  -o PasswordAuthentication=no \
  -o KbdInteractiveAuthentication=no \
  -o PubkeyAuthentication=no \
  -o PermitRootLogin=no \
  -o UsePAM=no \
  -o AllowUsers=portmate \
  -o 'Subsystem=sftp internal-sftp'
