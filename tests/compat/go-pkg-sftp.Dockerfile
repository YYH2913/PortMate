FROM debian:trixie-slim AS build

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        golang-github-pkg-sftp-dev \
        golang-golang-x-crypto-dev \
        golang-go \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

COPY tests/compat/go-pkg-sftp/main.go ./main.go
RUN GO111MODULE=off GOPATH=/usr/share/gocode CGO_ENABLED=0 \
    go build -trimpath -ldflags="-s -w" -o /out/portmate-go-sftp ./main.go

FROM debian:bookworm-slim

RUN mkdir -p /home/portmate \
    && chmod 0700 /home/portmate

COPY --from=build /out/portmate-go-sftp /usr/local/bin/portmate-go-sftp

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/portmate-go-sftp"]
