# syntax=docker/dockerfile:1.7

FROM node:24-bookworm AS web-builder
WORKDIR /work/web/control-panel

COPY web/control-panel/package.json web/control-panel/pnpm-lock.yaml ./
RUN --mount=type=cache,id=nostr-vpn-umbrel-pnpm-store,target=/pnpm/store \
    corepack enable \
    && corepack prepare pnpm@10.28.2 --activate \
    && pnpm install --frozen-lockfile --store-dir /pnpm/store

COPY web/control-panel ./
RUN pnpm run build

FROM rust:1.94-bookworm AS rust-builder
WORKDIR /work

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      clang \
      libclang-dev \
      libdbus-1-dev \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN --mount=type=cache,id=nostr-vpn-umbrel-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=nostr-vpn-umbrel-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=nostr-vpn-umbrel-cargo-target,target=/work/target \
    cargo build --release -p nvpn -p nostr-vpn-web \
    && mkdir -p /out \
    && cp /work/target/release/nvpn /out/nvpn \
    && cp /work/target/release/nostr-vpn-web /out/nvpn-web

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      iproute2 \
      iptables \
      iputils-ping \
      libdbus-1-3 \
      procps \
      wireguard-tools \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rust-builder /out/nvpn /usr/local/bin/nvpn
COPY --from=rust-builder /out/nvpn-web /usr/local/bin/nvpn-web
COPY --from=web-builder /work/web/control-panel/dist /usr/share/nostr-vpn/web

ENV HOME=/data/home \
    XDG_CONFIG_HOME=/data/config \
    NVPN_CLI_PATH=/usr/local/bin/nvpn \
    RUST_LOG=info

EXPOSE 38080
VOLUME ["/data"]

ENTRYPOINT ["nvpn-web"]
CMD ["--listen", "0.0.0.0:38080", "--config", "/data/config/nvpn/config.toml"]
