ARG DEBIAN_TAG=bookworm-slim
FROM debian:${DEBIAN_TAG}

ARG TMUX_VERSION
ARG TMUX_SHA256

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        bison \
        build-essential \
        ca-certificates \
        coreutils \
        curl \
        libevent-dev \
        libncurses-dev \
        pkg-config \
        procps \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --retry 3 --retry-all-errors \
        "https://github.com/tmux/tmux/releases/download/${TMUX_VERSION}/tmux-${TMUX_VERSION}.tar.gz" \
        --output /tmp/tmux.tar.gz \
    && printf '%s  %s\n' "${TMUX_SHA256}" /tmp/tmux.tar.gz | sha256sum --check --strict \
    && mkdir /tmp/tmux-source \
    && tar --extract --gzip --file /tmp/tmux.tar.gz --directory /tmp/tmux-source --strip-components=1 \
    && cd /tmp/tmux-source \
    && ./configure --prefix=/usr/local \
    && make -j2 \
    && make install \
    && rm -rf /tmp/tmux.tar.gz /tmp/tmux-source

ENTRYPOINT ["sleep", "infinity"]
