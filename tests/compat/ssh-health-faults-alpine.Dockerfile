FROM alpine:3.20

RUN apk add --no-cache openssh-server openssh-sftp-server \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate \
    && chown -R portmate:portmate /home/portmate

COPY tests/compat/ssh-health-fault-entrypoint.sh /usr/local/bin/portmate-ssh-health-fault-entrypoint
COPY tests/compat/ssh-health-force-command.sh /usr/local/bin/portmate-ssh-health-force-command

RUN chmod 0755 \
    /usr/local/bin/portmate-ssh-health-fault-entrypoint \
    /usr/local/bin/portmate-ssh-health-force-command

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/portmate-ssh-health-fault-entrypoint"]
