# ------------------------------------------------------------
# 1) Dependency build stage (warms the shared cargo cache)
# ------------------------------------------------------------
FROM rust:1.98-alpine AS deps

WORKDIR /usr/src/app

# Musl target needs a C toolchain to link (ring also compiles C/asm code).
# TLS is rustls (pure Rust), so no system OpenSSL is required.
RUN apk add --no-cache build-base

# Copy only manifest files
COPY Cargo.toml Cargo.lock ./

# Create a dummy src to force dependency compilation
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs

# Build dependencies only. Cache mounts keep the compiled artifacts and the
# crate registry across rebuilds, so changing the manifests no longer forces a
# full recompile.
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-target,target=/usr/src/app/target \
    cargo build --release --locked \
    && rm -rf src


# ------------------------------------------------------------
# 2) Application build stage
# ------------------------------------------------------------
FROM rust:1.98-alpine AS builder

WORKDIR /usr/src/app

RUN apk add --no-cache build-base

# Real source code (manifests first so src-only edits reuse the deps cache)
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the binary, reusing the same target cache as the deps stage. Copy the
# result out of the cache mount so it lands in the image layer for the runner.
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-target,target=/usr/src/app/target \
    cargo build --release --bin trashtalk --locked \
    && cp /usr/src/app/target/release/trashtalk /trashtalk


# ------------------------------------------------------------
# 3) Runtime stage
# ------------------------------------------------------------
FROM alpine:3.24 AS runner

WORKDIR /

RUN apk add --no-cache ca-certificates \
    && rm -rf /var/cache/apk/*

# Copy final binary only
COPY --from=builder /trashtalk /trashtalk

CMD ["/trashtalk"]