# syntax=docker/dockerfile:1
# Build from the repo root: docker build -f backend/Dockerfile -t laseask-backend .
# Needs BuildKit (the cache mounts below require it) — a plain `docker build` uses it
# automatically once the buildx plugin is installed; see README.md's Docker deploy section.
#
# Build and runtime stages are pinned to the SAME Debian release (bookworm) on purpose: the
# unpinned `rust:1-slim` tag tracks whatever Debian release is current (trixie as of writing,
# glibc 2.38+), which is newer than `debian:bookworm-slim`'s glibc (2.36) below — a binary built
# in a newer base fails to run in an older one ("GLIBC_2.38 not found"). Keep these two lines on
# the same Debian codename if you ever change one of them.
FROM rust:1-slim-bookworm AS build
WORKDIR /app
COPY backend/ ./backend/
WORKDIR /app/backend
# Cache mounts persist across builds (not into the image layer itself), so repeat builds only
# recompile what changed instead of every dependency from scratch every time.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/backend/target \
    cargo build --release && cp target/release/laseask-backend /app/laseask-backend

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/laseask-backend ./laseask-backend
COPY frontend ./frontend
ENV LASEASK_CONFIG=/app/config.toml
EXPOSE 8787
CMD ["./laseask-backend"]
