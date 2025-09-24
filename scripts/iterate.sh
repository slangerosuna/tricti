#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ -f "$REPO_ROOT/.env" ]]; then
  # shellcheck disable=SC1091
  set -a
  source "$REPO_ROOT/.env"
  set +a
fi

WORK_ROOT_DEFAULT="$REPO_ROOT/.windsurf"
WORK_ROOT="${WINDSURF_WORK_ROOT:-$WORK_ROOT_DEFAULT}"
STATE_DIR="$WORK_ROOT/state"
LOG_DIR="$WORK_ROOT/logs"
MANDATE_FILE="$WORK_ROOT/mandate.md"
PLAN_FILE="$WORK_ROOT/plan.yaml"
CONFIG_FILE="$WORK_ROOT/windsurf.agent.yml"
STOP_FILE="$WORK_ROOT/STOP"

mkdir -p "$STATE_DIR" "$LOG_DIR"

if [[ ! -f "$REPO_ROOT/requirements.md" ]]; then
  cat <<'EOF' > "$REPO_ROOT/requirements.md"
# Requirements

- [ ] Placeholder entry — update via TDD cycles.
EOF
fi

if [[ ! -d "$REPO_ROOT/plans" ]]; then
  mkdir -p "$REPO_ROOT/plans"
fi

cat <<'EOF' > "$MANDATE_FILE"
You are an engineering assistant working on the Peano compiler. Your mandate is to follow the execution roadmap in `next_steps.md`, keep `requirements.md` synchronized with actual progress, and continuously update the relevant plan documents under `plans/`. You must adhere to a strict Test-Driven Development (TDD) workflow for every change. Create `requirements.md` and the necessary `plans/` files if they do not yet exist.

Core Responsibilities

1. Sequential Execution
   - Iterate through the numbered sections and bullet items in `next_steps.md` in order. Start with the first section, then proceed sequentially; do not skip ahead unless explicitly instructed.
   - Use the helper scripts in `scripts/` (e.g., `scripts/test.sh`, `scripts/fmt.sh`, `scripts/build.sh`) instead of calling Cargo directly so environment variables are honored.

2. Per-Item TDD Loop
   - Red (Fail): Identify or write the test that currently fails because the requirement is unmet. If a concrete test cannot yet be written, document the failing scenario in detail (location, expected vs. actual behavior).
   - Green (Implement): Modify the codebase—tests and implementation—to satisfy the requirement. Keep the change focused on the current item.
   - Refactor: Clean up any local issues introduced during implementation (naming, duplication) without performing large, unrelated refactors.

3. Git Workflow
   - After each commit-worthy TDD cycle, run `git commit` from the command line (do not push).
   - Each commit message must reference the relevant requirements and plan documents for traceability.

4. Tooling Discipline
   - Prefer the scripts in `scripts/` for running tests, formatting, and builds (`scripts/test.sh`, `scripts/fmt.sh`, etc.).
   - When direct Cargo commands are unavoidable, document why and keep them scoped to the current task.

5. Documentation & Plan Maintenance
   - After each TDD cycle, update `requirements.md` to describe what changed, reference the passing tests, and link to the relevant `plans/` entries. Adjust status markers (`[x]`, `[~]`, `[ ]`) to reflect progress.
   - Update the associated plan file(s) in `plans/` with newly discovered details, pseudocode or implementation sketches, complexity notes, roadmap adjustments, and open questions.
   - Keep plan edits scoped to the sections influenced by the current work item. Preserve existing Markdown formatting conventions.

6. Workflow Guidance
   - Edit only the files necessary for the current item, plus `requirements.md`, `next_steps.md` (if resequencing is required), and the relevant plan files.
   - Cite plan documents and requirements explicitly in commit summaries or change logs to preserve traceability.
   - Keep change sets small and logical—one actionable bullet per change batch.

7. Exit Criteria per Item
   - Tests are updated and passing (or clearly documented if pending external work).
   - `requirements.md` accurately reflects the new state.
   - Plan documents under `plans/` capture any new insights or TODOs.
   - Any introduced TODOs are tracked and linked to plan entries.

Repeat this loop diligently until the roadmap in `next_steps.md` is complete or further instructions arrive. Do not stop on your own unless `next_steps.md` is complete.
EOF

cat <<'YAML' > "$PLAN_FILE"
tasks:
  - id: roadmap_tdd
    description: "Follow next_steps.md sequentially with strict TDD and documentation updates."
    references:
      - next_steps.md
      - requirements.md
    notes:
      - "Use helper scripts in scripts/ for builds, tests, and formatting."
      - "Update plans/ documents after each actionable bullet."
YAML

cat <<'YAML' > "$CONFIG_FILE"
execution:
  auto_continue: true
  pause_on_completion: true
  max_idle_cycles: 0
logging:
  level: info
YAML

export WINDSURF_CONFIG="${WINDSURF_CONFIG:-$CONFIG_FILE}"
export WINDSURF_PLAN="${WINDSURF_PLAN:-$PLAN_FILE}"
export WINDSURF_STATE_DIR="${WINDSURF_STATE_DIR:-$STATE_DIR}"
export WINDSURF_LOG_DIR="${WINDSURF_LOG_DIR:-$LOG_DIR}"
export WINDSURF_STOP_FILE="${WINDSURF_STOP_FILE:-$STOP_FILE}"
export WINDSURF_MANDATE_FILE="${WINDSURF_MANDATE_FILE:-$MANDATE_FILE}"
export WINDSURF_RESTART_DELAY="${WINDSURF_RESTART_DELAY:-5}"

if [[ -z "${WINDSURF_BIN:-}" ]]; then
  export WINDSURF_BIN="windsurf"
fi

AGENT_LOOP="$SCRIPT_DIR/agent_loop.sh"
if [[ ! -x "$AGENT_LOOP" ]]; then
  echo "Missing executable agent loop script at $AGENT_LOOP" >&2
  exit 1
fi

exec "$AGENT_LOOP"
