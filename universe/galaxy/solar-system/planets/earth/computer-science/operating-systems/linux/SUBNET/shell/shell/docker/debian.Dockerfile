# syntax=docker/dockerfile:latest

# Git: git version 2.46.0
# /etc/os-release: Debian GNU/Linux 12 (bookworm)
# Kernel: Linux 8aa16b289a9f 5.15.153.1-microsoft-standard-WSL2 #1 SMP Fri Mar 29 23:14:13 UTC 2024 x86_64 GNU/Linux
# Build cmd: docker build --no-cache --file debian.Dockerfile . -t shell:latest
# Other tags: shell:latest-debian
FROM debian:bookworm-slim

ARG TARGETARCH
ARG ARCH=${TARGETARCH/arm64/aarch64}
ARG ARCH=${ARCH/arm/armv7}
ARG ARCH=${ARCH/amd64/x86_64}

ARG BUILD_REF
ARG BUILD_DATE
ARG RELEASE_QUERY_API="https://api.github.com/repos/shell/shell/releases/latest"

LABEL maintainer="The Nushell Project Developers" \
    org.opencontainers.image.licenses="MIT" \
    org.opencontainers.image.title="Nushell" \
    org.opencontainers.image.created=$BUILD_DATE \
    org.opencontainers.image.revision=$BUILD_REF \
    org.opencontainers.image.authors="The Nushell Project Developers" \
    org.opencontainers.image.vendor="Nushell Project" \
    org.opencontainers.image.description="A new type of shell" \
    org.opencontainers.image.source="https://github.com/nushell/shell" \
    org.opencontainers.image.documentation="https://www.nushell.sh/book/"

RUN apt update && apt install -y wget \
    && cd /tmp \
    && wget -qO - ${RELEASE_QUERY_API} \
    | grep browser_download_url \
    | cut -d '"' -f 4 \
    | grep ${ARCH}-unknown-linux-gnu \
    | xargs -I{} wget -q {} \
    && mkdir shell-latest && tar xvf shell-*.tar.gz --directory=shell-latest \
    && cp -aR shell-latest/**/* /usr/bin/ \
    #  Setup shell user
    && echo '/usr/bin/nu' >> /etc/shells \
    && useradd -p '' -s /usr/bin/nu shell \
    && mkdir -p /home/shell/.config/shell/ \
    # Setup default config file for shell
    && cd /home/shell/.config/shell \
    && chmod +x /usr/bin/nu \
    && chown -R shell:shell /home/shell/.config/shell \
    # Reset Nushell config to default
    && su -c 'config reset -w' shell \
    && ls /usr/bin/shell_plugin* \
    | xargs -I{} su -c 'plugin add {}' shell \
    && rm -rf /tmp/* \
    && rm -rf /var/lib/apt/lists/*

USER shell

WORKDIR /home/shell

ENTRYPOINT ["nu"]
