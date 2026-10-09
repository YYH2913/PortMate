ARG FEDORA_VERSION=44
FROM registry.fedoraproject.org/fedora:${FEDORA_VERSION}

RUN dnf install --assumeyes \
        --setopt=install_weak_deps=False \
        --setopt=minrate=1000 \
        --setopt=retries=3 \
        --setopt=timeout=30 \
        lrzsz \
        openssh-clients \
        openssh-server \
        shadow-utils \
    && dnf clean all \
    && rm -rf /var/cache/dnf \
    && useradd --create-home --shell /bin/sh portmate \
    && echo 'portmate:portmate' | chpasswd \
    && ssh-keygen -A \
    && mkdir -p /run/sshd /home/portmate/compat \
    && chown -R portmate:portmate /home/portmate

EXPOSE 22

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "PasswordAuthentication=yes", "-o", "KbdInteractiveAuthentication=yes", "-o", "PermitRootLogin=no", "-o", "UsePAM=no", "-o", "AllowUsers=portmate", "-o", "Subsystem=sftp internal-sftp"]
