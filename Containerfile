# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# data-fabric Worker — OCI Container Image (Podman/Docker compatible)
# Uses OCI standards for maximum compatibility
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# ────────────────────────────────────────────────────────────────
# Stage 1: Build Rust → WASM
# ────────────────────────────────────────────────────────────────
FROM rust:1.81-bookworm AS builder

WORKDIR /build

# Install WASM target and build tools
RUN rustup target add wasm32-unknown-unknown && \
    cargo install worker-build wasm-pack wasm-bindgen-cli

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/

# Build WASM artifact with optimization
RUN cargo build --target wasm32-unknown-unknown --release && \
    worker-build --release

# Verify build succeeded
RUN ls -lh build/worker/shim.mjs && \
    echo "✅ WASM build complete"

# ────────────────────────────────────────────────────────────────
# Stage 2: Runtime (Node.js-based, OCI-compliant)
# ────────────────────────────────────────────────────────────────
FROM oven/bun:1.2-bookworm

# Add container labels (OCI standard)
LABEL org.opencontainers.image.title="data-fabric"
LABEL org.opencontainers.image.description="Cloudflare-native data fabric for autonomous AI agent orchestration"
LABEL org.opencontainers.image.url="https://github.com/stevedores-org/data-fabric"
LABEL org.opencontainers.image.source="https://github.com/stevedores-org/data-fabric"
LABEL org.opencontainers.image.version="0.1.0"

WORKDIR /app

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    sqlite3 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Copy built WASM artifacts from builder
COPY --from=builder /build/build ./build/
COPY --from=builder /build/Cargo.toml ./
COPY --from=builder /build/migrations ./migrations/

# Create non-root user and data directories with proper permissions
RUN groupadd -r appuser && useradd -r -g appuser -u 1000 appuser && \
    mkdir -p /data .wrangler/state/v3/d1 && \
    chown -R 1000:1000 /app /data

# Copy configuration
COPY wrangler.toml ./

# Set environment variables
ENV NODE_ENV=production
ENV RUST_LOG=info

# OCI Standard: Expose ports
EXPOSE 8787

# OCI Standard: Health check
HEALTHCHECK --interval=10s --timeout=5s --retries=3 --start-period=20s \
    CMD curl -f http://localhost:8787/health || exit 1

# OCI Standard: User specification (non-root for security)
USER 1000:1000

# OCI Standard: Entry point
ENTRYPOINT ["/bin/sh", "-c"]

# Default command: start development server
CMD ["bunx wrangler dev --local --host 0.0.0.0 --port 8787"]

# Build instructions:
# Docker:  docker build -f Containerfile -t data-fabric:latest .
# Podman:  podman build -f Containerfile -t data-fabric:latest .
# Buildah: buildah bud -f Containerfile -t data-fabric:latest .
