FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends lrzsz openssh-server openssh-client ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate/compat \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "PasswordAuthentication=yes", "-o", "KbdInteractiveAuthentication=yes", "-o", "PermitRootLogin=no", "-o", "UsePAM=no", "-o", "AllowUsers=portmate"]
