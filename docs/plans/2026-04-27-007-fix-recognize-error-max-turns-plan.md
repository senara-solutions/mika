---
title: "fix(self-dev): recognize error_max_turns as recover-work verdict, not retry"
type: fix
status: active
date: 2026-04-27
origin: senara-solutions/mika#838
---

# Plan — `error_max_turns` is a recover-work verdict, not a retry verdict (mika#838)

**Issue:** [mika#838](https://github.com/senara-solutions/mika/issues/838)
**Branch:** `fix/838/recognize-error-max-turns-as-recover-work`
**Type:** fix (mika-dev verdict-handler grounding gap)
**Labels:** enhancement, p1-important

## Problem (per issue body)

When claude-pilot terminates with `[guardrail] error_max_turns: SDK limit reached after N turns`, the implementation may be complete but the closing steps (push + PR + `/ce:compound`) are skipped. mika-dev currently treats this as a generic pipeline failure and proposes redispatch — which would burn another claude-pilot session redoing the same work.

**Witnessed (mika#659, 2026-04-26):** claude-pilot ran 201 turns / $22.98 / 40 minutes, completed implementation + `/ce:review` + review-fix commits (9 commits on local branch `feat/659/dashboard-time-range-filter`, tip `e5bb119d`), hit max_turns at `/ce:compound`. mika-dev's heartbeat reported "pipeline produced no commits after 1 attempt" — true at origin (never pushed) but false at the local repo layer.

**Source:** claude-pilot log at `/var/log/claude-pilot/1027bc40-7bbf-4d5d-8e87-8f771022ff87.log` (470 lines).

**Grounding gap:** mika-dev checked GitHub for evidence (`gh pr list --head <branch>`) and concluded none existed, never queried the local branch ref. Same failure shape as `feedback_mika_dev_llm_fabricates_tool_errors.md` — claiming downstream state without verifying local state first.

## Audit results (verified during planning)

### How mika-dev handles pipeline failures today

- **`mika/skills/bundled/self-dev/system_prompt.md`** owns the dispatch loop and the post-callback close-out logic. It tracks `pipeline_retry_count` in task metadata and decides whether to redispatch or escalate.
- **`mika/skills/bundled/self-dev-webhook-qa/system_prompt.md`** handles QA verdicts (pass/hold/block[ci]/block[ac]). The `block[ac]` path was added in mika#825 with explicit operator-routing semantics.
- **claude-pilot callback delivery:** the `run_claude_pilot` tool returns the subprocess result (stored in `tasks.result` as TEXT) when the long-running task completes. For mika#659, that field reads literally: `HANDLER CRASH (exit code 128). Script failed before building result.` was the case at exit 128. For max_turns, the log line `[guardrail] error_max_turns: SDK limit reached after 201 turns` is the marker.

### What the existing failure-class taxonomy looks like

mika#825's verdict-handler split fan-in/fan-out into named classes: `pass`, `hold[review]`, `block[ci]` (auto-retry), `block[ac]` (operator review, no retry), `block[security/pipeline]` (notify Vincent, pause sprint). The structural lesson: each verdict has a corresponding handler action, and adding a new failure class means adding both the recognition step and the handler step.

`error_max_turns` is currently absorbed into the generic "pipeline failed" path — handled as if claude-pilot crashed, not as if it succeeded-but-skipped-closeout. That's the exact thing this plan fixes.

### Where the local-branch query needs to happen

mika-dev's worktree is the meta-repo at `/data/workspace/mika-platform`. The sub-repo `mika` lives at `/data/workspace/mika-platform/mika`. Local branch refs are under `/data/workspace/mika-platform/mika/.git/refs/heads/<branch>`. The `git log <branch>` query that surfaces the unpushed commits is:

```bash
git -C /data/workspace/mika-platform/mika log --oneline origin/main..<branch>
```

Returns 0 lines if no local commits exist, ≥1 if they do. This is the ground-truth check mika-dev should run BEFORE concluding "no commits produced."

## Approach

Three changes — one structural recognition, one named handler action, one runbook codification.

### Change 1 — Add `error_max_turns` recognition + grounding check + handler in self-dev

**File:** `mika/skills/bundled/self-dev/system_prompt.md`

**Per architect Finding 1 (first-pass, HARD BLOCKER):** the `recover_unpushed_work` HANDLER lives in `self-dev/system_prompt.md`, not in `self-dev-webhook-qa/system_prompt.md`. The trigger is callback-side (claude-pilot returns `error_max_turns` marker via `tasks.result`), which is processed by self-dev's post-callback close-out logic. Placing the handler in webhook-qa would be a layering inversion — the handler must live where the signal originates. The webhook-qa file may reference the verdict class by name (for taxonomy awareness), but the recognition + grounding-check + send_message + metadata-write logic all live in self-dev.

**Verdict-class registry** (added at top of self-dev/system_prompt.md, per architect's recommendation): named source-of-truth for all verdicts, with the originating signal annotated for each. This consolidates the taxonomy without co-locating unrelated handlers.

In the post-callback section, add the pipeline-result-classification step:

> **Pipeline result classification (two-branch trigger per architect Finding 2):**
>
> **Primary trigger (marker-match):** When the callback's `tasks.result` field contains the literal substring `error_max_turns` (claude-pilot's max-turns guardrail produces `[guardrail] error_max_turns: SDK limit reached after N turns`), run the grounding check below.
>
> **Secondary trigger (conservative heuristic, time-bounded):** When ALL of the following hold simultaneously:
> - `tasks.result` is NULL or empty (subprocess output not captured)
> - No PR exists on origin for the task's branch (`gh pr list --head <branch>` returns `[]`)
> - Task status is currently `in_progress`
> - Task `created_at` is more than **2 hours** ago (per architect second-pass: 30 minutes can produce false positives on legitimate long-running pipelines doing complex compound or large migrations; 2 hours is beyond any reasonable pipeline run, so the bound is unambiguously stale)
> - Task `updated_at` is more than 30 minutes ago (recently-updated `updated_at` on an `in_progress` task means the pipeline is still making progress; only stale-progress rows trigger)
>
> Run the grounding check below. **The secondary trigger is conservative and treats only stale-in-progress-with-no-result-and-no-PR cases.** Active pipelines (including ones that legitimately run > 30 minutes) are NOT triggered because their `updated_at` will be recent.
>
> **Grounding check (the load-bearing logic):**
>
> ```bash
> git -C <repo-path> log --oneline origin/main..<branch>
> ```
>
> Branch resolution: from the task's `metadata.claude_pilot.branch` (set when claude-pilot dispatch succeeds) or, if absent, from the issue body's `> - **Branch:**` callout (per the #825 plan-on-branch convention).
>
> **Decision tree:**
> - **`git log` returns ≥ 1 commit:** verdict is `recover_unpushed_work`. Apply the handler below. Do NOT redispatch claude-pilot.
> - **`git log` returns 0 commits:** verdict is genuine no-progress failure. Existing pipeline-retry-or-escalation path applies (no change).
>
> **Handler when verdict is `recover_unpushed_work` (atomicity per architect Finding 3 — write metadata BEFORE send_message):**
>
> 1. **First**, write `unpushed_recovery_pending: true` to `tasks.metadata` JSON via `update_task_status` (status stays `in_progress` — the work is recoverable, not failed).
> 2. **Then**, emit `send_message` to operator with the structured payload (per architect Finding 6 — required fields named):
>
>    ```json
>    {
>      "verdict": "recover_unpushed_work",
>      "task_id": "<uuid>",
>      "branch": "<branch-name>",
>      "tip_sha": "<commit-sha>",
>      "commit_count": <int>,
>      "turn_count_at_exhaustion": <int>,
>      "claude_pilot_log_path": "/var/log/claude-pilot/<subprocess-task-id>.log",
>      "suggested_recovery_command": "WT=$(git -C /data/workspace/mika-platform/mika worktree list --porcelain | awk -v b=<branch> '/^worktree /{p=$2} /^branch refs\\/heads\\/'b'$/{print p}'); [ -z \"$WT\" ] && { WT=/data/workspace/mika-platform/.claude/worktrees/<sanitized-branch>/mika; git -C /data/workspace/mika-platform/mika worktree add \"$WT\" <branch>; }; cd \"$WT\" && git rebase origin/main && git push origin <branch> && gh pr create --repo senara-solutions/mika"
>    }
>    ```
>
>    The metadata-first ordering ensures: if `send_message` fails after metadata is written, the operator can be re-notified on the next heartbeat (the `unpushed_recovery_pending` flag is durable, message history is searchable). If the order were reversed, a `send_message` failure would leave no durable record that the recovery verdict was reached.
>
> 3. Do NOT increment `pipeline_retry_count`. The pipeline didn't fail — it ran out of turns mid-closeout. Retry is the wrong frame.
> 4. Do NOT call `run_claude_pilot` again for this task.

### Change 2 — Reference `recover_unpushed_work` in the webhook-qa verdict registry (taxonomy awareness only)

**File:** `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md`

**Per architect Finding 1 (first-pass):** the handler does NOT live here — it lives in `self-dev/system_prompt.md` (Change 1) where the originating signal (`tasks.result`) is processed. This file gets a one-paragraph reference for taxonomy completeness:

> **Verdict class `recover_unpushed_work`** (callback-originated, not webhook-originated) is handled in `self-dev/system_prompt.md`. It fires when claude-pilot returns the `error_max_turns` marker AND the task's branch has unpushed local commits. The webhook handler does NOT apply pipeline-retry, AC-routing, or QA-routing logic to tasks bearing `unpushed_recovery_pending: true` in their metadata — those tasks are awaiting operator recovery, not webhook resolution.

This addition prevents the webhook handler from accidentally applying its retry/escalation paths to a recovery-pending task that happens to receive a webhook event (e.g., a stale check_suite event referring to the unpushed branch).

Net diff: ~10 lines.

### Change 3 — Document the recovery runbook in `docs/solutions/best-practices/`

**File:** `mika/docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md` (new)

A short compound doc capturing:
- **The failure shape:** max_turns mid-`/ce:compound`. claude-pilot completed implementation, hit the 200-turn ceiling at the closing steps.
- **The grounding rule:** `git log origin/main..<branch>` before declaring no commits. The local branch ref is the ground truth, not GitHub origin.
- **Failure-recognition decision tree (per architect Finding 5 — preserved as institutional knowledge survivable across prompt rewrites):**
  1. `tasks.result` contains `error_max_turns`? → run grounding check.
  2. Grounding check returns ≥ 1 commit? → verdict `recover_unpushed_work`. Apply handler (metadata-write THEN send_message).
  3. Grounding check returns 0 commits? → genuine no-progress failure. Existing path.
- **The recovery procedure:** rebase onto origin/main, resolve conflicts, push, open PR — bypassing claude-pilot.
- **Out-of-scope guardrails (per architect Finding 4 — name them so future grooming doesn't re-derive):**
  - `error_stall_threshold` — LLM produced no output. Likely infinite loop, not "did work then ran out of budget." Recovery doesn't apply.
  - `error_idle_timeout` — process abandoned mid-run. Different failure semantics; manual investigation needed.
  - `error_empty_response_threshold` — successive empty responses. Recovery is provider-side, not work-recovery.
  - Each guardrail has its own failure semantics; uniform handling would produce incorrect guidance. Treat each separately as needs surface.
- **Citation:** mika#659 as the founding incident. mika#838 as the structural fix.

This sits alongside `feedback_mika_dev_llm_fabricates_tool_errors.md` (memory) and the mika#825 plan as a related cluster of "verify local state before claiming downstream state" patterns.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/skills/bundled/self-dev/system_prompt.md` | +~25 lines: pipeline-result-classification step with grounding check + `recover_unpushed_work` verdict path |
| 2 | `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md` | +~20 lines: new verdict class with no-retry / no-pipeline_retry_count-increment / operator-send_message handler |
| 3 | `mika/docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md` (new) | +~60 lines: compound doc with failure shape, grounding rule, recovery procedure |

Estimated diff: ~100 lines across 3 files. Skill-prompt change, no code change.

## Tests

These are skill-prompt changes. Per architect Finding 7 — the standing three-command discovery discipline doesn't apply to prompt layers; behavioral verification is the analog. Three named test cases:

1. **Regression test (positive):** Simulate `tasks.result = "[guardrail] error_max_turns: SDK limit reached after 201 turns"` with a branch that has 9 local commits ahead of `origin/main`. Confirm:
   - mika-dev runs `git log origin/main..<branch>` and observes 9 commits.
   - Verdict emitted is `recover_unpushed_work`, NOT `pipeline_retry`.
   - `tasks.metadata.unpushed_recovery_pending` is set to `true` BEFORE `send_message` fires.
   - `send_message` payload contains all required fields (task_id, branch, tip_sha, commit_count, turn_count_at_exhaustion, claude_pilot_log_path, suggested_recovery_command).
   - `pipeline_retry_count` is NOT incremented.
   - `run_claude_pilot` is NOT called again for this task.

2. **Negative test:** Same `error_max_turns` marker but `git log` returns 0 commits (genuine no-progress crash). Confirm:
   - Verdict is NOT `recover_unpushed_work`.
   - Existing pipeline-retry-or-escalation path applies (no behavioral change vs today).

3. **Miss test (different guardrail):** `tasks.result` contains `error_stall_threshold` instead of `error_max_turns`. Confirm:
   - Pipeline-result-classification step does NOT trigger the grounding check.
   - Verdict is NOT `recover_unpushed_work`.
   - Existing failure-handling path applies unchanged.

Plus the standing build + doc-lint:

4. **Build:** `cargo check -p mika-agent` — confirms skill bundle compiles via `build.rs` discovery.
5. **Doc lint:** `mika-doc-audit` should accept the new compound doc; verify on the worktree before PR.

**Eval harness coverage** (out of scope per Out of scope below): an `EvalHarness` test that exercises the post-callback turn against a `MockLlmProvider` returning the max_turns marker would lock the contract structurally. Defer to a follow-up if/when self-dev gets eval coverage; this plan does not add the eval scaffolding.

## Acceptance criteria

- [ ] `mika/skills/bundled/self-dev/system_prompt.md` includes the pipeline-result-classification step with the `git log origin/main..<branch>` grounding check, documented as a precondition to declaring "no commits produced."
- [ ] `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md` declares `recover_unpushed_work` as a named verdict class, with explicit "no auto-retry" and "send_message to operator" handlers, following mika#825's `block[ac]` precedent.
- [ ] `tasks.metadata` schema is extended (in prompt-doc form) with the optional `unpushed_recovery_pending: bool` field; mika-dev's close-out logic sets it on the recover_unpushed_work path.
- [ ] `mika/docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md` exists and cites mika#659 as the founding incident.
- [ ] `cargo check -p mika-agent` succeeds.
- [ ] `mika-doc-audit` passes on the new compound doc.

## Out of scope

- **Increasing claude-pilot's `maxTurns` guardrail above 200.** Symptom-side fix; doesn't close the recovery gap. Separate ticket if pursued.
- **Detecting `/ce:compound started` in the log and short-circuiting compound when turn budget is low.** Requires claude-pilot self-awareness; orthogonal change. Separate ticket.
- **Building an automatic recovery dispatcher** (a tool that performs rebase + push + PR open without operator intervention). Per mika#825's "operator-routing on `block[ac]`" precedent, this verdict is operator-routed by design — automating recovery would duplicate the manual judgement step. Separate ticket if/when patterns prove repeatable.
- **Adding eval harness coverage for max_turns** (per Tests §3). Self-dev doesn't have eval coverage today; adding it is a separate scaffolding ticket.
- **Generalizing to other guardrails** (`stallThreshold`, `emptyResponseThreshold`, `idleTimeout`). The same recover-work shape *might* apply to those, but each has different failure semantics. Confine the change to `error_max_turns`; revisit if other guardrails surface the same recovery need.

## Risks

| Risk | Mitigation |
|---|---|
| The `git log origin/main..<branch>` check assumes the branch ref is in the meta-repo's bare repo. If grounding-time worktrees were removed or never created, the ref might not exist locally | Plan's recover_unpushed_work verdict gracefully degrades: if `git log` returns 0 lines because the branch doesn't exist locally either, the verdict falls back to `pipeline_failed` (existing path). The check is additive, not load-bearing. |
| mika-dev's LLM may still fabricate "no commits" without actually running the grounding check | Same fabrication-class failure as `feedback_mika_dev_llm_fabricates_tool_errors.md`. Mitigation is structural: the prompt explicitly names the grounding-check step as a hard precondition with an output format the LLM must surface (commit count + tip SHA). If the LLM skips it, mika-dev's audit logs reveal the gap and the operator can correct via prompt iteration. |
| `recover_unpushed_work` and `block[ac]` could be conflated by the LLM since both are operator-routed | Each has a different trigger marker (max_turns log line vs. AC-violation pattern in QA review) and a different metadata flag. Naming them distinctly in §5.x of the verdict-class table prevents drift. |
| Adding the failure class to `self-dev-webhook-qa` rather than `self-dev` core might confuse the layering — max_turns is dispatched via callback, not webhook | Acceptable. The webhook handler doc already centralizes verdict-class definitions; recover_unpushed_work belongs there for taxonomy completeness even though the trigger is callback-side. Structural layering is consistent with mika#825. |

## Sequencing

1. **Change 1 first** (self-dev pipeline-result classification + grounding check). Standalone, self-dev's dispatch loop only.
2. **Change 2 second** (verdict-class addition in webhook handler). Depends on Change 1's verdict naming convention.
3. **Change 3 last** (compound doc). Documents the shipped behavior.
4. **Build + doc-audit verification.** `cargo check -p mika-agent` + `mika-doc-audit` on the worktree.
5. **Open PR** referencing #838 with mika#659 cited as the founding incident.

## Verification

```bash
# Confirm pipeline-result classification step exists in self-dev prompt
grep -c "error_max_turns" mika/skills/bundled/self-dev/system_prompt.md  # → ≥ 1
grep -c "recover_unpushed_work" mika/skills/bundled/self-dev/system_prompt.md  # → ≥ 1
grep -c "git log .* origin/main\.\." mika/skills/bundled/self-dev/system_prompt.md  # → ≥ 1 (the grounding check)

# Confirm verdict class declared in webhook handler
grep -c "recover_unpushed_work" mika/skills/bundled/self-dev-webhook-qa/system_prompt.md  # → ≥ 1
grep -c "no auto-retry\|do not auto-retry\|do NOT auto-retry" mika/skills/bundled/self-dev-webhook-qa/system_prompt.md  # → ≥ 1

# Confirm compound doc exists
test -f mika/docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md && echo OK
grep -c "mika#659" mika/docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md  # → ≥ 1

# Build verification
cargo check -p mika-agent
```

## Discovery items (verified during planning)

1. **claude-pilot's `maxTurns=200` guardrail is in the runtime config, not the system prompt.** Verified by reading `/var/log/claude-pilot/1027bc40-...log` line 2: `[guardrails] maxTurns=200 stallThreshold=5 emptyResponseThreshold=5 idleTimeout=300.0s`. The threshold itself is out of scope for #838 (named in Out of scope as a separate ticket).
2. **The `tasks.result` column captures the claude-pilot subprocess output verbatim.** For mika#659 it read literally `HANDLER CRASH (exit code 128). Script failed before building result.` That's the same column the new classification step inspects.
3. **mika#659 is the founding incident.** The recovery for #659 itself is out of scope (separate dispatch happening in parallel) but its data is the citation source.
4. **mika#825's verdict-handler precedent provides the architectural template.** `block[ac]` was added with the same shape (named class, no auto-retry, operator routing). `recover_unpushed_work` follows it exactly.
5. **Pre-commit discovery discipline applied:** the `git log` grounding check is the same shape as the "verify before claiming" pattern from `feedback_smoke_before_claiming_done.md` and `feedback_verify_before_claiming.md` memories. The fix is making that discipline mechanical rather than prompt-only.
