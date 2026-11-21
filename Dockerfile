# Multi-stage build for RustStrom

# ---- Build stage ----
FROM rust:1-slim-bookworm AS builder

# perl + make: vendored OpenSSL (openssl-sys "vendored" feature)
# gcc: mimalloc and other C dependencies
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config perl make gcc \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Build dependencies first with a stub main.rs so this layer is cached
# until Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

# Build the real binary (touch so cargo does not reuse the stub build)
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

# ---- Runtime stage ----
FROM debian:bookworm-slim

# ca-certificates: ACME / outbound TLS; curl: container healthcheck
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/rust-strom /usr/local/bin/rust-strom

# Default config; override by mounting your own file at /app/config.toml
COPY configs/config.toml /app/config.toml

EXPOSE 80 443 9090

STOPSIGNAL SIGTERM

CMD ["rust-strom", "--config", "/app/config.toml"]
