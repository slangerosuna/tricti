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
REPO_ROOT="$(pwd)"
trap 'echo "[tools] cleaning tmp artifacts"; rm -f tmp/output tests/tmp_*.o tests/tmp_*.out || true' EXIT

# Array of stdlib test files
stdlib_tests=(
  "src/simple_test.tri"
  "src/stdlib_tests.tri"
  "src/test_traits_syntax.tri"
  "src/minimal_parallel_test.tri"
  "src/parallel_vec_test.tri"
  "src/parallel_vec_comprehensive_test.tri"
  "src/simple_parallel_test.tri"
)

TEST_TIMEOUT="${TEST_TIMEOUT:-30s}"

# Argument parsing
RUN_TRI=false
RUN_CARGO=false
CUSTOM_TEST_TIMEOUT=false
TRI_ARGS=()
CARGO_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -t)
      RUN_TRI=true
      shift
      # collect everything until next -c or end
      while [[ $# -gt 0 && "$1" != "-c" && "$1" != "-t" && "$1" != "-to" ]]; do
        TRI_ARGS+=("$1")
        shift
      done
      ;;
    -c)
      RUN_CARGO=true
      shift
      # collect everything until next -t or end
      while [[ $# -gt 0 && "$1" != "-c" && "$1" != "-t" && "$1" != "-to" ]]; do
        CARGO_ARGS+=("$1")
        shift
      done
      ;;
    -to)
      CUSTOM_TEST_TIMEOUT=true
      shift
      if [[ $# -eq 0 ]]; then
        echo "[tools] error: -to requires a timeout value"
        exit 1
      fi

      TEST_TIMEOUT="$1"
      shift
      ;;
    *)
      # treat as positional args if no flag
      TRI_ARGS+=("$1")
      CARGO_ARGS+=("$1")
      shift
      ;;
  esac
done

if ! ($RUN_TRI || $RUN_CARGO); then
  RUN_TRI=true
  RUN_CARGO=true
fi

if $CUSTOM_TEST_TIMEOUT && ! $RUN_CARGO; then
  echo "[tools] warning: -to specified without -c, ignoring custom timeout"
fi

if $RUN_CARGO; then
  echo "[tools] running cargo tests"
  if [ "$TEST_TIMEOUT" == "none" ]; then
    cargo test -- --nocapture "${CARGO_ARGS[@]}" &>tmp/test_output
  else
    timeout "$TEST_TIMEOUT" cargo test -- --nocapture "${CARGO_ARGS[@]}" &>tmp/test_output
  fi
  
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
    TRI_ARGS+=("${stdlib_tests[@]}")
  else
    # Expand directories and prepend "tests/" to everything
    expanded=()
    for arg in "${TRI_ARGS[@]}"; do
      path="tests/$arg"
      if [[ -d "$path" ]]; then
        mapfile -t files < <(find "$path" -name "*.tri")
        expanded+=("${files[@]}")
      else
        expanded+=("$path")
      fi
    done
    TRI_ARGS=("${expanded[@]}")
  fi

  declare -A RUN_ALL_DIRECTORIES=()

  for tri_file in "${TRI_ARGS[@]}"; do
    if [[ ! -f "$tri_file" ]]; then
      echo "[tools] warning: test file '$tri_file' does not exist, skipping"
      continue
    fi

    dir_name=$(dirname "$tri_file")
    run_all_path="$dir_name/run_all.sh"
    if [[ -f "$run_all_path" ]]; then
      RUN_ALL_DIRECTORIES["$dir_name"]=1
      echo "[tools] found run_all.sh in $dir_name"
      continue
    fi

    echo "[tools] found test file $tri_file"
  done

  declare -A RUN_ALL_EXECUTED=()

  for tri_file in "${TRI_ARGS[@]}"; do
    if [[ ! -f "$tri_file" ]]; then
      continue
    fi

    dir_name=$(dirname "$tri_file")
    run_all_path="$dir_name/run_all.sh"

    if [[ -f "$run_all_path" ]]; then
      if [[ -z "${RUN_ALL_EXECUTED["$dir_name"]:-}" ]]; then
        echo "[tools] running $run_all_path"
        if [[ -x "$run_all_path" ]]; then
          if ! (cd "$dir_name" && ./run_all.sh) &>"$REPO_ROOT/tmp/test_output"; then
            cat tmp/test_output
            exit 1
          fi
        else
          if ! (cd "$dir_name" && bash ./run_all.sh) &>"$REPO_ROOT/tmp/test_output"; then
            cat tmp/test_output
            exit 1
          fi
        fi
        RUN_ALL_EXECUTED["$dir_name"]=1
      fi
      continue
    fi

    echo "[tools] running $tri_file"
    if ! ./target/debug/tricti "$tri_file" &>tmp/test_output; then
      cat tmp/test_output
      exit 1
    fi
  done
  echo "[tools] all TriCTI native tests passed"
fi
