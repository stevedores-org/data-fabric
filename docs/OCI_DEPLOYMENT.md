# Data Fabric — OCI Container Deployment Guide

> **Status**: OCI-standard compliant (Podman/Docker/Buildah compatible)

Deploy data-fabric using OCI (Open Container Initiative) standards for maximum portability and security.

## Overview

Instead of Dockerfile, we use **Containerfile** (OCI standard):
- ✅ Works with Podman (rootless, more secure)
- ✅ Works with Docker (widely available)
- ✅ Works with Buildah (lower-level building)
- ✅ Works with any OCI-compliant engine
- ✅ Better for CI/CD pipelines
- ✅ Smaller, more efficient

---

## Quick Start

### Using Podman (Recommended - Rootless)

```bash
cd /path/to/data-fabric

# Build image
podman build -f Containerfile -t data-fabric:latest .

# Run container (rootless, port-forward required)
podman run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest

# Check health
curl http://localhost:8787/health

# View logs
podman logs -f data-fabric

# Stop container
podman stop data-fabric
podman rm data-fabric
```

### Using Docker

```bash
cd /path/to/data-fabric

# Build image
docker build -f Containerfile -t data-fabric:latest .

# Run container
docker run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest

# Check health
curl http://localhost:8787/health
```

### Using Buildah (Advanced)

```bash
cd /path/to/data-fabric

# Build image
buildah bud -f Containerfile -t data-fabric:latest .

# Create container from image
buildah from --name data-fabric-run data-fabric:latest

# Run commands in container
buildah run data-fabric-run /bin/sh -c "bunx wrangler dev --local"

# Commit to image
buildah commit data-fabric-run data-fabric:latest
```

---

## Containerfile Explained

The `Containerfile` is functionally equivalent to `Dockerfile` but follows OCI standards:

```dockerfile
# Stage 1: Multi-stage build (efficient layer caching)
FROM rust:1.81-bookworm AS builder
# Build artifacts in separate stage

# Stage 2: Runtime (lean, production-ready)
FROM oven/bun:1.2-bookworm
# Only runtime dependencies, not build tools

# OCI Standard Labels
LABEL org.opencontainers.image.title="data-fabric"
LABEL org.opencontainers.image.description="..."

# Expose ports (OCI standard)
EXPOSE 8787

# Health check (OCI standard)
HEALTHCHECK --interval=10s ...

# Entry point (OCI standard)
ENTRYPOINT ["/bin/sh", "-c"]

# Default command
CMD ["bunx wrangler dev --local --host 0.0.0.0 --port 8787"]
```

---

## Build Options

### Option 1: Local Build & Test

```bash
# Build
podman build -f Containerfile -t data-fabric:local .

# Test build
podman run --rm data-fabric:local --help

# Inspect image
podman inspect data-fabric:local | jq '.[] | .Config'
```

### Option 2: Build with Buildah (Lower-level)

Buildah gives you more control:

```bash
# Create working container
ctr=$(buildah from rust:1.81-bookworm)

# Install tools in container
buildah run $ctr rustup target add wasm32-unknown-unknown
buildah run $ctr cargo install worker-build

# Copy files
buildah copy $ctr Cargo.toml /build/
buildah copy $ctr src /build/src/

# Build WASM
buildah run $ctr -w /build cargo build --target wasm32-unknown-unknown --release

# Commit to image
buildah commit $ctr data-fabric:buildah
```

### Option 3: CI/CD Pipeline (GitHub Actions)

```yaml
name: Build OCI Image

on: [push]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build image (Podman)
        run: |
          podman build \
            -f Containerfile \
            -t ghcr.io/stevedores-org/data-fabric:${{ github.sha }} \
            .

      - name: Push to registry
        run: |
          podman login ghcr.io -u ${{ github.actor }} -p ${{ secrets.GITHUB_TOKEN }}
          podman push ghcr.io/stevedores-org/data-fabric:${{ github.sha }}
```

---

## Docker Compose with OCI Containers

### Compose File

The `docker-compose.yml` automatically uses `Containerfile`:

```yaml
services:
  worker:
    build:
      context: .
      dockerfile: Containerfile  # OCI standard, not Dockerfile
    ports:
      - "8787:8787"
    volumes:
      - data-fabric-db:/data
```

### Usage

```bash
# Docker (OCI containers via Docker)
docker-compose up -d

# Podman (OCI containers via Podman)
podman-compose up -d

# View logs
docker-compose logs -f worker

# Stop
docker-compose down
```

---

## Security Considerations

### Rootless Containers (Podman)

Podman runs rootless by default - more secure:

```bash
# Check podman configuration
podman info | grep "rootless"

# Run rootless (no sudo needed)
podman build -f Containerfile -t data-fabric:latest .
podman run -d --name data-fabric -p 8787:8787 data-fabric:latest
```

### Image Signing (Optional)

Sign and verify OCI images:

```bash
# Generate key (one-time)
gpg --generate-key

# Sign image
podman image sign oci://data-fabric:latest

# Verify signature
podman image sign --verify oci://data-fabric:latest
```

### CVE Scanning

Scan for vulnerabilities:

```bash
# Using grype (multi-tool support)
grype ghcr.io/stevedores-org/data-fabric:latest

# Using podman (built-in)
podman run --rm -v /var/run/podman/podman.sock:/run/podman/podman.sock \
  quay.io/containers/image-scanning podman-scan
```

---

## Publishing to Registry

### Push to GitHub Container Registry (GHCR)

```bash
# Tag image
podman tag data-fabric:latest \
  ghcr.io/stevedores-org/data-fabric:1.0.0

# Login
podman login ghcr.io \
  -u stevenirvin \
  -p $GITHUB_TOKEN

# Push
podman push ghcr.io/stevedores-org/data-fabric:1.0.0

# Verify
podman pull ghcr.io/stevedores-org/data-fabric:1.0.0
```

### Push to Docker Hub

```bash
# Tag
podman tag data-fabric:latest \
  docker.io/stevedores/data-fabric:1.0.0

# Login
podman login docker.io

# Push
podman push docker.io/stevedores/data-fabric:1.0.0
```

---

## Troubleshooting

### Container Fails to Start

```bash
# Check logs
podman logs data-fabric

# If "Port already in use":
podman run -p 9000:8787 data-fabric:latest  # Use 9000 instead

# If "Module not found":
podman run -it --entrypoint /bin/sh data-fabric:latest
# Then: ls -la build/worker/
```

### Build Fails

```bash
# Clean build (no cache)
podman build --no-cache -f Containerfile -t data-fabric:latest .

# Verbose output
podman build --log-level debug -f Containerfile .

# Check Containerfile syntax
podman build --dry-run -f Containerfile .
```

### OCI Image Spec Compliance

```bash
# Inspect OCI image configuration
podman inspect data-fabric:latest | jq '.[] | .Config'

# Validate image against OCI spec
oci-image-spec validate data-fabric:latest
```

---

## Comparison: Docker vs Podman vs Buildah

| Feature | Docker | Podman | Buildah |
|---------|--------|--------|---------|
| **Rootless** | ❌ (requires daemon) | ✅ (by default) | ✅ (by default) |
| **Daemon** | ✅ (required) | ❌ (daemonless) | ❌ (daemonless) |
| **OCI Standard** | ✅ (containers) | ✅ (full) | ✅ (full) |
| **Security** | ⚠️ (root required) | ✅ (more secure) | ✅ (most secure) |
| **Learning Curve** | Easy | Easy | Moderate |
| **CI/CD Integration** | ✅ | ✅ | ✅ (preferred) |

**Recommendation for development**: Podman (secure, simple)
**Recommendation for CI/CD**: Buildah (efficient, flexible)

---

## Advanced: Custom OCI Runtime

Use alternative OCI runtimes for specific scenarios:

```bash
# Use crun (faster)
podman run --runtime crun data-fabric:latest

# Use kata (VM isolation)
podman run --runtime kata data-fabric:latest

# Use runc (default, most stable)
podman run --runtime runc data-fabric:latest
```

---

## Next Steps

1. **Build locally**: `podman build -f Containerfile -t data-fabric:latest .`
2. **Run locally**: `podman run -p 8787:8787 data-fabric:latest`
3. **Test with oxidizedgraph**: Point to `http://localhost:8787`
4. **Push to registry**: `podman push ghcr.io/stevedores-org/data-fabric:latest`

---

## Resources

- [OCI Image Spec](https://github.com/opencontainers/image-spec)
- [Podman Documentation](https://podman.io/docs/)
- [Buildah Documentation](https://buildah.io/)
- [Docker to Podman Migration](https://podman.io/docs/tutorials/migrate-docker-to-podman)
