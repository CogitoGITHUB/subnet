# syntax=docker/dockerfile:latest

# Git: git version 2.30.2
# /etc/os-release: Alpine Linux v3.20
# Kernel: Linux ca3abedc4fb1 5.17.15-76051715-generic #202206141358~1655919116~22.04~1db9e34 SMP PREEMPT Wed Jun 22 19 x86_64 Linux
# Build cmd: docker build --no-cache . -t shell:latest
# Other tags: shell:latest-alpine, shell
FROM alpine

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

RUN echo '/usr/bin/nu' >> /etc/shells \
    && adduser -D -s /usr/bin/nu shell \
    && mkdir -p /home/shell/.config/shell/ \
    && cd /tmp \
    && wget -qO - ${RELEASE_QUERY_API} \
    | grep browser_download_url \
    | cut -d '"' -f 4 \
    | grep ${ARCH}-unknown-linux-musl \
    | xargs -I{} wget {} \
    && mkdir shell-latest && tar xvf shell-*.tar.gz --directory=shell-latest \
    && cp -aR shell-latest/**/* /usr/bin/ \
    # Setup default config file for shell
    && cd /home/shell/.config/shell \
    && chmod +x /usr/bin/nu \
    && chown -R shell:shell /home/shell/.config/shell \
    # Reset Nushell config to default
    && su -c 'config reset -w' shell \
    && ls /usr/bin/shell_plugin* \
    | xargs -I{} su -c 'plugin add {}' shell \
    && rm -rf /tmp/*

USER shell

WORKDIR /home/shell

ENTRYPOINT ["nu"]
