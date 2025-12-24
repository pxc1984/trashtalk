# ------------------------------------------------------------
# 1) Dependency build stage (cached)
# ------------------------------------------------------------
FROM rust:1.92.0-bullseye AS deps

RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy only manifest files
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto

# Create a dummy src to force dependency compilation
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs

# Build dependencies only
RUN cargo build --release \
    && rm -rf src


# ------------------------------------------------------------
# 2) Application build stage
# ------------------------------------------------------------
FROM rust:1.92.0-bullseye AS builder

RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Reuse cached target and registry from deps stage
COPY --from=deps /usr/src/app/target /usr/src/app/target
COPY --from=deps /usr/local/cargo /usr/local/cargo

# Copy real source code
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY proto ./proto

# Build the actual binary
RUN cargo build --release --bin trashtalk


# ------------------------------------------------------------
# 3) Runtime stage
# ------------------------------------------------------------
FROM debian:11 AS runner

WORKDIR /

# Copy final binary only
COPY --from=builder /usr/src/app/target/release/trashtalk /trashtalk

EXPOSE 50051
CMD ["/trashtalk"]
