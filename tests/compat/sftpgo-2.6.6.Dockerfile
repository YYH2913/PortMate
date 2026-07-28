FROM ghcr.io/drakkan/sftpgo:v2.6.6@sha256:ca17d735651ce1b5c54a8fa2d4fb9c85036d4137e32b700de260324619ff3f88

USER root

RUN mkdir -p /srv/portmate /var/lib/sftpgo

EXPOSE 22

ENTRYPOINT ["sftpgo", "portable", "--config-dir", "/var/lib/sftpgo", "--directory", "/srv/portmate", "--username", "portmate", "--password", "portmate", "--permissions", "*", "--sftpd-port", "22", "--log-file-path", "", "--log-level", "warn"]
