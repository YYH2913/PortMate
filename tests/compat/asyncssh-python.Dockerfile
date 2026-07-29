ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

ENV DEBIAN_FRONTEND=noninteractive

ARG ASYNCSSH_VERSION=2.24.0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates openssh-client passwd python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/portmate/venv \
    && /opt/portmate/venv/bin/python -m pip install --no-cache-dir "asyncssh==${ASYNCSSH_VERSION}" \
    && useradd --create-home --home-dir /home/portmate --shell /bin/sh portmate \
    && mkdir -p /etc/portmate /home/portmate/compat \
    && /opt/portmate/venv/bin/python -c "import asyncssh; asyncssh.generate_private_key('ssh-ed25519').write_private_key('/etc/portmate/ssh_host_key')" \
    && chmod 0600 /etc/portmate/ssh_host_key \
    && chown -R portmate:portmate /etc/portmate /home/portmate

COPY tests/compat/asyncssh-server.py /usr/local/bin/portmate-asyncssh-server

EXPOSE 22

USER portmate

ENTRYPOINT ["/opt/portmate/venv/bin/python", "/usr/local/bin/portmate-asyncssh-server"]
