# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Alfred e2e image: a pristine, self-contained deployment used for continuous
# feature checks. Mirrors the root Dockerfile but ships an OpenCode Go config
# and a container-level HEALTHCHECK so `docker inspect ... State.Health.Status`
# can gate the harness.
#
# The suite is driven by Alfred's own scheduler, so there is no OS cron and the
# container runs as the unprivileged `alfred` user (uid 10001).
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
# The Node base image supplies the Node >= 22.19 runtime and npm that the Pi
# dependency needs; Alfred delegates the agent loop to Pi at runtime.
FROM node:22-bookworm-slim

# curl is required by the HEALTHCHECK and smoke tests. cron is no longer needed:
# the e2e suite is owned by Alfred's own scheduler.
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
COPY prompts /opt/alfred/prompts
COPY docker/e2e/config.toml /opt/alfred/config.toml
COPY docker/e2e/dummy_tasks.sh /opt/alfred/e2e/dummy_tasks.sh
COPY docker/e2e/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /opt/alfred/e2e/dummy_tasks.sh /usr/local/bin/entrypoint.sh

# Seed the default config/prompts locations under the container user's HOME.
# Alfred loads $HOME/.alfred/config/config.toml by default.
RUN mkdir -p /home/alfred/.alfred/config/prompts /home/alfred/.alfred/data \
    && cp /opt/alfred/config.toml /home/alfred/.alfred/config/config.toml \
    && cp /opt/alfred/prompts/system.md.example /home/alfred/.alfred/config/prompts/system.md \
    && cp /opt/alfred/prompts/user.md.example /home/alfred/.alfred/config/prompts/user.md \
    && chown -R alfred:alfred /home/alfred

ENV HOME=/home/alfred
ENV RUST_LOG=info
EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:3000/health || exit 1

# Runs as the unprivileged alfred user; the scheduler owns the jobs, so no
# root-owned cron daemon is needed.
USER alfred
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
