# Update local main branch
new:
    git switch main && git pull --ff-only

# === Build ===

# Build all workspace members
build:
    cargo build --workspace

# Release build
build-release:
    cargo build --workspace --release

# === Run ===

# Run the app
run:
    cargo run -p wellfeather

# Run the app in release mode
run-release:
    cargo run -p wellfeather --release

# === Code Quality ===

# Format code
fmt:
    cargo fmt --all

# Verify formatting (no changes)
fmt-check:
    cargo fmt --all -- --check

# Run clippy (-D warnings)
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run clippy with auto-fix
clippy-fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty

# Quick pre-commit gate: fmt-check + clippy
check: fmt-check clippy

# === Testing ===

# Run all tests (unit + integration) for all crates
test:
    cargo test --workspace

# Run unit tests: all crates / specific crate / specific test in crate
# Examples:
#   just unit-test                        # All unit tests
#   just unit-test wf-db                  # All unit tests in wf-db
#   just unit-test wf-db connect_should_  # Specific test in wf-db
unit-test crate="" test="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "{{crate}}" ]; then
        cargo test --workspace --lib
    elif [ -z "{{test}}" ]; then
        cargo test -p {{crate}} --lib
    else
        cargo test -p {{crate}} --lib {{test}}
    fi

# Run integration tests: all crates / specific crate / specific test in crate
# Examples:
#   just integration-test                 # All integration tests
#   just integration-test wf-db           # All integration tests in wf-db
#   just integration-test wf-db test_pg_  # Specific test in wf-db
integration-test crate="" test="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "{{crate}}" ]; then
        cargo test --workspace --tests
    elif [ -z "{{test}}" ]; then
        cargo test -p {{crate}} --tests
    else
        cargo test -p {{crate}} --tests {{test}}
    fi

# Run tests sequentially (useful when DB connections are limited)
test-seq:
    cargo test --workspace -- --test-threads=1

# === Misc ===

# Full CI pipeline: fmt-check + clippy + build + test
ci: fmt-check clippy build test

# Clean build artifacts
clean:
    cargo clean
