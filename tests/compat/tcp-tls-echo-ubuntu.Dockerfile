FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ncat openssl \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0700 /etc/portmate-tls \
    && openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 3650 \
        -subj "/CN=localhost" \
        -addext "subjectAltName=DNS:localhost" \
        -keyout /etc/portmate-tls/server.key \
        -out /etc/portmate-tls/server.crt \
    && chmod 0600 /etc/portmate-tls/server.key \
    && chmod 0644 /etc/portmate-tls/server.crt

EXPOSE 23

ENTRYPOINT ["ncat", "--listen", "--keep-open", "--ssl", "--ssl-cert", "/etc/portmate-tls/server.crt", "--ssl-key", "/etc/portmate-tls/server.key", "--exec", "/bin/cat", "23"]
