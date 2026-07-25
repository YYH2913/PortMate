ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache nmap-ncat socat

COPY tests/compat/tcp-compat-server.sh /usr/local/bin/tcp-compat-server
RUN chmod 0755 /usr/local/bin/tcp-compat-server

EXPOSE 23

ENTRYPOINT ["/usr/local/bin/tcp-compat-server"]
