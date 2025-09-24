#!/usr/bin/env bash
set -euo pipefail

if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  set -a
  source ".env"
  set +a
fi

# Always clean temporary test artifacts on exit
trap 'echo "[tools] cleaning tmp artifacts"; rm -f tests/tmp_*.o tests/tmp_*.out || true' EXIT

echo "[tools] cargo test -- --nocapture $*"
cargo test -- --nocapture "$@"
