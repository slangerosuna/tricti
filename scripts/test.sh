#!/usr/bin/env bash
set -euo pipefail

set +e

if [[ -f ".env" ]]; then
  set -a
  source ".env"
  set +a
fi

export RUSTFLAGS="-Awarnings"
mkdir -p tmp
trap 'echo "[tools] cleaning tmp artifacts"; rm -f tmp/output tests/tmp_*.o tests/tmp_*.out || true' EXIT

# Argument parsing
RUN_TRI=false
RUN_CARGO=false
TRI_ARGS=()
CARGO_ARGS=()

NEITHER_PASSED=true

while [[ $# -gt 0 ]]; do
  case "$1" in
    -t)
      NEITHER_PASSED=false
      RUN_TRI=true
      shift
      # collect everything until next -c or end
      while [[ $# -gt 0 && "$1" != "-c" && "$1" != "-t" ]]; do
        TRI_ARGS+=("$1")
        shift
      done
      ;;
    -c)
      NEITHER_PASSED=false
      RUN_CARGO=true
      shift
      # collect everything until next -t or end
      while [[ $# -gt 0 && "$1" != "-c" && "$1" != "-t" ]]; do
        CARGO_ARGS+=("$1")
        shift
      done
      ;;
    *)
      # treat as positional args if no flag
      TRI_ARGS+=("$1")
      CARGO_ARGS+=("$1")
      shift
      ;;
  esac
done

if $NEITHER_PASSED; then
  RUN_TRI=true
  RUN_CARGO=true
fi

if $RUN_CARGO; then
  echo "[tools] running cargo tests"
  timeout 30s cargo test -- --nocapture "${CARGO_ARGS[@]}" &>tmp/test_output
  rc=$?
  if [[ $rc -ne 0 ]]; then
    echo "[tools] cargo tests failed (code $rc)"
    cat tmp/test_output
    exit 1
  fi
  echo "[tools] all cargo tests passed"
fi

if $RUN_TRI; then
  echo "[tools] building TriCTI executable"
  if ! cargo build &>tmp/test_output; then
    cat tmp/test_output
    exit 1
  fi
  echo "[tools] cargo build passed"

  echo "[tools] running TriCTI native tests"

  # No args → run all
  if [[ ${#TRI_ARGS[@]} -eq 0 ]]; then
    mapfile -t TRI_ARGS < <(find tests -name "*.tri")
    # also add stdlib tests
    mapfile -t stdlib_tests < <(find stdlib/test -name "*.tri")
    TRI_ARGS+=("${stdlib_tests[@]}")
  else
    # Expand directories and prepend "tests/" to everything
    expanded=()
    for arg in "${TRI_ARGS[@]}"; do
      path="tests/$arg"
      # if arg is stdlib, pass in stdlib/test directory
      if [[ "$arg" == "stdlib" ]]; then
        path="stdlib/test"
      fi
      if [[ -d "$path" ]]; then
        mapfile -t files < <(find "$path" -name "*.tri")
        expanded+=("${files[@]}")
      else
        expanded+=("$path")
      fi
    done
    TRI_ARGS=("${expanded[@]}")
  fi

  for tri_file in "${TRI_ARGS[@]}"; do
    echo "[tools] running $tri_file"
    if ! ./target/debug/tricti "$tri_file" &>tmp/test_output; then
      cat tmp/test_output
      exit 1
    fi
  done
  echo "[tools] all TriCTI native tests passed"
fi
