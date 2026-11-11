#!/usr/bin/env bash
# Full regression run: Rust tests, lints, and the Playwright GUI tests.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo test"
cargo test

echo "== cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "== playwright (GUI end-to-end)"
cargo build
cd e2e
if command -v playwright >/dev/null 2>&1; then
  playwright test "$@"
else
  npx playwright test "$@"
fi
