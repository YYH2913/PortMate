ARG DEBIAN_TAG=bookworm-slim
FROM debian:${DEBIAN_TAG}

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        dialog \
        less \
        ncurses-bin \
        procps \
        util-linux \
        vim \
        vttest \
    && rm -rf /var/lib/apt/lists/*

COPY tests/compat/fullscreen-probe.txt /opt/portmate/fullscreen-probe.txt
