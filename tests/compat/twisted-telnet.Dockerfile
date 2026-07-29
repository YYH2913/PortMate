ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

ENV DEBIAN_FRONTEND=noninteractive \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

ARG TWISTED_VERSION

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/portmate/venv \
    && /opt/portmate/venv/bin/python -m pip install --no-cache-dir \
        "Twisted==${TWISTED_VERSION}" \
    && /opt/portmate/venv/bin/python -m pip check

COPY tests/compat/twisted-telnet-server.py /usr/local/bin/portmate-twisted-telnet
RUN chmod 0755 /usr/local/bin/portmate-twisted-telnet

EXPOSE 23

ENTRYPOINT ["/opt/portmate/venv/bin/python", "/usr/local/bin/portmate-twisted-telnet"]
