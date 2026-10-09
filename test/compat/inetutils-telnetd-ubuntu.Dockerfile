FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends inetutils-telnetd socat \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 23

CMD ["socat", "TCP-LISTEN:23,reuseaddr,fork", "EXEC:/usr/sbin/telnetd -E /bin/sh"]
