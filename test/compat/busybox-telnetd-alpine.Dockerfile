ARG ALPINE_VERSION=3.20
FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache busybox-extras

EXPOSE 23

CMD ["telnetd", "-F", "-K", "-p", "23", "-l", "/bin/sh"]
