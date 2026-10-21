#!/usr/bin/env bash
# Run editor-core shaping checks without hosted CI minutes.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is missing. Install a Rust toolchain first." >&2
  exit 2
fi

if [[ "${1:-core}" != "core" && "${1:-core}" != "full" ]]; then
  echo "Usage: scripts/verify_local.sh [core|full]" >&2
  exit 2
fi

echo "Checking Rust formatting..."
cargo fmt --all -- --check

echo "Testing vector shaping..."
cargo test --locked -p tracedraw-core shaping::tests

echo "Checking core lints..."
cargo clippy --locked -p tracedraw-core -- -D warnings

if [[ "${1:-core}" == "full" ]]; then
  echo "Running all workspace tests..."
  cargo test --workspace --locked
fi

echo "Local checks passed."
