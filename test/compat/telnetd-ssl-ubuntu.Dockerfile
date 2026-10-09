FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends telnetd-ssl \
    && printf '#!/bin/sh\nexec /bin/sh\n' > /usr/local/bin/portmate-telnet-login \
    && chmod 0755 /usr/local/bin/portmate-telnet-login \
    && sed -i 's#^telnet[[:space:]].*#23\tstream\ttcp\tnowait\troot\t/usr/sbin/in.telnetd\tin.telnetd -L /usr/local/bin/portmate-telnet-login#' /etc/inetd.conf \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 23

CMD ["/usr/sbin/inetd", "-d"]
