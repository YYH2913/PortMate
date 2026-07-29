ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

ENV DEBIAN_FRONTEND=noninteractive \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

ARG TELNETLIB3_VERSION
ARG WCWIDTH_VERSION

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/portmate/venv \
    && /opt/portmate/venv/bin/python -m pip install --no-cache-dir \
        "wcwidth==${WCWIDTH_VERSION}" \
        "telnetlib3==${TELNETLIB3_VERSION}" \
    && /opt/portmate/venv/bin/python -m pip check

EXPOSE 23

ENTRYPOINT ["/opt/portmate/venv/bin/telnetlib3-server", "0.0.0.0", "23", "--loglevel", "warning", "--connect-maxwait", "1.0", "--line-mode", "--pty-exec", "/bin/sh"]
