# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Alfred e2e image: a pristine, self-contained deployment used for continuous
# feature checks. Mirrors the root Dockerfile but ships an OpenCode Go config
# and a container-level HEALTHCHECK so `docker inspect ... State.Health.Status`
# can gate the harness.
# ---------------------------------------------------------------------------

# Stage 1: compile Alfred from source.
FROM rust:1-bookworm AS builder

# OpenSSL headers are required because reqwest uses native-tls on Linux.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# Stage 2: minimal runtime image.
FROM debian:bookworm-slim

# curl is required by the HEALTHCHECK and by host-side smoke tests.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 --shell /usr/sbin/nologin alfred

WORKDIR /opt/alfred
COPY --from=builder /build/target/release/alfred /usr/local/bin/alfred
COPY prompts /opt/alfred/prompts
COPY docker/e2e/config.toml /opt/alfred/config.toml

# Seed the default config/prompts locations under the container user's HOME.
# Alfred loads $HOME/.alfred/config/config.toml by default.
RUN mkdir -p /home/alfred/.alfred/config/prompts /home/alfred/.alfred/data \
    && cp /opt/alfred/config.toml /home/alfred/.alfred/config/config.toml \
    && cp /opt/alfred/prompts/system.md.example /home/alfred/.alfred/config/prompts/system.md \
    && cp /opt/alfred/prompts/user.md.example /home/alfred/.alfred/config/prompts/user.md \
    && chown -R alfred:alfred /home/alfred

ENV HOME=/home/alfred
ENV RUST_LOG=info
USER alfred
EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:3000/health || exit 1

ENTRYPOINT ["alfred"]
