---
module: dev-pilot
tags: [dispatch, handler, silent-failure, exit-trap, set-e]
problem_type: dispatch-reliability
category: dev-loop
date: 2026-04-29
status: open-investigation
related_tickets: [mika#861, mika#879]
---

# dev-pilot handler silent exit-0: callback delivers `HANDLER CRASH (exit code 0)` without claude-pilot ever launching

## Symptom (fingerprint)

mika-dev dispatches `run_claude_pilot` for a ticket. The `long_running:run_claude_pilot` callback subtask is created, then transitions to `delivered`/`completed` ~3 seconds later. Inspecting the row:

- **`tasks.result`** column verbatim: `"HANDLER CRASH (exit code 0). Script failed before building result."`
- **No log file** at `/var/log/claude-pilot/<callback_subtask_id>.log` (claude-pilot was never invoked).
- **No `long-running exec failed` warning** in `/var/log/mika/server.log` (the executor only logs warnings on non-zero exit; exit 0 is silent on the executor side).
- **mika-dev's callback session emits a clean assistant message** describing the failure and asking for retry — confirming mika#870's `callback_terminal_action` guard works correctly (this is the success story; the bug below is the upstream cause).

## Mechanism

The dev-pilot handler at `mika/skills/bundled/dev-pilot/handlers/run.sh` runs with `set -e` and an EXIT trap:

```sh
deliver_callback() {
    _EXIT_CODE=$?
    # ... try to recover RESULT from STDOUT_FILE / STDERR_FILE ...
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    # ... mika ask --task-id $TASK_ID --task-complete -- "$RESULT" ...
}
trap deliver_callback EXIT
```

When the trap fires with `_EXIT_CODE=0` and `RESULT` empty, the handler:

1. Exited cleanly (no `set -e` propagation killed it; not signal-killed).
2. Did NOT reach the result-building code after claude-pilot launch (line ~365+).
3. Did NOT enter either of the two visible `exit 0` paths (the dry-run branches at `run.sh:312` and `run.sh:331`) — verified this turn against tonight's incidents.

There is therefore an unidentified silent-exit-0 path in `run.sh` between input parsing (~line 90) and claude-pilot launch (line 343), affecting some worktree states but not others.

## Evidence (from 2026-04-28 → 2026-04-29 overnight sprint)

| Ticket | Branch | Outcome | Callback subtask |
|---|---|---|---|
| mika#879 (×2) | `feat/879/skills-mika-arch-add-milestone-grooming` | crashed | `8480a256-…`, `d1881951-…` |
| mika#861 (×1) | `ci/861/verify-pipeline-inherit-documentation` | crashed | `80bac586-…` |
| mika#862 | `engine/862/asserted-unavailability-endturn-guard` | shipped | `9e1f79a8-…` |
| mika#863 | `engine/863/quoted-resource-pre-fetch-guard-fetch` | shipped | `94eed7e2-…` |
| mika#864 | `engine/864/required-suffix-line-endturn-guard-for` | shipped | `c27670a9-…` |
| mika-platform#62 | `ci/62/pr-creation-hook-block-pr-open-when` | shipped | `6bf89875-…` |

Production tool inputs (`tool_calls.input` for the `run_claude_pilot` calls) are byte-identical in shape between crashed and shipped dispatches — 90 chars, `{"prompt":"<repo>#<n>","skill":"dev-pilot","task_id":"<uuid>"}`. No `dry_run` field anywhere.

## Hypothesis (unverified, ranked)

1. **Worktree-prep step quirk.** The block at `run.sh:240-247` (`git worktree remove --force` + fallback `git worktree add` ladder) has nuanced error suppression. A specific worktree state where `git worktree add -b "$BRANCH" "$WORKTREE_DIR" origin/main 2>/dev/null` AND `git worktree add "$WORKTREE_DIR" "$BRANCH"` (no error suppression) both succeed silently without actually setting up a usable worktree could leave the script in a state where later steps return 0 but never reach claude-pilot. Needs `set -x` trace from a reproducer.
2. **`derive-branch-name` or `derive-worktree-path` returning unexpected output.** Both scripts are invoked via `$(...)` command-substitution. If either prints to stdout but exits 0 with output that fails a later check (e.g., empty BRANCH or WORKTREE_DIR), the script may continue silently and exit 0 at end-of-script.
3. **Rebase-or-abort guard edge case.** `BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)` could yield non-numeric output that `[ "$BEHIND" -gt 0 ]` interprets unexpectedly under POSIX `sh` semantics.

The dry-run paths at lines 312 and 331 were ruled out: production inputs don't include `dry_run`, and an explicit dry-run reproduction confirmed the dry-run path DOES delete the worktree (which would have been visible if it had fired in production — the failed worktrees were intact post-crash).

## Diagnostic recipe (for next investigation)

When a `run_claude_pilot` dispatch fails with this fingerprint:

```sql
SELECT id, status, result, substr(updated_at, 1, 19) AS updated
  FROM tasks
 WHERE label = 'long_running:run_claude_pilot'
   AND result LIKE 'HANDLER CRASH (exit code 0)%'
 ORDER BY updated_at DESC LIMIT 5;
```

Then:

1. **Confirm no claude-pilot log:** `ls /var/log/claude-pilot/<subtask_id>.log` — if absent, you're in this pattern.
2. **Confirm no executor warning:** `grep <subtask_id> /var/log/mika/server.log | grep "long-running exec failed"` — if zero results, the handler exited 0.
3. **Inspect the parent's worktree:** the failed dispatches keep their groomed worktrees (engine doesn't clean them up — that's mika-platform-refresh's job and only after PR merge). `git -C <worktree>/mika status -sb` and `git log --oneline origin/main..HEAD` to compare with a known-good worktree's state.
4. **Add tracing to the handler:**

```sh
# Inserted at top of run.sh after `set -e`:
exec 3>>/var/log/dev-pilot-trace-$$.log
BASH_XTRACEFD=3
set -x
```

Redeploy via `make deploy`. Trigger the failed dispatch again. Inspect `/var/log/dev-pilot-trace-*.log` to see exactly which line is the last one logged before exit. The PID-suffixed filename ensures concurrent dispatches don't overwrite each other.

5. **Defensive enhancement (independent of root cause):** the EXIT trap's fallback could capture the trace tail automatically. Worth a follow-up ticket once the root cause is known: pipe stderr to a per-PID trace file unconditionally, include the last 50 lines in `RESULT` on `_EXIT_CODE=0 && RESULT=""`. Costs ~4KB per dispatch but every future silent crash gets immediately diagnosable.

## Why this matters: structural enforcement is working

The dispatch-reliability fixes from mika#870 (`callback_terminal_action` guard) and mika#871 (engine reaper) shipped earlier 2026-04-28 caught **every** handler-crash in the overnight sprint cleanly:

- The callback turn produced `update_task_status` (parent → `blocked`) + `send_message` ("Reply 'retry mika#XXX' to re-dispatch"). Operator was notified, no silent loops, no leaked `in_progress` parents.
- The reaper never had to fire because the callback turn always reached terminal state. Safety net intact for the rarer crash-mid-callback class.

This handler-crash pattern is the FIRST thing the new guards caught in production. The bug existed before — the visible difference is operator awareness. Pre-#870, this pattern silently ate dispatches; post-#870, every occurrence emits an actionable message in under 60 seconds.

## Recurrence record

| Date | Tickets | Notes |
|------|---------|-------|
| 2026-04-29 | mika#879 (×2), mika#861 (×1) | Investigated; root cause not isolated; diagnostic recipe filed (this doc) |

If a fourth instance lands, the structural fix moves from sentinel to imperative — at N=4 across distinct branches, the `set -e` + `2>/dev/null` patterns in `run.sh` should be hardened end-to-end (e.g., wholesale replacement with `set -euo pipefail` + explicit error handling at every command-substitution boundary).

## References

- `mika/skills/bundled/dev-pilot/handlers/run.sh` — the handler script (441 lines, `set -e`, EXIT trap with fallback message)
- `mika/crates/mika-agent/src/skills/executor.rs:870-1043` — the `spawn_long_running_exec` that runs the handler
- `mika/docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — the N=3+ → CI gate methodology that motivated #870/#871
- `feedback_prompt_enforcement_fragile.md` (mika-platform memory) — the meta-rule the dispatch-reliability fix family ships
- `mika-platform/docs/logs/2026-04-29 - Overnight Sprint (mika#862-864 + mika-platform#62).md` — full timeline of the sprint that surfaced this pattern
