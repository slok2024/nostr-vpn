FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH=/root/.cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    libgtk-4-dev \
    libadwaita-1-dev \
    librsvg2-bin \
    adwaita-icon-theme \
    dbus-x11 \
    fluxbox \
    imagemagick \
    fonts-dejavu \
    fonts-noto-color-emoji \
    x11-apps \
    x11-utils \
    x11vnc \
    xdotool \
    xvfb \
    zbar-tools \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable

COPY linux/scripts/dev-entrypoint.sh /usr/local/bin/dev-entrypoint
RUN chmod +x /usr/local/bin/dev-entrypoint

WORKDIR /workspace/nostr-vpn/linux
ENTRYPOINT ["/usr/local/bin/dev-entrypoint"]
