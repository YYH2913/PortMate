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
  normal|rejected|operation-denied) ;;
  *)
    echo "PORTMATE_GSSAPI_SFTP must be normal, rejected, or operation-denied" >&2
    exit 2
    ;;
esac

/usr/sbin/krb5kdc -n &
cd /home/portmate
exec runuser -u portmate -- env \
  HOME=/home/portmate \
  LOGNAME=portmate \
  USER=portmate \
  java \
  -Djava.security.krb5.conf=/etc/krb5.conf \
  -jar /opt/portmate/apache-mina-gssapi-server.jar
