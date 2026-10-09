ARG DEBIAN_VERSION=bookworm-slim
FROM debian:${DEBIAN_VERSION}

ARG RCLONE_VERSION
ARG RCLONE_SHA256

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        unzip \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --retry 3 --retry-all-errors \
        "https://github.com/rclone/rclone/releases/download/v${RCLONE_VERSION}/rclone-v${RCLONE_VERSION}-linux-amd64.zip" \
        --output /tmp/rclone.zip \
    && printf '%s  %s\n' "${RCLONE_SHA256}" /tmp/rclone.zip | sha256sum --check --strict \
    && unzip /tmp/rclone.zip -d /tmp/rclone \
    && install -m 0755 \
        "/tmp/rclone/rclone-v${RCLONE_VERSION}-linux-amd64/rclone" \
        /usr/local/bin/rclone \
    && mkdir -p /srv/portmate \
    && rm -rf /tmp/rclone.zip /tmp/rclone

EXPOSE 22

ENTRYPOINT ["rclone", "serve", "sftp", "/srv/portmate", "--addr", ":22", "--user", "portmate", "--pass", "portmate", "--log-level", "NOTICE"]
