# ---- Builder Stage ----
# Multi-arch: TARGETARCH is set by Buildx (amd64 / arm64).
FROM rust:1.95-slim AS builder

ARG TARGETARCH

# Map Docker arch → Rust musl target
RUN case "$TARGETARCH" in \
      amd64) echo "x86_64-unknown-linux-musl" > /rust-target ;; \
      arm64) echo "aarch64-unknown-linux-musl" > /rust-target ;; \
      *) echo "Unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && rustup target add "$(cat /rust-target)"

# Create a non-root user and group for the build process
RUN groupadd --gid 1000 builder && \
    useradd --uid 1000 --gid 1000 -m builder

# Install build dependencies needed for musl target
# - musl-tools: Required for linking against musl libc
# - ca-certificates: Needed to copy into the final scratch image for HTTPS support
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Change ownership to the builder user
RUN chown builder:builder /app /rust-target
USER builder

# Copy manifests first to leverage Docker layer caching
COPY --chown=builder:builder Cargo.toml Cargo.lock ./

# Build dependencies separately to cache them
# Create dummy src files to allow dependency-only build
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn lib() {}" > src/lib.rs
# Build only dependencies for the musl target
RUN RUST_TARGET="$(cat /rust-target)" && \
    cargo build --release --locked --target "$RUST_TARGET"
# Remove dummy source files after building dependencies
RUN rm -rf src

# Copy the actual source code
COPY --chown=builder:builder src ./src

# Invalidate just the main crate's cached artifacts so the second cargo
# build actually recompiles against the real src. Without this, Docker COPY
# preserves host mtimes (older than the dummy fingerprint from the dep-only
# build above), so cargo decides nothing changed and we ship the dummy.
# The dep cache is preserved — only MailLaser's artifacts are dropped.
RUN RUST_TARGET="$(cat /rust-target)" && \
    cargo clean --release --target "$RUST_TARGET" -p MailLaser && \
    cargo build --release --locked --target "$RUST_TARGET" && \
    cp "/app/target/${RUST_TARGET}/release/mail_laser" /app/mail_laser

# ---- Final Stage ----
# Use scratch for the absolute minimal image
FROM scratch

# Copy CA certificates from the builder stage for HTTPS support
# The builder stage installed them via apt-get
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

WORKDIR /app

# Copy only the statically compiled binary from the builder stage
COPY --from=builder /app/mail_laser .

# The COPY command preserves execute permissions

# Run the application using CMD
CMD ["/app/mail_laser"]
