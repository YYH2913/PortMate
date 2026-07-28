ARG PORTMATE_GSSAPI_BASE_IMAGE=ubuntu:24.04
FROM ${PORTMATE_GSSAPI_BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        krb5-user \
        openssh-server \
        samba \
        samba-ad-provision \
        samba-dsdb-modules \
        samba-vfs-modules \
        winbind \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh portmate \
    && printf '%s\n' 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /run/samba /var/cache/samba /var/lib/samba \
    && chown -R portmate:portmate /home/portmate

COPY tests/compat/gssapi-krb5.conf /etc/krb5.conf
COPY tests/compat/gssapi-samba-ad-entrypoint.sh /usr/local/bin/portmate-gssapi-samba-ad-entrypoint

RUN chmod 0755 /usr/local/bin/portmate-gssapi-samba-ad-entrypoint

EXPOSE 22 88/tcp

ENTRYPOINT ["/usr/local/bin/portmate-gssapi-samba-ad-entrypoint"]
