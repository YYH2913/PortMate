FROM ghcr.io/drakkan/sftpgo:v2.7.5@sha256:9011fe608d336d3daf6ed6224b16fd40443aab8f3335e0b20853d6a539c58738

USER root

RUN mkdir -p /srv/portmate /var/lib/sftpgo

EXPOSE 22

ENTRYPOINT ["sftpgo", "portable", "--config-dir", "/var/lib/sftpgo", "--directory", "/srv/portmate", "--username", "portmate", "--password", "portmate", "--permissions", "*", "--sftpd-port", "22", "--log-file-path", "", "--log-level", "warn"]
