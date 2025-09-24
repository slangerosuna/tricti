#!/usr/bin/env bash
set -euo pipefail

windsurf_bin=${WINDSURF_BIN:-windsurf}
stop_file=${WINDSURF_STOP_FILE:-}
state_dir=${WINDSURF_STATE_DIR:-}
log_dir=${WINDSURF_LOG_DIR:-}
config_file=${WINDSURF_CONFIG:-}
plan_file=${WINDSURF_PLAN:-}
restart_delay=${WINDSURF_RESTART_DELAY:-5}
max_restarts=${WINDSURF_MAX_RESTARTS:-}
command_spec=${WINDSURF_COMMAND:-"agent run"}
extra_args=${WINDSURF_EXTRA_ARGS:-}
mandate_file=${WINDSURF_MANDATE_FILE:-}

if [[ -n "$stop_file" && -f "$stop_file" ]]; then
  exit 0
fi

if [[ -n "$state_dir" ]]; then
  mkdir -p "$state_dir"
fi

if [[ -n "$log_dir" ]]; then
  mkdir -p "$log_dir"
fi

read -r -a command_parts <<< "$command_spec"
cmd=("$windsurf_bin")
cmd+=("${command_parts[@]}")

if [[ -n "$config_file" ]]; then
  cmd+=("--config" "$config_file")
fi

if [[ -n "$plan_file" ]]; then
  cmd+=("--plan" "$plan_file")
fi

if [[ -n "$state_dir" ]]; then
  cmd+=("--state-dir" "$state_dir")
fi

if [[ -n "$log_dir" ]]; then
  cmd+=("--log-dir" "$log_dir")
fi

if [[ -n "$extra_args" ]]; then
  read -r -a extra_parts <<< "$extra_args"
  cmd+=("${extra_parts[@]}")
fi

if [[ -n "$mandate_file" ]]; then
  cmd+=("--mandate" "$mandate_file")
fi

attempt=0

while true; do
  if [[ -n "$stop_file" && -f "$stop_file" ]]; then
    exit 0
  fi

  attempt=$((attempt + 1))

  set +e
  "${cmd[@]}"
  status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    exit 0
  fi

  if [[ -n "$max_restarts" ]]; then
    if [[ ! "$max_restarts" =~ ^[0-9]+$ ]]; then
      echo "Invalid WINDSURF_MAX_RESTARTS: $max_restarts" >&2
      exit 1
    fi
    if [[ $attempt -ge $max_restarts ]]; then
      exit 1
    fi
  fi

  if [[ -n "$restart_delay" ]]; then
    if [[ "$restart_delay" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
      sleep "$restart_delay"
    else
      sleep 5
    fi
  fi

done
