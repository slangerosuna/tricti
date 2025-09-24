#!/usr/bin/env python3
"""Continuous automation harness driven by OpenAI GPT models.

This script mirrors the intent of `scripts/iterate.sh`, but it implements the
automation loop purely in Python using the `openai` client library. It reads the
roadmap from `next_steps.md`, coordinates Test-Driven Development (TDD) cycles,
and keeps auxiliary artifacts such as `requirements.md` and `plans/` in sync.

The agent loops until every actionable roadmap item is complete or a stop file
is detected. For each item it:

1. Summarizes the current roadmap entry along with the relevant requirements and
   plan documents.
2. Asks an OpenAI GPT model for a TDD-oriented execution plan.
3. Extracts any shell commands suggested by the model (lines beginning with
   ``RUN:``) and executes them, ensuring helper scripts under `scripts/` are
   used in preference to raw Cargo invocations.
4. Persists an interaction transcript so runs can resume after interruptions.

Environment Variables
---------------------
- ``OPENAI_API_KEY`` (required): API key for the OpenAI account.
- ``OPENAI_MODEL`` (optional): Chat model to use (default: ``gpt-4o-mini``).
- ``AGENT_WORK_DIR`` (optional): Directory where state/logs/mandate files are
  persisted. Defaults to ``.windsurf`` under the repository root.
- ``AGENT_RESTART_DELAY`` (optional): Delay in seconds between task iterations
  (default: ``5``).
- ``AGENT_STOP_FILE`` (optional): Path to stop-file sentinel. Defaults to
  ``<work_dir>/STOP``.

Prerequisites
-------------
Install the OpenAI Python client and (optionally) ``rich`` for prettier logs::

    pip install openai>=1.30 rich>=13

Usage
-----
    python continuous_agent.py

The script emits structured logs under ``<work_dir>/logs`` and maintains state in
``<work_dir>/state.json`` so it can resume where it left off.
"""
from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence

try:
    from openai import OpenAI
except ImportError as exc:  # pragma: no cover - dependency hint
    raise SystemExit(
        "The 'openai' package is required. Install it with 'pip install openai'."
    ) from exc

try:  # optional nice-to-have logging
    from rich.console import Console
    from rich.markdown import Markdown
except ImportError:  # pragma: no cover - fallback to print
    Console = None
    Markdown = None


ROOT = Path(__file__).resolve().parent
NEXT_STEPS_PATH = ROOT / "next_steps.md"
REQUIREMENTS_PATH = ROOT / "requirements.md"
PLANS_DIR = ROOT / "plans"
SCRIPTS_DIR = ROOT / "scripts"


@dataclass
class AgentConfig:
    """Runtime configuration for the automation agent."""

    api_key: str
    model: str = field(default_factory=lambda: os.environ.get("OPENAI_MODEL", "gpt-4o-mini"))
    work_dir: Path = field(default_factory=lambda: Path(os.environ.get("AGENT_WORK_DIR", ROOT / ".windsurf")))
    restart_delay: float = field(default_factory=lambda: float(os.environ.get("AGENT_RESTART_DELAY", "5")))
    stop_file: Path = field(default_factory=lambda: Path(os.environ.get("AGENT_STOP_FILE")) if os.environ.get("AGENT_STOP_FILE") else None)

    def __post_init__(self) -> None:
        self.work_dir.mkdir(parents=True, exist_ok=True)
        (self.work_dir / "logs").mkdir(parents=True, exist_ok=True)
        (self.work_dir / "state").mkdir(parents=True, exist_ok=True)
        if self.stop_file is None:
            self.stop_file = self.work_dir / "STOP"


@dataclass
class RoadmapItem:
    """Represents an actionable bullet extracted from `next_steps.md`."""

    section: str
    label: str
    description: str
    plan: Optional[str]
    pseudocode: Optional[str]
    checkbox_path: str

    def friendly_title(self) -> str:
        return f"{self.section} {self.label}"


@dataclass
class AgentState:
    """Serialized interaction state so the agent can resume reliably."""

    completed_ids: List[str] = field(default_factory=list)
    current_id: Optional[str] = None

    @classmethod
    def load(cls, path: Path) -> "AgentState":
        if not path.exists():
            return cls()
        data = json.loads(path.read_text())
        return cls(
            completed_ids=data.get("completed_ids", []),
            current_id=data.get("current_id"),
        )

    def save(self, path: Path) -> None:
        payload = {"completed_ids": self.completed_ids, "current_id": self.current_id}
        path.write_text(json.dumps(payload, indent=2))


class ContinuousAgent:
    """Main automation loop orchestrating GPT interactions and command execution."""

    def __init__(self, config: AgentConfig) -> None:
        if not config.api_key:
            raise SystemExit("OPENAI_API_KEY is not set")

        self.config = config
        self.client = OpenAI(api_key=config.api_key)
        self.state_file = config.work_dir / "state" / "state.json"
        self.transcript_dir = config.work_dir / "logs"
        self.state = AgentState.load(self.state_file)
        self.console = Console(stderr=True) if Console else None
        self.pending_writes: Dict[str, str] = {}

    # ------------------------------------------------------------------
    # High-level control flow
    # ------------------------------------------------------------------
    def run(self) -> None:
        self._log_heading("Starting continuous agent loop")
        while True:
            if self.config.stop_file.exists():
                self._log("Stop file detected at %s; exiting." % self.config.stop_file)
                break

            roadmap_items = self._load_roadmap()
            next_item = self._select_next_item(roadmap_items)
            if next_item is None:
                self._log("All roadmap items appear completed. Done!")
                break

            self.state.current_id = next_item.checkbox_path
            self.state.save(self.state_file)

            guidance = self._request_guidance(next_item)
            operations = self._extract_operations(guidance)

            self._log(
                "Executing %d operations suggested by the model" % len(operations)
            )
            executed_log: List[str] = []
            for op in operations:
                result = self._execute_operation(op)
                executed_log.append(result)

            self._record_transcript(next_item, guidance, executed_log)

            self.state.completed_ids.append(next_item.checkbox_path)
            self.state.current_id = None
            self.state.save(self.state_file)

            time.sleep(self.config.restart_delay)

    # ------------------------------------------------------------------
    # Roadmap processing helpers
    # ------------------------------------------------------------------
    def _load_roadmap(self) -> List[RoadmapItem]:
        text = NEXT_STEPS_PATH.read_text(encoding="utf-8")
        sections = re.split(r"^\d+\.\s+", text, flags=re.MULTILINE)
        headers = re.findall(r"^(\d+\.\s+.+)$", text, flags=re.MULTILINE)

        items: List[RoadmapItem] = []
        for header, section_body in zip(headers, sections[1:]):
            section_title = header.strip()
            matches = re.finditer(
                r"- \*\*\[(?P<label>[^\]]+)\]\*\*\s+(?P<desc>.+?)\n"
                r"\s*- \*\*Plan:\*\*\s*(?P<plan>.+?)\[ \]\n"
                r"(?:\s*- \*\*Pseudocode:\*\*\n``\w+\n(?P<pseudo>.*?)\n``\n)?",
                section_body,
                flags=re.DOTALL,
            )
            for match in matches:
                label = match.group("label")
                description = match.group("desc").strip()
                plan_text = match.group("plan").strip()
                pseudo = match.group("pseudo")
                checkbox_path = f"{section_title} -> {label}"
                items.append(
                    RoadmapItem(
                        section=section_title,
                        label=label,
                        description=description,
                        plan=plan_text,
                        pseudocode=pseudo.strip() if pseudo else None,
                        checkbox_path=checkbox_path,
                    )
                )
        return items

    def _select_next_item(self, items: Sequence[RoadmapItem]) -> Optional[RoadmapItem]:
        for item in items:
            if item.checkbox_path in self.state.completed_ids:
                continue
            return item
        return None

    # ------------------------------------------------------------------
    # GPT interaction utilities
    # ------------------------------------------------------------------
    def _request_guidance(self, item: RoadmapItem) -> str:
        context = self._build_context(item)
        messages = [
            {
                "role": "system",
                "content": (
                    "You are a senior compiler engineer practicing strict TDD. "
                    "Work sequentially through roadmap items, prefer helper shell scripts in `scripts/`, "
                    "and update requirements/plans after each cycle. The automation can execute shell commands, "
                    "read files, read specific line ranges, list directories, and write, append, or edit specific "
                    "line ranges in files. Use the following directives so actions can be parsed and performed "
                    "automatically: prefix shell commands with 'RUN: ', emit file reads as 'READ: <path>' or "
                    "'READ_LINES: <path> <start> <end>', list directories with 'LIST: <path>', write/appends using "
                    "'WRITE: <path>' or 'APPEND: <path>' followed by a fenced code block containing the file contents. "
                    "Use 'EDIT_LINES: <path> <start> <end>' to fetch the existing lines (they will be returned to you), "
                    "then provide replacements with 'APPLY_EDIT: <path> <start> <end>' followed by a fenced code block. "
                    "When overwriting an existing file with WRITE, you must confirm using 'APPLY_WRITE: <path>' after inspecting the preview. "
                    "WRITE directives are limited to 100 lines of content—exceeding this limit will be rejected with a warning."
                ),
            },
            {
                "role": "user",
                "content": context,
            },
        ]

        self._log("Requesting guidance from %s" % self.config.model)
        response = self.client.chat.completions.create(
            model=self.config.model,
            messages=messages,
            temperature=0.2,
        )
        choice = response.choices[0]
        return choice.message.content or ""

    def _build_context(self, item: RoadmapItem) -> str:
        requirements = REQUIREMENTS_PATH.read_text(encoding="utf-8") if REQUIREMENTS_PATH.exists() else ""
        related_plans = self._gather_related_plans(item)
        context_parts = [
            f"Roadmap item: {item.friendly_title()}",
            f"Description: {item.description}",
            f"Plan snippet: {item.plan or 'N/A'}",
            f"Pseudocode:\n{item.pseudocode or 'N/A'}",
            "Current requirements.md:\n" + requirements,
            "Relevant plan documents:" + ("\n" + "\n".join(related_plans) if related_plans else " (none found)"),
            "Remember: respond with clear TDD steps and include shell commands prefixed with 'RUN:'.",
        ]
        return "\n\n".join(context_parts)

    def _gather_related_plans(self, item: RoadmapItem) -> List[str]:
        matches = re.findall(r"`(plans/.+?\.md)`", item.description + (item.plan or ""))
        contents = []
        for rel_path in matches:
            plan_path = ROOT / rel_path
            if plan_path.exists():
                contents.append(f"## {rel_path}\n" + plan_path.read_text(encoding="utf-8"))
        return contents

    # ------------------------------------------------------------------
    # Command execution + logging
    # ------------------------------------------------------------------
    RUN_PATTERN = re.compile(r"^RUN:\s*(.+)$", re.MULTILINE)
    READ_PATTERN = re.compile(r"^READ:\s*(.+)$", re.MULTILINE)
    READ_LINES_PATTERN = re.compile(
        r"^READ_LINES:\s*(?P<path>.+?)\s+(?P<start>\d+)\s+(?P<end>\d+)$",
        re.MULTILINE,
    )
    LIST_PATTERN = re.compile(r"^LIST:\s*(.+)$", re.MULTILINE)
    WRITE_PATTERN = re.compile(
        r"^WRITE:\s*(?P<path>.+?)\s*$\n```(?P<lang>[^\n]*)\n(?P<body>.*?)\n```",
        re.MULTILINE | re.DOTALL,
    )
    APPEND_PATTERN = re.compile(
        r"^APPEND:\s*(?P<path>.+?)\s*$\n```(?P<lang>[^\n]*)\n(?P<body>.*?)\n```",
        re.MULTILINE | re.DOTALL,
    )
    EDIT_LINES_PATTERN = re.compile(
        r"^EDIT_LINES:\s*(?P<path>.+?)\s+(?P<start>\d+)\s+(?P<end>\d+)$",
        re.MULTILINE,
    )
    APPLY_EDIT_PATTERN = re.compile(
        r"^APPLY_EDIT:\s*(?P<path>.+?)\s+(?P<start>\d+)\s+(?P<end>\d+)\s*$\n``(?P<lang>[^\n]*)\n(?P<body>.*?)\n```",
        re.MULTILINE | re.DOTALL,
    )
    APPLY_WRITE_PATTERN = re.compile(
        r"^APPLY_WRITE:\s*(?P<path>.+?)\s*$",
        re.MULTILINE,
    )

    @dataclass
    class Operation:
        kind: str  # run | read | read_lines | list | write | append | edit_lines | apply_edit | apply_write
        args: Optional[Sequence[str]] = None
        path: Optional[str] = None
        content: Optional[str] = None
        start_line: Optional[int] = None
        end_line: Optional[int] = None

    def _extract_operations(self, guidance: str) -> List["ContinuousAgent.Operation"]:
        operations: List[ContinuousAgent.Operation] = []

        for match in self.RUN_PATTERN.finditer(guidance):
            raw_cmd = match.group(1).strip()
            if raw_cmd:
                parts = shlex.split(raw_cmd)
                operations.append(self.Operation(kind="run", args=parts))

        for match in self.READ_PATTERN.finditer(guidance):
            path_str = match.group(1).strip()
            if path_str:
                operations.append(self.Operation(kind="read", path=path_str))

        for match in self.READ_LINES_PATTERN.finditer(guidance):
            path_str = match.group("path").strip()
            start = int(match.group("start"))
            end = int(match.group("end"))
            if path_str:
                operations.append(
                    self.Operation(
                        kind="read_lines",
                        path=path_str,
                        start_line=start,
                        end_line=end,
                    )
                )

        for match in self.LIST_PATTERN.finditer(guidance):
            path_str = match.group(1).strip()
            if path_str:
                operations.append(self.Operation(kind="list", path=path_str))

        for pattern, kind in [(self.WRITE_PATTERN, "write"), (self.APPEND_PATTERN, "append")]:
            for match in pattern.finditer(guidance):
                path_str = match.group("path").strip()
                body = match.group("body")
                if path_str:
                    operations.append(self.Operation(kind=kind, path=path_str, content=body))

        for match in self.EDIT_LINES_PATTERN.finditer(guidance):
            path_str = match.group("path").strip()
            start = int(match.group("start"))
            end = int(match.group("end"))
            if path_str:
                operations.append(
                    self.Operation(
                        kind="edit_lines",
                        path=path_str,
                        start_line=start,
                        end_line=end,
                    )
                )

        for match in self.APPLY_EDIT_PATTERN.finditer(guidance):
            path_str = match.group("path").strip()
            start = int(match.group("start"))
            end = int(match.group("end"))
            body = match.group("body")
            if path_str:
                operations.append(
                    self.Operation(
                        kind="apply_edit",
                        path=path_str,
                        content=body,
                        start_line=start,
                        end_line=end,
                    )
                )

        for match in self.APPLY_WRITE_PATTERN.finditer(guidance):
            path_str = match.group("path").strip()
            if path_str:
                operations.append(
                    self.Operation(
                        kind="apply_write",
                        path=path_str,
                    )
                )

        return operations

    def _execute_operation(self, op: "ContinuousAgent.Operation") -> str:
        if op.kind == "run":
            if not op.args:
                raise ValueError("RUN operation missing arguments")
            args = list(op.args)
            self._log(
                "Running command: %s" % " ".join(shlex.quote(a) for a in args)
            )
            process = subprocess.run(args, cwd=ROOT, text=True)
            if process.returncode != 0:
                raise SystemExit(
                    "Command failed with exit code %d: %s"
                    % (process.returncode, " ".join(args))
                )
            return "RUN: " + " ".join(shlex.quote(a) for a in args)

        if not op.path:
            raise ValueError(f"{op.kind} operation missing path")
        path = self._resolve_path(op.path)
        if op.kind == "read":
            content = path.read_text(encoding="utf-8")
            self._log(f"READ {path}:\n{content}")
            return f"READ {path}\n{content}"

        if op.kind == "read_lines":
            lines = path.read_text(encoding="utf-8").splitlines()
            start = max((op.start_line or 1) - 1, 0)
            end = op.end_line or (start + 1)
            snippet = lines[start:end]
            rendered = "\n".join(snippet)
            self._log(
                f"READ_LINES {path} [{op.start_line}-{op.end_line}] ->\n{rendered}"
            )
            return f"READ_LINES {path} {op.start_line}-{op.end_line}\n{rendered}"

        if op.kind == "edit_lines":
            lines = path.read_text(encoding="utf-8").splitlines()
            start = max((op.start_line or 1) - 1, 0)
            end = op.end_line or (start + 1)
            snippet = lines[start:end]
            rendered = "\n".join(
                f"{idx}: {text}"
                for idx, text in zip(range(start + 1, start + 1 + len(snippet)), snippet)
            )
            self._log(
                f"EDIT_LINES preview {path} [{op.start_line}-{op.end_line}] ->\n{rendered}"
            )
            return (
                f"EDIT_LINES {path} {op.start_line}-{op.end_line} preview\n"
                f"{rendered or '(empty range)'}"
            )

        if op.kind == "list":
            entries = (
                sorted(p.name for p in path.iterdir())
                if path.exists() and path.is_dir()
                else []
            )
            listing = "\n".join(entries)
            self._log(f"LIST {path} ->\n{listing}")
            return f"LIST {path}\n{listing}"

        if op.kind == "write":
            path.parent.mkdir(parents=True, exist_ok=True)
            content = op.content or ""
            line_count = len(content.splitlines())
            if line_count > 100:
                warning = (
                    f"WRITE directive rejected: {line_count} lines exceeds 100-line limit for {path}."
                )
                self._log(warning)
                return warning
            key = str(path)
            if path.exists() and key not in self.pending_writes:
                existing = path.read_text(encoding="utf-8")
                preview = "\n".join(
                    f"{idx + 1}: {line}"
                    for idx, line in enumerate(existing.splitlines())
                )
                self.pending_writes[key] = content
                confirmation = (
                    "WRITE confirmation required. Existing file contents (first 2000 chars):\n"
                    f"{preview[:2000]}\n"
                    f"Respond with APPLY_WRITE: {path} to confirm overwrite."
                )
                self._log(confirmation)
                return confirmation
            pending = self.pending_writes.pop(key, content)
            final_line_count = len(pending.splitlines())
            path.write_text(pending, encoding="utf-8")
            self._log(f"WRITE {path} ({len(pending)} bytes, {final_line_count} lines)")
            return f"WRITE {path} ({len(pending)} bytes, {final_line_count} lines)"

        if op.kind == "append":
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as fh:
                fh.write(op.content or "")
            self._log(f"APPEND {path} ({len(op.content or '')} bytes)")
            return f"APPEND {path} ({len(op.content or '')} bytes)"

        if op.kind == "apply_edit":
            existing = path.read_text(encoding="utf-8").splitlines()
            start_idx = max((op.start_line or 1) - 1, 0)
            end_idx = (op.end_line or (op.start_line or 1))
            replacement = (op.content or "").splitlines()
            existing[start_idx:end_idx] = replacement
            new_text = "\n".join(existing) + "\n"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(new_text, encoding="utf-8")
            self._log(
                f"EDIT_LINES {path} [{op.start_line}-{op.end_line}] ({len(replacement)} lines)"
            )
            return (
                f"EDIT_LINES {path} {op.start_line}-{op.end_line} "
                f"({len(replacement)} lines)"
            )

        if op.kind == "apply_write":
            key = str(path)
            if key not in self.pending_writes:
                message = f"APPLY_WRITE received for {path}, but no pending write was recorded."
                self._log(message)
                return message
            pending = self.pending_writes.pop(key)
            final_line_count = len(pending.splitlines())
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(pending, encoding="utf-8")
            self._log(
                f"APPLY_WRITE committed {path} ({len(pending)} bytes, {final_line_count} lines)"
            )
            return (
                f"APPLY_WRITE {path} ({len(pending)} bytes, {final_line_count} lines committed)"
            )

        raise ValueError(f"Unknown operation kind: {op.kind}")

    def _resolve_path(self, path_str: str) -> Path:
        path = Path(path_str)
        if not path.is_absolute():
            path = ROOT / path
        return path.resolve()

    def _record_transcript(
        self, item: RoadmapItem, guidance: str, executed_log: Iterable[str]
    ) -> None:
        timestamp = int(time.time())
        transcript_path = self.transcript_dir / f"{timestamp}_{item.label}.md"
        command_lines = "\n".join(executed_log)
        transcript = (
            f"# {item.friendly_title()}\n\n"
            f"Guidance:\n\n{guidance}\n\n"
            f"Commands executed:\n\n{command_lines or '(none)'}\n"
        )
        transcript_path.write_text(transcript, encoding="utf-8")

        if self.console and Markdown:
            self.console.rule(f"Transcript saved: {transcript_path}")
            self.console.print(Markdown(transcript))

    # ------------------------------------------------------------------
    # Logging helpers
    # ------------------------------------------------------------------
    def _log(self, message: str) -> None:
        if self.console:
            self.console.log(message)
        else:  # pragma: no cover
            print(message)

    def _log_heading(self, message: str) -> None:
        if self.console:
            self.console.rule(message)
        else:  # pragma: no cover
            print(f"==== {message} ====")


# ----------------------------------------------------------------------
# Entrypoint
# ----------------------------------------------------------------------

def main(argv: Optional[Sequence[str]] = None) -> int:
    api_key = os.environ.get("OPENAI_API_KEY")
    config = AgentConfig(api_key=api_key or "")
    agent = ContinuousAgent(config)
    try:
        agent.run()
    except KeyboardInterrupt:
        agent._log("Interrupted by user.")
        return 130
    return 0


if __name__ == "__main__":
    sys.exit(main())
