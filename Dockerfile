# syntax=docker/dockerfile:1.7
#
# Trading Center images (one Dockerfile, two targets):
#   server : tc-server (api | engine roles) + tc CLI, distroless runtime, uid 10001
#   web    : Caddy serving the Next.js static export + reverse proxy to the API
#
# Build:  docker build --target server -t trading-center/server .
#         docker build --target web    -t trading-center/web .
# Base images are pinned by major version; pin by digest for production (docs/deployment.md).

############################
# Rust build stage
############################
FROM rust:1-bookworm AS rust-builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# Context is filtered by .dockerignore (no apps/desktop, docs, secrets, rust-toolchain.toml).
COPY . .
# BuildKit cache mounts keep the registry and target dir between builds (no cargo-chef needed).
# Optional `build_ca` secret: extra CA for builds behind a TLS-intercepting proxy (unused in normal builds).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    --mount=type=secret,id=build_ca,required=false \
    if [ -s /run/secrets/build_ca ]; then export CARGO_HTTP_CAINFO=/run/secrets/build_ca; fi \
 && cargo build --release --locked -p tc-server -p tc-cli \
 && mkdir -p /out \
 && cp target/release/tc-server target/release/tc /out/

############################
# Server runtime (api + engine)
############################
# distroless/cc-debian12 = glibc, libgcc, libssl3, ca-certificates; no shell, no package manager.
FROM gcr.io/distroless/cc-debian12 AS server
COPY --from=rust-builder /out/tc-server /usr/local/bin/tc-server
COPY --from=rust-builder /out/tc /usr/local/bin/tc
USER 10001:10001
ENV TC_LOG=info
EXPOSE 8080
ENTRYPOINT []
CMD ["/usr/local/bin/tc-server", "--role", "api"]

############################
# Web build stage (Next.js static export)
############################
FROM node:22-alpine AS web-builder
WORKDIR /web
COPY apps/web/package.json apps/web/package-lock.json ./
RUN --mount=type=secret,id=build_ca,required=false \
    if [ -s /run/secrets/build_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/build_ca; fi \
 && npm ci --no-audit --no-fund
COPY apps/web ./
ENV NEXT_TELEMETRY_DISABLED=1
RUN npm run build \
 && test -f out/index.html
# CSP: the web build emits out/_csp/script-hashes.txt (one 'sha256-…' per line). The build
# fails without it, so the CSP never silently falls back to 'unsafe-inline'.
RUN set -eu; \
    test -s out/_csp/script-hashes.txt; \
    SCRIPT_SRC="'self' $(tr '\n' ' ' < out/_csp/script-hashes.txt)"; \
    printf 'header Content-Security-Policy "default-src '"'"'self'"'"'; script-src %s; style-src '"'"'self'"'"' '"'"'unsafe-inline'"'"'; img-src '"'"'self'"'"' data: blob:; font-src '"'"'self'"'"'; connect-src '"'"'self'"'"' wss://{host}; manifest-src '"'"'self'"'"'; worker-src '"'"'self'"'"'; object-src '"'"'none'"'"'; base-uri '"'"'none'"'"'; form-action '"'"'self'"'"'; frame-ancestors '"'"'none'"'"'; upgrade-insecure-requests"\n' "$SCRIPT_SRC" > /web/csp.caddy; \
    cat /web/csp.caddy

############################
# Web runtime (Caddy)
############################
FROM caddy:2-alpine AS web
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=web-builder /web/csp.caddy /etc/caddy/csp.caddy
COPY --from=web-builder /web/out /srv
RUN TC_DOMAIN=localhost TC_ACME_EMAIL=ops@example.com caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
