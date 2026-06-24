# Development commands for Almagest
# "Dashboards as files, not services."

# List available recipes
default:
    @just --list

# Install frontend dependencies
frontend-install:
    cd frontend && npm install

# Build the frontend bundle into frontend/dist (embedded by the server)
frontend:
    cd frontend && npm run build

# Type-check the frontend
frontend-check:
    cd frontend && npm run check

# Build the bundle + binary, then run the Playwright end-to-end smoke suite
e2e: frontend
    cargo build -p almagest-cli
    cd frontend && npm run e2e

# Build backend and frontend
build: frontend
    cargo build --workspace

# Run all tests
test:
    cargo test --workspace

# Check formatting, lints, and tests
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Format all code
fmt:
    cargo fmt --all

# Lint with clippy
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run the CLI in dev mode (pass args after --)
dev *ARGS:
    cargo run --bin almagest -- {{ARGS}}

# Build an optimized release binary with embedded frontend
release: frontend
    cargo build --release --bin almagest

# Report release binary size
release-size: release
    ls -lh target/release/almagest

# cargo-deny advisory + license check
deny:
    cargo deny check

# Package a release archive for the current platform
package: release
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(cargo metadata --format-version=1 --no-deps | python3 -c "import sys,json; print([p for p in json.load(sys.stdin)['packages'] if p['name']=='almagest-cli'][0]['version'])")
    TARGET=$(rustc -vV | awk '/^host:/ { print $2 }')
    ARCHIVE="almagest-v${VERSION}-${TARGET}.tar.gz"
    tar -czf "$ARCHIVE" -C target/release almagest
    shasum -a 256 "$ARCHIVE"
    echo "Created $ARCHIVE"
    ls -lh "$ARCHIVE"
