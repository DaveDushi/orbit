# syntax=docker/dockerfile:1.7
ARG ORBIT_BUILD_REVISION=container
FROM oven/bun:1.4.4-alpine AS web
WORKDIR /src
COPY package.json bun.lock bunfig.toml ./
COPY apps/web/package.json apps/web/
COPY apps/mobile/package.json apps/mobile/
COPY packages/api/package.json packages/api/
COPY packages/editor/package.json packages/editor/
RUN bun install --frozen-lockfile --filter web
COPY packages/api/ packages/api/
COPY packages/editor/ packages/editor/
COPY apps/web/ apps/web/
WORKDIR /src/apps/web
ARG ORBIT_BUILD_REVISION
RUN ORBIT_BUILD_REVISION=${ORBIT_BUILD_REVISION} bun run build

# rustc on Alpine uses the slow musl allocator and builds about 2x slower. Build on Debian and target musl,
# so the binary stays static.
FROM rust:1.97.1-slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup target add "$(uname -m)-unknown-linux-musl" \
    && cargo install cargo-chef --locked --version 0.1.78

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY apps/server/ apps/server/
RUN cargo chef prepare --recipe-path recipe.json

# The dependency layer changes only when the recipe (manifests and Cargo.lock) changes, so the layer cache
# keeps it between CI runs.
FROM chef AS server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --locked --release -p orbit-server --target "$(uname -m)-unknown-linux-musl" \
    --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY apps/server/ apps/server/
COPY --from=web /src/apps/web/dist/ apps/web/dist/
ARG ORBIT_BUILD_REVISION
RUN target="$(uname -m)-unknown-linux-musl" \
    && ORBIT_BUILD_REVISION=${ORBIT_BUILD_REVISION} \
    cargo build --locked --release -p orbit-server --target "$target" \
    && cp "target/$target/release/orbit" /orbit

FROM alpine:3.23
RUN apk add --no-cache bash ca-certificates tzdata \
    && mkdir -p /var/lib/orbit /var/backups/orbit \
    && chown -R 65532:65532 /var/lib/orbit /var/backups/orbit
COPY --from=server /orbit /orbit
# The image trusts forwarding headers from every address: it expects to run behind a proxy that is the only
# way in. Publish port 8080 directly, or share the network with untrusted containers, and clients can fake
# their IP address and https. Then set ORBIT_TRUSTED_PROXIES to the proxy network.
ENV ORBIT_ENV=production \
    ORBIT_BIND=0.0.0.0 \
    ORBIT_TRUSTED_PROXIES=0.0.0.0/0,::/0 \
    ORBIT_DATABASE_PATH=/var/lib/orbit/orbit.sqlite \
    ORBIT_ATTACHMENTS_PATH=/var/lib/orbit/attachments \
    ORBIT_BACKUPS_PATH=/var/backups/orbit
USER 65532:65532
VOLUME ["/var/lib/orbit", "/var/backups/orbit"]
EXPOSE 8080
HEALTHCHECK --interval=5s --timeout=3s --start-period=5s --retries=10 CMD ["/orbit", "healthcheck"]
ENTRYPOINT ["/orbit"]
CMD ["serve"]
