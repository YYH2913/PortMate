FROM alpine:3.20

RUN apk add --no-cache openssh-server openssh-sftp-server python3 \
    && adduser -D -h /home/portmate -s /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate \
    && chown -R portmate:portmate /home/portmate

COPY tests/compat/ssh-health-fault-entrypoint.sh /usr/local/bin/portmate-ssh-health-fault-entrypoint
COPY tests/compat/ssh-health-force-command.sh /usr/local/bin/portmate-ssh-health-force-command
COPY tests/compat/sftp-health-fault-server.py /usr/local/bin/portmate-sftp-health-fault-server

RUN chmod 0755 \
    /usr/local/bin/portmate-ssh-health-fault-entrypoint \
    /usr/local/bin/portmate-ssh-health-force-command \
    /usr/local/bin/portmate-sftp-health-fault-server

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/portmate-ssh-health-fault-entrypoint"]
