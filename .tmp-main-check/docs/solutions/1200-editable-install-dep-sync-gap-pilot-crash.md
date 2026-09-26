---
title: "Editable install dependency sync gap crashes claude-pilot silently"
date: 2026-05-18
category: runtime-errors
module: dispatch-lib
problem_type: runtime_error
component: tooling
symptoms:
  - "claude-pilot exits code 1 in ~5 seconds with zero log output"
  - "Empty /var/log/claude-pilot/<task-id>.log file"
  - "Task result shows ModuleNotFoundError traceback (buried in child task row)"
  - "Branch created but unpopulated — worktree mutation happens after crash point"
root_cause: config_error
resolution_type: code_fix
severity: high
tags:
  - claude-pilot
  - editable-install
  - uv-tool-install
  - dispatch-lib
  - pyproject-toml
  - dependency-sync
  - smoke-test
---

# Editable install dependency sync gap crashes claude-pilot silently

## Problem

After mika#1192 added `import yaml` to `claude_pilot/policy.py` and `pyyaml>=6.0,<7` to `pyproject.toml`, all claude-pilot dispatches (both grooming and implementation) crashed with exit code 1 in ~5 seconds. Zero log output, zero commits, opaque failure — the operator had to trace through child task DB rows to find the buried Python traceback.

## Symptoms

- `run_claude_pilot_groom` child task exits in ~3-5 seconds with `status=delivered`, `result="claude-pilot FAILED (exit code 1)"`
- Log file at `/var/log/claude-pilot/<task-id>.log` is empty (file-logger init lives below the failing import)
- Supervisor task shows `status=cancelled, result=NULL` — no diagnostic in the supervisor row
- Branch created on disk but unpopulated (worktree mutation happens after the crash point)
- Traceback buried in `tasks.result` column of the child task row:
  ```
  File "/home/samidarko/.local/bin/claude-pilot", line 4, in <module>
      from claude_pilot.cli import main
  File ".../cli.py", line 23, in <module>
      from .agent import run_agent
  File ".../agent.py", line 21, in <module>
      from .permissions import CanUseTool
  File ".../permissions.py", line 25, in <module>
      from .policy import Policy, evaluate, load_policy
  File ".../policy.py", line 19, in <module>
      import yaml
  ModuleNotFoundError: No module named 'yaml'
  ```

## What Didn't Work

- **mika#1168 fix (PR #1197)** addressed a different failure class (sonnet-classifier refusal + qa-review-allowlist-shadow). The empty-handed signature persisted post-#1197 because it was an independent co-cause.
- **Checking the operator-spawn path** worked fine (`/mika-spawn /mika-groom-ticket mika#N`) — the operator shell had the correct venv because it was rebuilt more recently. This masked the autonomous-loop breakage.

## Solution

**Immediate restoration (operator action):**
```bash
cd /data/workspace/mika-platform
uv tool install --force --editable ./claude-pilot-py
```
This re-resolves dependencies against the current `pyproject.toml` and installs `pyyaml` into the venv.

**Structural guard (code fix in `dispatch-lib.sh`):**
Added a `claude-pilot --help` smoke test immediately after the existing `command -v claude-pilot` check in `dispatch_claude_pilot()`, before any worktree mutation:

```bash
# claude-pilot venv smoke test (mika#1200)
if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then
    cat >&2 <<'EOF'
Error: claude-pilot venv is broken — `claude-pilot --help` exited non-zero.
Most likely cause: pyproject.toml changed in claude-pilot-py without an
accompanying `uv tool install` to re-sync dependencies.

To restore the loop:
    cd <mika-platform-root> && uv tool install --force --editable ./claude-pilot-py
EOF
    exit 1
fi
```

Key design choices:
- `timeout 15` prevents hung venvs from blocking dispatch indefinitely
- `2>&9` routes stderr to the trace file (fd 9) for diagnostics instead of discarding
- `exit 1` matches surrounding `command -v` control flow (not `return 1`)
- Fires BEFORE `_set_up_worktree` — converts "branch created but unpopulated" into "no worktree, clear error"
- The `--help` command exercises the full import chain because `cli.py` has top-level imports of `.agent`, `.permissions`, and transitively `.policy` (which imports `yaml`)

## Why This Works

**Root cause:** `uv tool install --force --editable` provisions the venv with exactly the dependencies declared in `pyproject.toml` at install time. The `--editable` flag makes source changes take effect immediately (via `.pth` file), but new dependencies declared after install are NOT auto-synced — `uv` must be re-run. When mika#1192 added `pyyaml>=6.0,<7` to `pyproject.toml` and `import yaml` to `policy.py`, the editable source resolution picked up `policy.py` immediately, but `pyyaml` was never installed because `uv tool install` was not re-run.

**The smoke test catches this class** because `claude-pilot --help` triggers Python to evaluate the entire `cli.py` module body (top-level imports), which transitively pulls in the full dependency chain. A missing import causes the help command to fail with the same `ModuleNotFoundError` that crashes real dispatches. The error message names the exact restoration command, replacing the opaque "exit 1, empty log" signature with an actionable diagnostic.

## Prevention

- **Convention:** Any change to `claude-pilot-py/pyproject.toml` requires `make deploy` (which runs `uv tool install --force --editable`) before the next autonomous-loop dispatch.
- **Structural backstop:** The `dispatch-lib.sh` smoke test catches the failure when the convention is missed. The error message names the fix command.
- **Regression guard (Test 10):** A grep-based test asserts `cli.py` keeps `.agent` and `.permissions` imports at module top level. If a future refactor moves them inside `main()` (lazy import), the smoke test silently stops detecting import-time failures — this test catches that regression.

## Related Issues

- mika#1200 — this bug
- mika#1192 / PR #1199 — introduced the `import yaml` dependency via deterministic policy evaluator
- mika#1168 / PR #1197 — different failure class (sonnet-classifier + qa-review-allowlist-shadow)
- mika#1173 / PR #1187 — dev-groom revert (added `run_claude_pilot_groom` tool)
- mika#1189 — same empty-handed signature (pre-#1197, may have been same root cause)
