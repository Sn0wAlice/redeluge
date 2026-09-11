# syntax=docker/dockerfile:1

# redeluge: the daemon and the Web UI, in one image.
#
# Two Rust binaries and the C++ libtorrent they drive. No Python, no virtualenv,
# no interpreter: the Web UI assets are compiled into redeluge-web, so the whole
# runtime is two files and a shared library.
#
# Upgrading is a deliberate act. The application is whatever is checked out
# here, dependency versions are pinned in Cargo.lock, and libtorrent comes from
# the distribution rather than floating.

ARG RUST_TAG=1.98.1-slim-trixie
ARG DEBIAN_TAG=trixie-slim

# ---------------------------------------------------------------- build stage
FROM rust:${RUST_TAG} AS builder

ENV CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0

# libtorrent from Debian: trixie ships 2.0.11, the version Deluge is tested
# against, with the pkg-config metadata the bridge's build script needs.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        libssl-dev \
        libtorrent-rasterbar-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Dependencies in their own layer, so editing the application does not refetch
# them. Building them separately would need dummy sources, which go stale in
# ways that are hard to see; fetching is most of the wait and none of the risk.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY contract ./contract
COPY tools ./tools
RUN cargo fetch --locked

RUN cargo build --release --locked --bin redeluged --bin redeluge-web

# The Web UI assets must have made it into the binary, or the page loads empty.
RUN test "$(strings target/release/redeluge-web | grep -c 'deluge-all')" -gt 0

# -------------------------------------------------------------- runtime stage
FROM debian:${DEBIAN_TAG}

ENV DELUGE_CONFIG_DIR=/config \
    DELUGE_DOWNLOAD_DIR=/downloads \
    DELUGE_WEB_INTERFACE=0.0.0.0 \
    DELUGE_WEB_PORT=8112 \
    DELUGE_WEB_BASE=/ \
    RUST_LOG=info \
    PUID=1000 \
    PGID=1000 \
    UMASK=022

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        gosu \
        libtorrent-rasterbar2.0t64 \
        tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 1000 redeluge \
    && useradd -u 1000 -g redeluge -d /config -s /usr/sbin/nologin redeluge

COPY --from=builder /src/target/release/redeluged /usr/local/bin/redeluged
COPY --from=builder /src/target/release/redeluge-web /usr/local/bin/redeluge-web
COPY --from=builder /src/tools/migrate_state.py /usr/local/share/redeluge/migrate_state.py

COPY docker/entrypoint.sh docker/run.sh /app/
RUN chmod +x /app/entrypoint.sh /app/run.sh

VOLUME ["/config", "/downloads"]

# 8112 Web UI, 58846 daemon RPC, 58946 BitTorrent peers
EXPOSE 8112 58846 58946 58946/udp

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD redeluge-web --health-check || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/app/entrypoint.sh"]
