#!/usr/bin/env bash
set -euo pipefail

if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  set -a
  source ".env"
  set +a
fi

echo "[tools] cargo test -- --nocapture $*"
cargo test -- --nocapture "$@"
