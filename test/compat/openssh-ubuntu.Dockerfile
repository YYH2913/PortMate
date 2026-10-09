ARG UBUNTU_VERSION=24.04
FROM ubuntu:${UBUNTU_VERSION}

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates lrzsz openssh-client openssh-server \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && sed -i '/^[[:space:]]*Subsystem[[:space:]]/d' /etc/ssh/sshd_config \
    && mkdir -p /run/sshd /home/portmate/compat \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "PasswordAuthentication=yes", "-o", "KbdInteractiveAuthentication=yes", "-o", "PermitRootLogin=no", "-o", "UsePAM=no", "-o", "AllowUsers=portmate", "-o", "Subsystem=sftp internal-sftp"]
