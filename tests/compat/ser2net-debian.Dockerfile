ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ser2net socat \
    && rm -rf /var/lib/apt/lists/*

COPY tests/compat/ser2net.yaml /etc/ser2net/ser2net.yaml
COPY tests/compat/ser2net-entrypoint.sh /usr/local/bin/portmate-ser2net
RUN chmod 0755 /usr/local/bin/portmate-ser2net

EXPOSE 23

ENTRYPOINT ["/usr/local/bin/portmate-ser2net"]
