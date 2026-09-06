# ------------------------------------------------------------
# 1) Dependency build stage (cached)
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

# Build dependencies only
RUN cargo build --release \
    && rm -rf src


# ------------------------------------------------------------
# 2) Application build stage
# ------------------------------------------------------------
FROM rust:1.98-alpine AS builder

WORKDIR /usr/src/app

RUN apk add --no-cache build-base

# Reuse cached target and registry from deps stage
COPY --from=deps /usr/src/app/target /usr/src/app/target
COPY --from=deps /usr/local/cargo /usr/local/cargo

# Copy real source code
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the actual binary
RUN cargo build --release --bin trashtalk


# ------------------------------------------------------------
# 3) Runtime stage
# ------------------------------------------------------------
FROM alpine:3.24 AS runner

WORKDIR /

RUN apk add --no-cache ca-certificates \
    && rm -rf /var/cache/apk/*

# Copy final binary only
COPY --from=builder /usr/src/app/target/release/trashtalk /trashtalk

CMD ["/trashtalk"]