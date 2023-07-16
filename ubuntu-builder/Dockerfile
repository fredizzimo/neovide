from ubuntu:18.04

RUN DEBIAN_FRONTEND=noninteractive \
    apt update \
    && apt install -y curl \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

ENV PATH="/root/.cargo/bin:${PATH}"
ENV CARGO_TARGET_DIR=target-ubuntu
ENV CARGO_HOME=target-ubuntu/cache

RUN DEBIAN_FRONTEND=noninteractive \
    apt update \
    && apt install -y \
        curl gnupg ca-certificates git gcc-multilib g++-multilib cmake \
        libssl-dev pkg-config libfreetype6-dev libasound2-dev \
        libexpat1-dev libxcb-composite0-dev libbz2-dev freeglut3-dev \
        libxi-dev libfuse2 appstream \
    && rm -rf /var/lib/apt/lists/*

RUN curl -Lo linuxdeploy.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/latest/download/linuxdeploy-x86_64.AppImage \
    && chmod +x linuxdeploy.AppImage \
    && ./linuxdeploy.AppImage --appimage-extract \
    && mv squashfs-root/ /opt/linuxdeploy/ \
    && ln -s /opt/linuxdeploy/AppRun /usr/local/bin/linuxdeploy \
    && rm linuxdeploy.AppImage

RUN DEBIAN_FRONTEND=noninteractive \
    apt update \
    && apt install -y \
        libfontconfig-dev file libpng16-16 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -Lo appimage-builder.AppImage https://github.com/AppImageCrafters/appimage-builder/releases/download/v0.9.2/appimage-builder-0.9.2-35e3eab-x86_64.AppImage \
    && chmod +x appimage-builder.AppImage \
    && ./appimage-builder.AppImage --appimage-extract \
    && mv squashfs-root/ /opt/appimage-builder/ \
    && ln -s /opt/appimage-builder/AppRun /usr/local/bin/appimage-builder \
    && rm appimage-builder.AppImage

RUN DEBIAN_FRONTEND=noninteractive \
    apt update \
    && apt install -y \
        fakeroot \
    && rm -rf /var/lib/apt/lists/*
