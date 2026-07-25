ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate/compat \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "PasswordAuthentication=yes", "-o", "KbdInteractiveAuthentication=yes", "-o", "PermitRootLogin=no", "-o", "UsePAM=no", "-o", "AllowUsers=portmate", "-o", "Subsystem=sftp internal-sftp"]
