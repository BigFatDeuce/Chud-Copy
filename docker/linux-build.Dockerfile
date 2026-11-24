# Compile et teste Chud Copy pour Linux.
# Ubuntu 22.04 : une glibc assez ancienne pour que le binaire tourne sur les distributions récentes.
FROM ubuntu:22.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl file git libayatana-appindicator3-dev libgtk-3-dev \
    librsvg2-dev libssl-dev libwebkit2gtk-4.1-dev libxdo-dev patchelf squashfs-tools unzip wget xz-utils xdg-utils

RUN curl -fsSL https://nodejs.org/dist/v24.11.0/node-v24.11.0-linux-x64.tar.xz | tar -xJ -C /opt \
    && ln -s /opt/node-v24.11.0-linux-x64/bin/node /usr/local/bin/node \
    && ln -s /opt/node-v24.11.0-linux-x64/bin/npm /usr/local/bin/npm \
    && ln -s /opt/node-v24.11.0-linux-x64/bin/npx /usr/local/bin/npx

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# Pas de FUSE dans un conteneur : les outils AppImage s'extraient eux-mêmes.
ENV APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=true
WORKDIR /src
