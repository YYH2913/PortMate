ARG PORTMATE_GSSAPI_BASE_IMAGE=ubuntu:24.04
FROM ${PORTMATE_GSSAPI_BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends krb5-kdc openssh-server \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh portmate \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /etc/krb5kdc \
    && chown -R portmate:portmate /home/portmate

RUN apt-get update \
    && apt-get install --yes --no-install-recommends krb5-admin-server \
    && rm -rf /var/lib/apt/lists/*

RUN passwd -d portmate

COPY tests/compat/gssapi-krb5.conf /etc/krb5.conf
COPY tests/compat/gssapi-kdc.conf /etc/krb5kdc/kdc.conf
COPY tests/compat/gssapi-openssh-entrypoint.sh /usr/local/bin/portmate-gssapi-entrypoint

RUN printf '%s\n' '*/admin@PORTMATE.TEST *' > /etc/krb5kdc/kadm5.acl \
    && kdb5_util create -s -r PORTMATE.TEST -P portmate-master \
    && kadmin.local -r PORTMATE.TEST -q 'addprinc -randkey host/localhost@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'ktadd -k /etc/krb5.keytab -norandkey host/localhost@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'addprinc -randkey portmate@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'ktadd -k /portmate-client.keytab -norandkey portmate@PORTMATE.TEST' \
    && chmod 0644 /portmate-client.keytab \
    && chmod 0755 /usr/local/bin/portmate-gssapi-entrypoint

EXPOSE 22 88/tcp

ENTRYPOINT ["/usr/local/bin/portmate-gssapi-entrypoint"]
