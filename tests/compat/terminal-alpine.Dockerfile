ARG ALPINE_TAG=3.20
FROM alpine:${ALPINE_TAG}

RUN apk add --no-cache \
        dialog \
        less \
        ncurses \
        procps \
        util-linux-misc \
        vim

COPY tests/compat/fullscreen-probe.txt /opt/portmate/fullscreen-probe.txt
