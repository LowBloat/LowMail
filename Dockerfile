# syntax=docker/dockerfile:1.7
# LowMail webmail: the web interface and the Rust server in one small image.

ARG RUST_VERSION=1.98
ARG BUN_VERSION=1.4

# ---- web interface ---------------------------------------------------------
FROM oven/bun:${BUN_VERSION} AS web
WORKDIR /src
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile
COPY index.html vite.config.ts svelte.config.js tsconfig.json ./
COPY public public
COPY src src
RUN bun run build

# ---- server ----------------------------------------------------------------
FROM rust:${RUST_VERSION}-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates crates
# The desktop app is a workspace member; only its manifest is needed here.
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml
RUN mkdir -p src-tauri/src && echo "fn main() {}" > src-tauri/src/main.rs && echo "" > src-tauri/src/lib.rs && echo "fn main() {}" > src-tauri/build.rs
COPY --from=web /src/dist dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p lowmail-server \
    && cp target/release/lowmail /usr/local/bin/lowmail

# ---- runtime ---------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/lowmail /usr/local/bin/lowmail
RUN useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin lowmail \
    && mkdir -p /data && chown lowmail:lowmail /data
USER lowmail
ENV LOWMAIL_DATA_DIR=/data \
    LOWMAIL_LISTEN=0.0.0.0:8480
VOLUME ["/data"]
EXPOSE 8480
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["lowmail", "healthcheck"]
ENTRYPOINT ["tini", "--", "lowmail"]
CMD ["serve"]
