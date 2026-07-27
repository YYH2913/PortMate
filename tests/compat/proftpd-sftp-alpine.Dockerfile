FROM alpine:3.21

RUN apk add --no-cache proftpd proftpd-mod_sftp openssh-keygen \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && mkdir -p /run/proftpd /var/log/proftpd /home/portmate \
    && chown -R portmate:portmate /home/portmate \
    && ssh-keygen -t ed25519 -f /etc/proftpd/ssh_host_ed25519_key -N ''

COPY tests/compat/proftpd-sftp-entrypoint.sh /usr/local/bin/portmate-proftpd-sftp-entrypoint

RUN chmod 0755 /usr/local/bin/portmate-proftpd-sftp-entrypoint

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/portmate-proftpd-sftp-entrypoint"]
