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
# The Node base image supplies the Node >= 22.19 runtime and npm that the Pi
# dependency needs. Alfred delegates the agent loop, providers, tools, and
# sessions to Pi, so an image without Pi cannot run a single job (and, because
# startup probes the binary, cannot even boot the server).
FROM node:22-bookworm-slim

# curl is used by the entrypoint health wait and by smoke tests.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Pi is a RUNTIME dependency, not a build dependency. Install it from npm (the
# same method and package .github/workflows/release.yml pins) and fail the build
# loudly if it is missing, rather than shipping a container that cannot run
# jobs. The version is read from the release workflow instead of being retyped,
# so the image and the release pipeline share one pin and cannot drift; `.github`
# is kept in the build context for exactly this reason (see .dockerignore).
COPY .github/workflows/release.yml /tmp/pi-version.yml
RUN PI_VERSION="$(sed -n 's/^[[:space:]]*PI_VERSION:[[:space:]]*"\([^"]*\)".*/\1/p' /tmp/pi-version.yml)" \
    && test -n "$PI_VERSION" \
    && npm install -g "@earendil-works/pi-coding-agent@${PI_VERSION}" \
    && rm -f /tmp/pi-version.yml \
    && command -v pi \
    && pi --version

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
