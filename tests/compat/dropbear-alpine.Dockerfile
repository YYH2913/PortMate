ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache dropbear openssh-client openssh-sftp-server \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && mkdir -p /etc/dropbear /home/portmate/compat /usr/libexec \
    && dropbearkey -t ed25519 -f /etc/dropbear/dropbear_ed25519_host_key \
    && ln -sf /usr/lib/ssh/sftp-server /usr/libexec/sftp-server \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/dropbear", "-F", "-E", "-p", "0.0.0.0:22"]
