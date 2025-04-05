# ---- Builder Stage ----
# Use a specific version of the official Rust image
# Use the latest stable slim image to ensure Cargo compatibility
FROM rust:slim AS builder

# Add the musl target
RUN rustup target add x86_64-unknown-linux-musl

# Create a non-root user and group for the build process
RUN groupadd --gid 1000 builder && \
    useradd --uid 1000 --gid 1000 -m builder

# Install build dependencies needed for musl target
# - musl-tools: Required for linking against musl libc
# - ca-certificates: Needed to copy into the final scratch image for HTTPS support
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* # Clean up apt lists

WORKDIR /app

# Change ownership to the builder user
RUN chown builder:builder /app
USER builder

# Copy manifests first to leverage Docker layer caching
COPY --chown=builder:builder Cargo.toml Cargo.lock ./

# Build dependencies separately to cache them
# Create dummy src files to allow dependency-only build
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn lib() {}" > src/lib.rs
# Build only dependencies for the musl target
RUN cargo build --release --locked --target x86_64-unknown-linux-musl
# Remove dummy source files after building dependencies
RUN rm -rf src

# Copy the actual source code
COPY --chown=builder:builder src ./src

# Build the application statically for the musl target
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

# Strip the binary to further reduce size
RUN strip target/x86_64-unknown-linux-musl/release/mail-laser

# ---- Final Stage ----
# Use scratch for the absolute minimal image
FROM scratch

# Copy CA certificates from the builder stage for HTTPS support
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

WORKDIR /app

# Copy only the statically compiled and stripped binary from the builder stage
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/mail-laser .

# Set the entrypoint to run the application
# The binary is executed directly as PID 1
ENTRYPOINT ["/app/mail-laser"]

# Optional: Default arguments can be added via CMD
# CMD ["--some-default-arg"]