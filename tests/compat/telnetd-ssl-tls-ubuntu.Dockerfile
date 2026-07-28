FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends openssl telnetd-ssl \
    && printf '#!/bin/sh\nexec /bin/sh\n' > /usr/local/bin/portmate-telnet-login \
    && chmod 0755 /usr/local/bin/portmate-telnet-login \
    && openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
        -subj '/CN=localhost' \
        -addext 'subjectAltName=DNS:localhost' \
        -keyout /etc/ssl/private/portmate-telnet.key \
        -out /etc/ssl/certs/portmate-telnet.crt \
    && chmod 0600 /etc/ssl/private/portmate-telnet.key \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 23

CMD ["/usr/sbin/in.telnetd", "-debug", "23", "-L", "/usr/local/bin/portmate-telnet-login", "-z", "ssl", "-z", "cert=/etc/ssl/certs/portmate-telnet.crt", "-z", "key=/etc/ssl/private/portmate-telnet.key"]
