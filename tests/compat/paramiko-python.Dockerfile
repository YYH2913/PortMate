ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

ENV DEBIAN_FRONTEND=noninteractive \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

ARG PARAMIKO_VERSION=5.0.0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates openssh-client passwd python3 python3-venv util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/portmate/venv \
    && /opt/portmate/venv/bin/python -m pip install --no-cache-dir "paramiko==${PARAMIKO_VERSION}" \
    && /opt/portmate/venv/bin/python -m pip check \
    && useradd --create-home --home-dir /home/portmate --shell /bin/sh portmate \
    && mkdir -p /etc/portmate /home/portmate/compat \
    && ssh-keygen -q -t ed25519 -N "" -f /etc/portmate/ssh_host_key \
    && chmod 0600 /etc/portmate/ssh_host_key \
    && chown -R portmate:portmate /etc/portmate /home/portmate

COPY tests/compat/paramiko-server.py /usr/local/bin/portmate-paramiko-server

EXPOSE 22

USER portmate

ENTRYPOINT ["/opt/portmate/venv/bin/python", "/usr/local/bin/portmate-paramiko-server"]
