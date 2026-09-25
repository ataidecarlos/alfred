# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1: compile Alfred from source.
# ---------------------------------------------------------------------------
FROM rust:1-bookworm AS builder

# OpenSSL headers are required because reqwest uses native-tls on Linux.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# ---------------------------------------------------------------------------
# Stage 2: minimal runtime image.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

# curl is used by the entrypoint health wait and by smoke tests.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 --shell /usr/sbin/nologin alfred

WORKDIR /opt/alfred
COPY --from=builder /build/target/release/alfred /usr/local/bin/alfred
COPY docker/config.toml /opt/alfred/docker/config.toml
COPY prompts /opt/alfred/prompts
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENV HOME=/home/alfred
USER alfred
EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
