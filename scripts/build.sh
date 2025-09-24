#!/usr/bin/env bash
set -euo pipefail

# Load local environment overrides if present (e.g., LLVM paths)
if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  set -a
  source ".env"
  set +a
fi

echo "[tools] cargo build"
cargo build
