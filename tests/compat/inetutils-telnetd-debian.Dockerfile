ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends inetutils-telnetd socat \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 23

CMD ["socat", "TCP-LISTEN:23,reuseaddr,fork", "EXEC:/usr/sbin/telnetd -E /bin/sh"]
