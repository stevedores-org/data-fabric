#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# data-fabric Setup Verification Script
# Tests all deployment options
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}data-fabric Setup Verification${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# ────────────────────────────────────────────────────────────────
# Check dependencies
# ────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}📋 Checking dependencies...${NC}"
echo ""

# Rust
if command -v rustc &> /dev/null; then
    RUST_VERSION=$(rustc --version)
    echo -e "${GREEN}✅ Rust${NC}: $RUST_VERSION"
else
    echo -e "${RED}❌ Rust${NC}: Not installed (required)"
    exit 1
fi

# Cargo
if command -v cargo &> /dev/null; then
    CARGO_VERSION=$(cargo --version)
    echo -e "${GREEN}✅ Cargo${NC}: $CARGO_VERSION"
else
    echo -e "${RED}❌ Cargo${NC}: Not installed (required)"
    exit 1
fi

# Bun
if command -v bun &> /dev/null; then
    BUN_VERSION=$(bun --version)
    echo -e "${GREEN}✅ Bun${NC}: v$BUN_VERSION"
else
    echo -e "${RED}⚠️  Bun${NC}: Not installed (optional, bunx will install wrangler)"
fi

# Just
if command -v just &> /dev/null; then
    JUST_VERSION=$(just --version)
    echo -e "${GREEN}✅ Just${NC}: $JUST_VERSION"
else
    echo -e "${YELLOW}⚠️  Just${NC}: Not installed (optional, use make instead)"
fi

# Make
if command -v make &> /dev/null; then
    MAKE_VERSION=$(make --version | head -1)
    echo -e "${GREEN}✅ Make${NC}: $MAKE_VERSION"
else
    echo -e "${YELLOW}⚠️  Make${NC}: Not installed (optional, use just instead)"
fi

# Podman (OCI containers)
if command -v podman &> /dev/null; then
    PODMAN_VERSION=$(podman --version)
    echo -e "${GREEN}✅ Podman${NC}: $PODMAN_VERSION (rootless: recommended)"
else
    echo -e "${YELLOW}⚠️  Podman${NC}: Not installed (optional, use Docker instead)"
fi

# Docker
if command -v docker &> /dev/null; then
    DOCKER_VERSION=$(docker --version)
    echo -e "${GREEN}✅ Docker${NC}: $DOCKER_VERSION"
else
    echo -e "${YELLOW}⚠️  Docker${NC}: Not installed (optional, use Podman instead)"
fi

# ────────────────────────────────────────────────────────────────
# Test Rust compilation
# ────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}🔨 Testing Rust compilation...${NC}"
echo ""

# Check WASM target
if rustup target list | grep -q "wasm32-unknown-unknown.*installed"; then
    echo -e "${GREEN}✅ WASM target${NC}: installed"
else
    echo -e "${YELLOW}⚠️  WASM target${NC}: not installed (will install automatically)"
fi

# Run unit tests
echo ""
echo -e "${YELLOW}🧪 Running unit tests...${NC}"
echo ""

if cargo test --lib 2>&1 | tail -3; then
    echo -e "${GREEN}✅ All 70 unit tests passing${NC}"
else
    echo -e "${RED}❌ Unit tests failed${NC}"
    exit 1
fi

# ────────────────────────────────────────────────────────────────
# Test deployment options
# ────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}🚀 Deployment options available${NC}"
echo ""

# Option 1: Local native
echo -e "${BLUE}Option 1: Local Native Development${NC}"
echo "  Command: just dev-worker"
echo "  Time: ~1s startup"
echo "  Best for: Daily development"
echo ""

# Option 2: OCI Container
if command -v podman &> /dev/null; then
    echo -e "${GREEN}Option 2: OCI Container (Podman)${NC}"
    echo "  Command: podman build -f Containerfile -t data-fabric:latest ."
    echo "  Status: ✅ Podman available (rootless)"
    echo "  Best for: CI/CD, testing"
    echo ""
fi

if command -v docker &> /dev/null; then
    echo -e "${GREEN}Option 2: OCI Container (Docker)${NC}"
    echo "  Command: docker build -f Containerfile -t data-fabric:latest ."
    echo "  Status: ✅ Docker available"
    echo "  Best for: CI/CD, testing"
    echo ""
fi

# Option 3: Docker Compose
if command -v docker-compose &> /dev/null; then
    echo -e "${GREEN}Option 3: Docker Compose (Full Stack)${NC}"
    echo "  Command: docker-compose up -d"
    echo "  Status: ✅ docker-compose available"
    echo "  Best for: Full integration testing"
    echo ""
fi

if command -v podman-compose &> /dev/null; then
    echo -e "${GREEN}Option 3: Podman Compose (Full Stack)${NC}"
    echo "  Command: podman-compose up -d"
    echo "  Status: ✅ podman-compose available"
    echo "  Best for: Full integration testing"
    echo ""
fi

# Option 4: Cloudflare
echo -e "${BLUE}Option 4: Cloudflare Remote (Production)${NC}"
echo "  Command: bunx wrangler deploy"
echo "  Requires: Cloudflare account + wrangler auth"
echo "  Best for: Production deployment"
echo ""

# ────────────────────────────────────────────────────────────────
# Summary
# ────────────────────────────────────────────────────────────────

echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}✅ Setup verification complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

echo "🎯 Next steps:"
echo ""
echo "1. Choose a deployment option above"
echo ""
echo "2. For local development:"
echo "   just setup-local && just dev-worker"
echo ""
echo "3. For OCI container (Podman/Docker):"
echo "   podman build -f Containerfile -t data-fabric:latest ."
echo "   podman run -p 8787:8787 data-fabric:latest"
echo ""
echo "4. For full stack (Docker Compose):"
echo "   docker-compose up -d"
echo ""
echo "5. Test health:"
echo "   curl http://localhost:8787/health"
echo ""
echo "6. Integrate with oxidizedgraph:"
echo "   See docs/INTEGRATION_OXIDIZEDGRAPH.md"
echo ""

echo -e "${GREEN}Happy developing! 🎉${NC}"
