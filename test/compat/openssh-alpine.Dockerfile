ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache build-base curl gzip tar openssh-client openssh-server openssh-sftp-server \
    && curl -fsSL https://ohse.de/uwe/releases/lrzsz-0.12.20.tar.gz -o /tmp/lrzsz.tar.gz \
    && tar -xzf /tmp/lrzsz.tar.gz -C /tmp \
    && cd /tmp/lrzsz-0.12.20 \
    && CC="gcc -std=gnu89" ./configure --prefix=/usr \
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
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate/compat \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "PasswordAuthentication=yes", "-o", "KbdInteractiveAuthentication=yes", "-o", "PermitRootLogin=no", "-o", "UsePAM=no", "-o", "AllowUsers=portmate", "-o", "Subsystem=sftp internal-sftp"]
