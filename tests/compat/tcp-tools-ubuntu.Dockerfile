FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ncat socat \
    && rm -rf /var/lib/apt/lists/*

COPY tests/compat/tcp-compat-server.sh /usr/local/bin/tcp-compat-server
RUN chmod 0755 /usr/local/bin/tcp-compat-server

EXPOSE 23

ENTRYPOINT ["/usr/local/bin/tcp-compat-server"]
