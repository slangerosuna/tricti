#!/usr/bin/env bash
set -euo pipefail

if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  set -a
  source ".env"
  set +a
fi

echo "[tools] cargo fmt"
cargo fmt

if command -v cargo &>/dev/null; then
  if cargo clippy -V &>/dev/null; then
    echo "[tools] cargo clippy -- -D warnings"
    cargo clippy -- -D warnings
  fi
fi
