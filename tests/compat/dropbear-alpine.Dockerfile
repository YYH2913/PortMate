ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache build-base curl gzip tar dropbear openssh-client openssh-sftp-server \
    && curl -fsSL https://ohse.de/uwe/releases/lrzsz-0.12.20.tar.gz -o /tmp/lrzsz.tar.gz \
    && tar -xzf /tmp/lrzsz.tar.gz -C /tmp \
    && cd /tmp/lrzsz-0.12.20 \
    && ./configure --prefix=/usr \
    && make -j2 \
    && make install \
    && ln -sf /usr/bin/lrz /usr/bin/rx \
    && ln -sf /usr/bin/lrz /usr/bin/rb \
    && ln -sf /usr/bin/lrz /usr/bin/rz \
    && ln -sf /usr/bin/lsx /usr/bin/sx \
    && ln -sf /usr/bin/lsb /usr/bin/sb \
    && ln -sf /usr/bin/lsz /usr/bin/sz \
    && rm -rf /tmp/lrzsz.tar.gz /tmp/lrzsz-0.12.20 \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && mkdir -p /etc/dropbear /home/portmate/compat /usr/libexec \
    && dropbearkey -t ed25519 -f /etc/dropbear/dropbear_ed25519_host_key \
    && ln -sf /usr/lib/ssh/sftp-server /usr/libexec/sftp-server \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/dropbear", "-F", "-E", "-p", "0.0.0.0:22"]
