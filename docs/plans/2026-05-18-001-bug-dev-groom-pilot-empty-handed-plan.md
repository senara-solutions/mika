---
ticket: mika#1200
type: bug
status: active
branch: bug/1200/dev-groom-pilot-session-exits-empty
groomed_by: orchestrator-Claude
date: 2026-05-18
---

# Plan — mika#1200: claude-pilot exits empty-handed (PyYAML missing from editable venv after mika#1192 deploy)

## Revision history

- **Pass 1 → Pass 2 (this revision).** Addresses mika-arch first-pass findings:
  - **F1 BLOCKING (Phase 0 Pin absent):** Added a "Phase 0 Pin" section with verbatim
    slices of `dispatch-lib.sh:693-774` and `cli.py:1-29`, pinning the insertion site,
    execution mode (executed-not-sourced), the exact line offsets to `_set_up_worktree`,
    and the `set -x`/stderr routing pattern the new error message must match.
  - **F2 BLOCKING (smoke command may not exercise import chain):** Pinned the smoke
    command as `claude-pilot --help` with evidence (`cli.py` top-level imports at lines
    23-28 trigger the full chain via `from claude_pilot.cli import main` in the
    `~/.local/bin/claude-pilot` shim). Verified `--version` is NOT registered in
    `_build_parser()`. Added a future-proofing comment in the proposed code block and
    a regression-guard test (Phase 2 case d) that asserts cli.py keeps top-level
    imports.
  - **NF5 (ratification with caveat):** Phase 3 CLAUDE.md edit now cites both the
    convention AND the structural backstop, with explicit framing of how they work
    together.
  - **NF6 (observation):** Added explicit follow-up ticket commitments for both
    "loop-resolves-to-merged-source" policy question and "mika#1189 yesterday's same-
    signature failure" (file separately if it reproduces post-fix).
  - **NF1-NF4, NF7, U1-U6 (all ratifications):** No structural changes required; the
    pass-1 draft already matched.
- **Pass 1 (initial draft).** Diagnosis + 3-phase plan + 6 deliberate uncertainties.

## TL;DR

The ticket frames this as a groom-specific failure, the third co-cause after mika#1168's
sonnet-classifier + qa-review-allowlist-shadow fixes. **The ticket framing is wrong by
sampling bias.** The actual failure is environment, not code: today's mika#1192 (PR #1199)
added `import yaml` to `claude_pilot/policy.py` (the new deterministic policy evaluator),
which `cli.py → agent.py → permissions.py → policy.py` transitively pulls into every
claude-pilot invocation. The editable install at `~/.local/share/uv/tools/claude-pilot/`
was provisioned on 2026-05-18 12:45:48 — before policy.py + the `pyyaml>=6.0,<7`
dependency landed at 16:06 — so the venv METADATA (verified) lists only
`claude-agent-sdk`, `pydantic`, `python-dotenv` and does NOT include pyyaml. Editable
install resolves new source automatically but does NOT auto-sync new deps. Result: every
post-16:06 claude-pilot invocation imports `policy.py`, hits `import yaml`,
ModuleNotFoundError, exits 1 in ~3-5 seconds before the file-logger initializes — hence
the empty `/var/log/claude-pilot/<id>.log`. Implementation dispatches would fail the same
way; the only reason the ticket reads "dev-groom-specific" is that no implementation
dispatch was attempted in the post-16:06 window (verified via `tasks` table —
`4586dcf9-3589-417e-8e0c-6d12cb7efe16` is the only `long_running:run_claude_pilot*` row
since 15:00Z today, and one earlier `:deferred` slot-freed at 16:00).

The fix is therefore narrowly scoped at the surface level (re-run `uv tool install` to
pick up pyyaml) but the structural failure mode is broader: editable installs decouple
source-sync from dep-sync, and `dispatch-lib.sh` has no pre-flight smoke test, so a
broken venv fails opaquely (empty log file, 5s exit, traceback only in the stderr
capture inside the task result column of a single DB row). This plan addresses both:
restore the loop now, and add a structural guard so the next pyproject.toml change in
any pilot-class repo can't reproduce this class of failure silently.

## Evidence (citation-or-silence)

- DB row, task `4586dcf9-3589-417e-8e0c-6d12cb7efe16` (`long_running:run_claude_pilot_groom`,
  dispatch_class=groom, status=delivered, created 2026-05-18T17:04:17Z, completed
  2026-05-18T17:04:20Z). `result` column contains:
  > Log path: /var/log/claude-pilot/4586dcf9-3589-417e-8e0c-6d12cb7efe16.log
  >
  > claude-pilot FAILED (exit code 1).
  >
  > Stdout:
  >
  > Logs (last 10KB):
  > Traceback (most recent call last):
  >   File "/home/samidarko/.local/bin/claude-pilot", line 4, in <module>
  >     from claude_pilot.cli import main
  >   File "/data/workspace/mika-platform/claude-pilot-py/src/claude_pilot/cli.py", line 23, in <module>
  >     from .agent import run_agent
  >   File "/data/workspace/mika-platform/claude-pilot-py/src/claude_pilot/agent.py", line 21, in <module>
  >     from .permissions import CanUseTool
  >   File "/data/workspace/mika-platform/claude-pilot-py/src/claude_pilot/permissions.py", line 25, in <module>
  >     from .policy import Policy, evaluate, load_policy
  >   File "/data/workspace/mika-platform/claude-pilot-py/src/claude_pilot/policy.p[…]
- Parent (supervisor) task `c3d710a8-958d-4863-bc1f-8b20bd69cb07` ("Groom mika#1196",
  trigger=manual, status=cancelled, completed_at=NULL) — cancelled because the child
  callback delivered a failure result.
- `git -C claude-pilot-py log -1 --format='%h %ai %s' -- src/claude_pilot/policy.py` →
  `39b6ffb 2026-05-18 16:06:14 +0200 feat: deterministic policy-file evaluator (mika#1192)`.
- `git -C claude-pilot-py branch --show-current` → `feat/1192/deterministic-policy-file-replaces-relay-llm-call`
  (HEAD `8e9eeec`, ahead of `main` which is at `86bd3ee` from 2026-05-16). The autonomous
  loop is therefore running uncommitted-to-`main` source via editable install — see
  Concern (4) below.
- `/home/samidarko/.local/share/uv/tools/claude-pilot/lib/python3.14/site-packages/claude_pilot-0.1.0.dist-info/METADATA`:
  ```
  Requires-Dist: claude-agent-sdk==0.1.59
  Requires-Dist: pydantic<3,>=2.6
  Requires-Dist: python-dotenv>=1.0
  ```
  (no `Requires-Dist: pyyaml`). `pyproject.toml` line 12: `"pyyaml>=6.0,<7",`.
- `/home/samidarko/.local/share/uv/tools/claude-pilot/lib/python3.14/site-packages/_editable_impl_claude_pilot.pth`
  → `/data/workspace/mika-platform/claude-pilot-py/src` (editable install pin).
- `/home/samidarko/.local/share/uv/tools/claude-pilot/bin/python3 -c "import yaml"` →
  `ModuleNotFoundError: No module named 'yaml'` (verified live, not from logs).
- `stat /home/samidarko/.local/share/uv/tools/claude-pilot/bin/python3` →
  `2026-05-13 22:25:46.788767156` (venv created 5 days ago; symlink to bin updated
  2026-05-18 12:45:48 by a more recent `uv tool install` that re-used the venv).

## Proximate root cause

`uv tool install --force --editable ./claude-pilot-py` last ran around 12:45:48 today. At
that snapshot, `pyproject.toml` did not yet declare `pyyaml`. `uv` provisioned the venv
with exactly the three declared deps. The `_editable_impl_claude_pilot.pth` file then
pinned the venv's `sys.path` to the source workspace. When mika#1192's commits landed at
16:06 — adding `policy.py` with `import yaml` AND adding `pyyaml>=6.0,<7` to
pyproject.toml — the venv's source resolution picked up policy.py immediately (editable
behavior, expected), but the new dep was NOT installed because `uv tool install` was not
re-run. The first claude-pilot invocation after 16:06 (the 17:04 groom dispatch) executed
the new policy.py code path, hit the missing yaml import, and exited 1 before the file
logger initialized.

## Underlying / structural causes

1. **Editable install decouples source-sync from dep-sync.** The `--editable` flag makes
   source changes feel like "deploy runs on save," but new dependencies require a
   re-run of `uv tool install` to resolve. This is a known editable-install footgun and
   not a bug in uv — but it is a real ergonomic gap in `make deploy` semantics.
2. **`dispatch-lib.sh` has no claude-pilot smoke test.** Lines 709-711 of
   `mika/skills/bundled/_shared/dispatch-lib.sh` already check that the binaries `jq`,
   `mika`, and `claude-pilot` exist on PATH (`command -v`). But `command -v` only
   verifies the binary file is present; it does NOT verify the venv is importable.
   A pre-flight `claude-pilot --help` (or similar fast-failing smoke command) would
   convert "exit 1 in 5s with empty log" into "abort with a clear `claude-pilot venv
   broken: run \`cd <ws> && uv tool install --force --editable ./claude-pilot-py\`"
   diagnostic.
3. **Failure surface is opaque.** Because file-logger init lives inside `cli.py:main()`
   below the failing import, a broken venv produces an empty log file in the standard
   log dir AND a stderr capture buried in the DB `tasks.result` column of a single row.
   The supervisor task (`c3d710a8`) shows `status=cancelled, result=NULL` — nothing in
   that row tells the operator what happened. You have to follow the child-task
   pointer to find the traceback. A pre-flight check short-circuits this, but a longer-
   term improvement is to ensure import-time tracebacks surface in the file log too —
   out of scope for this ticket.

## Acceptance criteria

- **AC1: Immediate runtime restoration.** After applying this plan, `claude-pilot --help`
  exits 0 from the same shell environment the mika-dev OpenRC service uses (Gentoo
  OpenRC with `supervise-daemon`, PATH includes `~/.local/bin`). Verification command:
  `sudo -u <mika-dev-user> bash -lc 'claude-pilot --help' >/dev/null && echo ok` (or
  equivalent under the OpenRC user the service runs as). The follow-up `mika ask --agent
  mika-dev "groom mika issue#1196"` must produce non-empty
  `/var/log/claude-pilot/<task-id>.log` and reach at least the first Claude Code turn
  (verifiable via the DB `tasks.result` excerpt and the log tail).
- **AC2: Structural pre-flight smoke test.** `mika/skills/bundled/_shared/dispatch-lib.sh`
  (the function dispatch path uses, currently `dispatch_claude_pilot` near line 693)
  runs a `claude-pilot --help` (or `--version`) check immediately after the existing
  `command -v claude-pilot` block (lines 709-711). If the smoke command exits non-zero,
  the dispatch aborts with a structured error message that names the most likely fix
  (`uv tool install --force --editable ./claude-pilot-py` from the meta-repo root). The
  error message is written to BOTH stderr and the task result, so a future repro of
  this class shows up in the DB row, not just a missing log file.
- **AC3: Test coverage.** A unit test in `mika/skills/bundled/_shared/` (or wherever
  dispatch-lib's tests live — see Phase 2 step 2 for resolution) verifies:
  (a) the pre-flight smoke test fires before any worktree mutation; (b) a non-zero exit
  from the smoke command aborts dispatch with the expected error string; (c) a zero exit
  passes through to the normal dispatch path. If `dispatch-lib.sh` has no existing test
  harness, the AC degrades to: a shell-script test under `mika/scripts/` or `mika/tests/`
  invoking the function with a stubbed PATH.
- **AC4: Convention reminder.** `mika-platform/CLAUDE.md` (the meta-repo CLAUDE.md, NOT
  the per-repo CLAUDE.md files) gains a one-paragraph note under "Local Dev Environment"
  that says: any change to `claude-pilot-py/pyproject.toml` requires `make deploy`
  before the next autonomous-loop dispatch, because editable install does not auto-sync
  new dependencies. Link to this plan + mika#1200 from the note.
- **AC5: No regression in implementation dispatches.** After the change ships, a fresh
  `mika ask --agent mika-dev "implement mika issue#<some-trivial-ticket>"` (or the
  next ready-labeled implementation dispatch) succeeds through to at least the first
  Claude Code turn, verified the same way as AC1.

## Out of scope (file separate tickets if pursued)

- Switching claude-pilot to non-editable install. Real tradeoff (deterministic deps vs.
  fast iteration) but separate decision; the `feedback_make_deploy_wipes_editable.md`
  memory documents that this was previously the case and was changed. Revisiting is a
  conversation, not a side effect of this fix.
- Making `make deploy` auto-detect pyproject.toml drift and re-install on change. Nice
  ergonomic improvement; out of P1-bug scope. File as a separate ticket if architect
  rates this as load-bearing.
- Making cli-level import-time tracebacks land in the file log (in addition to the
  stderr capture). Improves diagnostics for a future repro class but is not what's
  blocking restoration today.
- Re-fixing mika#1168. PR #1197 is in main and addresses a different failure mode
  (sonnet-classifier refusal + qa-review-allowlist-shadow). This plan does not touch
  that surface.

### Follow-up tickets to file alongside this PR (revised in pass 2 per NF6)

- **Autonomous loop must resolve to a merged-to-main source (or editable-install-at-
  workspace-HEAD must be explicit policy).** The autonomous loop currently dispatches
  through whatever code is on the workspace's `claude-pilot-py` checkout — currently
  the unmerged `feat/1192/deterministic-policy-file-replaces-relay-llm-call` branch.
  This means unreviewed work flows directly into production dispatch. File on
  `mika-platform/` after this PR opens. The implementer ratifies my judgment that this
  is out of scope for the current bug fix but worth its own deliberation.
- **mika#1189 yesterday's pre-#1197 same-signature failure** (if still reproducible
  after Phase 0 restoration). The architect's NF3 confirms tight-scope is correct, but
  if the same empty-handed signature reproduces against a healthy venv post-fix, file
  a separate ticket with fresh diagnostics. Do NOT bundle here.

## Phase 0 Pin — verbatim slices of the load-bearing surfaces

> Added in revision pass 2 to address mika-arch first-pass F1 ("Phase 0 Pin absent"). The
> insertion site and the smoke command both depend on the exact shape of the surrounding
> code; this section pins both so the implementer is not re-deriving the contract.

### `dispatch-lib.sh:693-774` — `dispatch_claude_pilot` function (current HEAD on this branch)

```bash
dispatch_claude_pilot() {
    # --- Diagnostic trace (mika#887) ---
    TRACE_FILE="/tmp/dev-pilot-trace-$$.log"
    # ... trace setup elided ...
    set -x

    # Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
    export PATH="$HOME/.local/bin:$PATH"

    # Dependency checks                                        # ← lines 708-711, the
    command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
    command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
    command -v claude-pilot >/dev/null 2>&1 || { echo "Error: claude-pilot CLI is required but not in PATH" >&2; exit 1; }
    # ↑ INSERT SMOKE TEST IMMEDIATELY BELOW THIS LINE (line 712 in revised file)

    # mika-platform root — base for sub-repo resolution        # ← lines 713-716
    PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_REPO_NAME=$(basename "$PLATFORM_DIR")

    # Initialize callback guard
    CALLBACK_SENT=0

    _parse_input_json
    trap '_dispatch_lib_exit_trap' EXIT
    _validate_inputs

    # ... per-skill dispatch mapping (case statement on $SKILL) ...

    _setup_gh_auth
    _scrub_env
    _set_up_worktree                                          # ← line 769, the worktree
    _detect_plan_on_branch                                    #   creation we want to
    _handle_dry_run                                           #   short-circuit BEFORE
    _run_claude_pilot "$ENTRY_COMMAND"
    _deliver_callback
}
```

**Pinned facts from this slice:**

1. **Execution mode:** dispatch-lib.sh is **executed** (called via `bash ...handlers/run.sh`
   which itself calls `dispatch_claude_pilot` from a sourced lib), not sourced into an
   interactive shell. The existing `command -v` checks use `exit 1`, not `return 1`. The
   new smoke test MUST use the same shape (`exit 1`) to match the surrounding control
   flow. (If it ever moves to a sourced/library shape, all four early-exit branches in
   this block need to be converted together — that's an orthogonal refactor.)
2. **Insertion site is line 712** (immediately after the `command -v claude-pilot` line
   at 711). No PATH manipulation, no env var setup, no logging happens between 711 and
   712 — the smoke test is a pure addition with no displacement.
3. **`_set_up_worktree` is line 769** — 57 lines below the insertion site, separated by
   `_parse_input_json`, trap install, `_validate_inputs`, the per-skill case statement,
   `_setup_gh_auth`, `_scrub_env`. The smoke test fires WELL before any worktree
   mutation. Convergent with F1's "short-circuit BEFORE `_set_up_worktree`" requirement.
4. **`set -x` trace is active (line 703) — but routed to fd 9 / `$TRACE_FILE` only**,
   not stderr. Our new smoke test error message must be written via plain `echo ... >&2`
   to land in `$STDERR_FILE`, which is what the EXIT trap copies into the task `result`
   column. Same pattern as the existing `command -v` errors.

### `cli.py:1-29` — top-level imports (current HEAD of `claude-pilot-py` feat branch)

```python
"""CLI entry point. Port of src/cli.ts."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import signal
import sys
import traceback
from pathlib import Path
from typing import Any, NoReturn

from dotenv import dotenv_values
from pydantic import ValidationError

from .agent import run_agent                                  # ← line 23
from .guardrails import SessionGuardrails, resolve_guardrail_defaults
from .logger import close_file_log, init_file_log
from .permissions import create_permission_handler            # ← line 26
from .types import GuardrailConfig, PilotConfig, ResultJson
from .ui import log_config, log_env
```

**Pinned facts from this slice (resolves F2):**

1. Lines 23 and 26 are **module-top-level imports**, not inside `main()` or
   `_build_parser()`. Python evaluates ALL top-level statements when a module is
   imported — there is no way for the application's argparse `--help` handler to fire
   before these lines execute, because the argparse parser is constructed in
   `_build_parser()` (line 36, below the imports) and parsing happens inside `main()`
   (also below the imports).
2. The shim at `~/.local/bin/claude-pilot` (created by `uv tool install`) starts with
   `from claude_pilot.cli import main` — verified directly:
   ```
   $ head -5 /home/samidarko/.local/bin/claude-pilot
   #!/home/samidarko/.local/share/uv/tools/claude-pilot/bin/python3
   # -*- coding: utf-8 -*-
   import sys
   from claude_pilot.cli import main
   ```
   `from claude_pilot.cli import main` triggers Python to execute the entire `cli.py`
   module body, which runs lines 23 and 26, which transitively trigger the chain
   `agent.py:21 → permissions.py:25 → policy.py:19 → import yaml`. The actual failing
   traceback in `tasks.id=4586dcf9-…` shows exactly this import path. Therefore:

   **`claude-pilot --help` IS a valid smoke test for the import-time failure class.**
   The architect's F2 concern that CLI-framework `--help` short-circuits before app
   imports applies to languages with deferred-import semantics (or to Python tools that
   deliberately lazy-import inside functions), not to a Python CLI whose entry-point
   module imports its dependencies at the top level. The current `cli.py` is the
   latter shape — confirmed by direct read at HEAD.
3. **Future-proofing.** A future refactor of `cli.py` that moves the `from .agent
   import …` family inside `main()` (lazy import) would silently neuter the smoke
   test — `claude-pilot --help` would pass while a real invocation would still
   ModuleNotFound. This is addressed in Phase 1 step 2's revised text: the smoke
   test gets an inline comment pointing to this invariant, and the unit test in
   Phase 2 includes a regression assertion against lazy-import refactors (see Phase 2
   step 2c).

## Plan of work (3 phases — Phase 0 Pin above documents the surfaces; the phases below
are unchanged from the pre-iteration draft except where explicitly noted as "(revised in
pass 2)")

### Phase 0 — Restoration (operator action; no code change)

The runtime is unblocked by:
```bash
cd /data/workspace/mika-platform
uv tool install --force --editable ./claude-pilot-py
```
This re-resolves dependencies against the current `pyproject.toml`, installs `pyyaml` into
the venv, and the next claude-pilot invocation passes its `import yaml` step. **This is
operator action, not part of the PR diff** — but the plan documents it explicitly because
the PR alone does not restore the loop; the operator must run the install before merging
or before AC1's verification command will pass.

Mention in PR body: "Restoration requires `uv tool install --force --editable
./claude-pilot-py` on the host before merging. The PR diff alone does not restore the
autonomous loop."

### Phase 1 — Structural fix in `dispatch-lib.sh` (the PR diff)

1. **Locate the exact existing dependency-check block.** Read
   `/data/workspace/mika-platform/.claude/worktrees/bug-1200-dev-groom-pilot-session-exits-empty/mika/skills/bundled/_shared/dispatch-lib.sh`
   lines 700-720 (the area the investigator's report flagged as the entry of
   `dispatch_claude_pilot`). Confirm the function name and the exact location of the
   `command -v claude-pilot` check before editing.
2. **Add the smoke test immediately after the existing `command -v claude-pilot` line
   (revised in pass 2).** The smoke command is `claude-pilot --help >/dev/null 2>&1`,
   pinned by the cli.py top-level imports established in the Phase 0 Pin above. **Do
   NOT use `--version`** — the parser definition in `_build_parser()` does not register
   a `--version` action (verified by direct read of `cli.py:36-66`), so `--version`
   would fall into argparse's unknown-arg path and exit 2 even on a healthy venv. The
   shape of the new block:
   ```bash
   # claude-pilot venv smoke test (mika#1200): force the import chain that imports
   # yaml to actually execute. Relies on cli.py keeping its imports at module top
   # level — if cli.py is ever refactored to lazy-import .agent / .permissions inside
   # main(), THIS smoke test silently stops detecting the failure class. See
   # mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md § Phase 0
   # Pin / cli.py invariant.
   if ! claude-pilot --help >/dev/null 2>&1; then
       cat >&2 <<'EOF'
   Error: claude-pilot venv is broken — `claude-pilot --help` exited non-zero.
   Most likely cause: pyproject.toml changed in claude-pilot-py without an
   accompanying `uv tool install` to re-sync dependencies. Editable installs pick
   up new source automatically but do NOT auto-install new declared dependencies.

   To restore the loop:
       cd <mika-platform-root> && uv tool install --force --editable ./claude-pilot-py

   Reference: mika#1200 +
   mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md
   EOF
       exit 1
   fi
   ```
   `exit 1` matches the surrounding `command -v` blocks. The error is written to
   stderr via heredoc (single quote on EOF to prevent variable expansion of the
   informational text — the `cat` itself doesn't need expansion). The EXIT trap copies
   the last 10KB of `$STDERR_FILE` into the task `result` column, so the next repro
   of this class lands with a clear diagnostic in the DB row, not just an opaque
   Python traceback.
3. **Capture the error in the task result path.** Confirm that the existing stderr
   capture at line 452 (`2>"$STDERR_FILE"`) and the EXIT trap that copies the last 10KB
   of stderr into the task `result` column already cover this new error path. If not,
   ensure the smoke-test error message is written through the same channel so the next
   repro lands in `tasks.result` with a clear message instead of an opaque traceback.
4. **Verify the smoke test runs before worktree mutation.** Walk the function from
   entry to the `_set_up_worktree` call (around line 769) to confirm the new block
   short-circuits BEFORE any `git worktree add` runs. (The ticket repro showed the
   worktree was created — we want to convert that "branch created but unpopulated"
   half-failure into "no worktree, clear error" instead.)
5. **Run `mika/scripts/verify-pipeline.sh`** (or the equivalent test harness for the
   shared dispatch lib — discover via `ls mika/scripts/`) to confirm the existing
   test surface still passes.

### Phase 2 — Test coverage (AC3)

1. **Discover the existing test harness for `dispatch-lib.sh`.** Search
   `mika/scripts/*.sh`, `mika/skills/bundled/_shared/`, and any `tests/` directories
   for existing dispatch-lib test files. Read the existing tests so the new test
   follows established conventions.
2. **Write the smoke-test coverage (revised in pass 2).** Four cases:
   (a) PATH includes a stubbed `claude-pilot` script that exits 0 → dispatch proceeds;
   (b) PATH includes a stubbed `claude-pilot` script that exits 1 → dispatch aborts
   with the expected error string (assert presence of "claude-pilot venv is broken"
   and "uv tool install --force --editable ./claude-pilot-py" substrings); (c) PATH
   does NOT include `claude-pilot` at all → the existing `command -v` block fires
   (regression check that the new block is ordered AFTER `command -v`, not before).
   (d) **lazy-import regression guard** — a Python smoke test that asserts
   `cli.py` keeps `from .agent import …` and `from .permissions import …` at module
   top level (grep-based check). If a future refactor moves either import inside
   `main()` or any other function body, this test fails with a message pointing back
   to Phase 0 Pin / cli.py invariant. This is the structural backstop for the
   future-proofing concern raised in F2.
3. **If no existing test harness exists for dispatch-lib.sh**, downgrade to AC3's
   degraded form: add a `mika/scripts/dispatch-lib-test.sh` script that exercises the
   function with a stubbed PATH and asserts the expected exits. Run it from a CI hook
   if one already exists; otherwise leave it as a manual-run script and note the gap.

### Phase 3 — Convention reminder (AC4)

Edit `/data/workspace/mika-platform/CLAUDE.md` § "Local Dev Environment" to add one
paragraph after the existing "Sync and deploy are separate concerns" line:

> **pyproject.toml changes in claude-pilot-py require an explicit `make deploy`.**
> Editable installs (the default) pick up new source automatically but do NOT sync new
> dependencies. After any `pyproject.toml` change in `claude-pilot-py/`, run `make
> deploy` from the meta-repo root before the next autonomous-loop dispatch — otherwise
> claude-pilot exits 1 at import time with an empty log file. The structural backstop
> is the `dispatch-lib.sh` smoke test (mika#1200 Phase 1): if `claude-pilot --help`
> exits non-zero, dispatch aborts with a diagnostic naming the restoration command,
> rather than silently leaving a half-built worktree. The convention and the backstop
> work together — the convention prevents the broken state; the backstop catches it
> when the convention fails. See mika#1200 and
> `mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md`.

(Revised in pass 2 to cite both the structural guard and the convention, per
mika-arch first-pass NF5.)

This is documentation, not code — but it is the load-bearing convention reminder that
makes Phase 0 reproducible without re-deriving the diagnosis. Without it, the next
operator who lands a pyproject.toml change will hit this exact failure again.

## Concerns I want the architect to weigh in on (deliberate uncertainties)

1. **Smoke command choice.** `claude-pilot --help` is safe and fast, but if anyone has
   shadowed `--help` with a custom handler that does real work, the smoke test becomes
   load-bearing slow. `--version` is cleaner but depends on the CLI implementing it
   today (verify in Phase 1 step 2). If neither is suitable, falling back to a
   one-liner `python -c "from claude_pilot.cli import main"` invocation using the venv
   python is functionally equivalent but adds a layer of indirection. **Lean: prefer
   `--help`, fall back to `python -c "from claude_pilot.cli import main"` if `--help`
   has side effects.** Architect: is there a cleaner shape I'm missing?
2. **Test harness scope.** If `dispatch-lib.sh` has no existing test surface (likely —
   it's deep shared shell), AC3 might morph into "ship the new test infrastructure"
   which is scope creep. **Lean: ship one targeted script under `mika/scripts/` and
   note the gap; do NOT introduce bats or similar.** Architect: agree, or push for
   bats/equivalent?
3. **CLAUDE.md edit ownership.** The convention reminder belongs in the meta-repo
   CLAUDE.md (workspace-level rule), but the plan + diff live in the `mika/` repo.
   The fix-PR is on `mika/`; the docs change crosses repos. **Lean: ship the docs edit
   as a separate companion PR on `mika-platform/` referencing this PR. Two PRs, one
   ticket.** Architect: agree, or fold the docs change into a single dual-repo
   coordination?
4. **Editable install reconsideration.** The whole class of failure goes away if we
   convert claude-pilot back to non-editable install (per the historical state in
   `feedback_make_deploy_wipes_editable.md`). I deliberately put this out of scope
   because it's a real tradeoff worth deliberating separately. Architect: is there a
   structural reason to bundle that decision with this fix rather than treating it as
   a follow-up?
5. **Feat-branch source in the live loop.** The autonomous loop currently dispatches
   through code from
   `feat/1192/deterministic-policy-file-replaces-relay-llm-call` because that's what
   the workspace checkout is on. This means uncommitted-to-`main` work directly affects
   the autonomous loop. Out of scope for this ticket, but architect: should we open a
   separate ticket for "autonomous loop should never resolve to an unmerged feat
   branch"? Or is editable install at the workspace HEAD an explicit policy choice?

## Dispatch readiness

The work is one mika PR (Phase 1 + Phase 2) plus one optional companion PR on
mika-platform (Phase 3 docs). The mika PR is small (~30 lines of dispatch-lib.sh +
~50 lines of test). The mika-platform companion PR is ~10 lines of docs.

`/mika` from this branch (`bug/1200/dev-groom-pilot-session-exits-empty`) is the canonical
implementation path once GROOMED. Phase 0 restoration is operator action and runs
independently.
