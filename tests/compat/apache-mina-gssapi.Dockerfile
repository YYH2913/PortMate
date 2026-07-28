FROM ubuntu:24.04 AS build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends maven openjdk-21-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
COPY tests/compat/apache-mina-gssapi/pom.xml ./pom.xml
COPY tests/compat/apache-mina-gssapi/src ./src
RUN mvn --batch-mode --no-transfer-progress package

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        krb5-admin-server \
        krb5-kdc \
        openjdk-21-jre-headless \
        util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh portmate

RUN mkdir -p \
        /etc/krb5kdc \
        /opt/portmate \
        /srv/portmate/blocked \
        /srv/portmate/home/portmate \
        /var/lib/portmate

COPY tests/compat/gssapi-krb5.conf /etc/krb5.conf
COPY tests/compat/gssapi-kdc.conf /etc/krb5kdc/kdc.conf
COPY tests/compat/apache-mina-gssapi-entrypoint.sh /usr/local/bin/portmate-apache-mina-gssapi-entrypoint
COPY --from=build /workspace/target/apache-mina-gssapi-server.jar /opt/portmate/apache-mina-gssapi-server.jar

RUN printf '%s\n' '*/admin@PORTMATE.TEST *' > /etc/krb5kdc/kadm5.acl \
    && kdb5_util create -s -r PORTMATE.TEST -P portmate-master \
    && kadmin.local -r PORTMATE.TEST -q 'addprinc -randkey host/localhost@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'ktadd -k /etc/krb5.keytab -norandkey host/localhost@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'addprinc -randkey portmate@PORTMATE.TEST' \
    && kadmin.local -r PORTMATE.TEST -q 'ktadd -k /portmate-client.keytab -norandkey portmate@PORTMATE.TEST' \
    && chown root:portmate /etc/krb5.keytab \
    && chmod 0640 /etc/krb5.keytab \
    && chmod 0644 /portmate-client.keytab \
    && chown -R portmate:portmate /home/portmate /srv/portmate/home/portmate /var/lib/portmate \
    && chown root:root /srv/portmate/blocked \
    && chmod 0111 /srv/portmate/blocked \
    && chmod 0755 /usr/local/bin/portmate-apache-mina-gssapi-entrypoint

EXPOSE 2222/tcp 88/tcp

ENTRYPOINT ["/usr/local/bin/portmate-apache-mina-gssapi-entrypoint"]
