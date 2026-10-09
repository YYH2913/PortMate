ARG ALPINE_TAG=3.21
FROM alpine:${ALPINE_TAG}

RUN apk add --no-cache coreutils procps tmux

ENTRYPOINT ["sleep", "infinity"]
