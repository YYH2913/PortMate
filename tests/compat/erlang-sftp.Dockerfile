ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        erlang-base \
        erlang-ssh \
        openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /etc/portmate-erlang-ssh /home/portmate \
    && ssh-keygen -q -t rsa -b 3072 -m PEM -N '' \
        -f /etc/portmate-erlang-ssh/ssh_host_rsa_key

COPY tests/compat/erlang-sftp-entrypoint.escript /usr/local/bin/portmate-erlang-sftp

RUN chmod 0755 /usr/local/bin/portmate-erlang-sftp

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/portmate-erlang-sftp"]
