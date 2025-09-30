# Multi-stage build for RustStrom
FROM rust:1.84-slim as builder

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build release binary with optimizations
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/rust-strom /usr/local/bin/rust-strom

# Copy default config
COPY configs/config.toml /app/config.toml

EXPOSE 80 443 9090

CMD ["rust-strom", "/app/config.toml"]