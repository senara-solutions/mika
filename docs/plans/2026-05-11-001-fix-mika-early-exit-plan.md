---
title: "fix: /mika slash command exits at 'Proceeding to /ce:work' without invoking it"
type: fix
status: active
date: 2026-05-11
---

# fix: /mika slash command exits at 'Proceeding to /ce:work' without invoking it

## Overview

When `/mika <ref>` is dispatched against a ticket with a groomed plan-on-branch, the model narrates "Confidence check passed -- Proceeding to /ce:work" and calls `end_turn` without ever invoking `/ce:work` as a tool call. Zero commits are produced, `validate_head_unchanged` fires `PIPELINE FAILURE`, and `self-dev` retries with the identical prompt -- reproducing the same failure at ~$0.6-0.8/attempt.

This fix is **mitigation, not root-cause**. The root-cause fix is structural: `dispatch-lib.sh` should detect plan-on-branch and pass `--command "/ce:work"` instead of `--command "/mika"`, eliminating the model's opportunity to narrate instead of act (tracked as follow-up in Unit 3). The mitigation consists of (1) prompt hardening in `/mika` to reduce narrate-then-exit probability and (2) a targeted recovery-mode retry in `self-dev` for when the prompt hardening fails.

## Problem Frame

Observed twice on 2026-05-11 evening (v0.12.4):

| Ticket | Session | Turns | Cost | Outcome |
|--------|---------|-------|------|---------|
| mika-platform#100 | `4c5bb02a-...` | 12 | $0.58 | Stopped at "Confidence check passed -- Proceeding to /ce:work" |
| mika#1066 (2nd attempt) | `4594a1fb-...` | 22 | $0.83 | "DONE" with zero commits |

The model follows the `/ce:plan` SKILL.md fast-path (line 70-72): when it finds an existing complete plan, it "short-circuit[s] to Phase 5.3 (Confidence Check and Deepening)." The confidence-check narrative produces a "satisfied stopping" signal -- the model treats "Proceeding to /ce:work" as a terminal output rather than a precursor to an actual tool call.

This failure class is well-documented in the codebase:
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` -- prompt-level rules drift under cognitive load
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` -- 10+ recurrences of prompt enforcement decay
- `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md` -- "Proceeding to X" is intent, not completion

The usual remedy (structural engine guards) is NOT available here: `/mika` runs inside Claude Code via claude-pilot, not inside Mika's agent loop. The slash command prompt IS the only enforcement layer for Unit 1. This constraint is precisely WHY the structural fix (Unit 3: `dispatch-lib.sh --command "/ce:work"`) is the right long-term path -- if dispatch-lib passes `/ce:work` directly, the model doesn't need to "decide" to invoke it.

## Requirements Trace

- R1. `/mika` prompt must enforce that `/ce:work` is invoked as a Skill tool call, not narrated
- R2. Plan-on-branch detection must prohibit `/ce:plan` invocation and self-directed confidence checks
- R3. Pipeline-failure retry on HEAD-unchanged must use a modified prompt that forces `/ce:work` directly
- R4. Retry cost should not exceed ~$1.50 total (current: ~$1.40-2.40 for 2 blind retries)
- R5. Structural follow-up tracked as a ticket for `dispatch-lib.sh` command override

## Scope Boundaries

- The `/ce:plan` SKILL.md Phase 5.3 fast-path is NOT modified -- it is third-party compound-engineering plugin code
- No Rust engine changes -- the `validate_head_unchanged` check in `dispatch-lib.sh` already works correctly
- No changes to `dev-pilot/handlers/run.sh` or `dispatch-lib.sh` in this fix -- the structural command override is a follow-up (Unit 3)

### Deferred to Separate Tasks

- Regression test (AC #4 from issue): requires eval harness infrastructure for slash command behavior testing. First autonomous dispatch post-merge serves as manual regression validation.
- Structural fix in `dispatch-lib.sh` (Unit 3): tracked as follow-up ticket

## Context & Research

### Relevant Code and Patterns

- `.claude/commands/mika.md` lines 14-28: Plan-on-branch detection logic
- `.claude/commands/mika.md` line 74: "skip when a plan-on-branch was detected above" -- current weak enforcement
- `skills/bundled/self-dev/system_prompt.md` lines 180-185: Pipeline failure retry logic
- `skills/bundled/_shared/dispatch-lib.sh` lines 348-355: `validate_head_unchanged` detection
- `skills/bundled/_shared/dispatch-lib.sh` line 459: `ENTRY_COMMAND="/mika"` hardcoded from skill mapping

### Institutional Learnings

- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` -- merge enforcement into existing steps, avoid parallel mandatory blocks
- `docs/solutions/best-practices/slash-command-self-containment-2026-04-26.md` -- slash commands must be self-contained for headless execution
- `docs/solutions/runtime-errors/prose-style-tool-call-leaks-not-detected-2026-04-20.md` -- models emit tool names as prose text instead of structured calls
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` -- models narrate completion without executing

### Pinned Source

#### `.claude/commands/mika.md` — Plan-on-branch detection (lines 22-26, current)

```markdown
- **If the callout is present AND `<path>` exists in the worktree** (verify with `test -f <path>`): set `PLAN_PATH=<path>` and **skip Step 1 (`/ce:plan`)** in the Pipeline below. Run `/ce:work <PLAN_PATH>` directly (Step 2). Frame the prompt explicitly so claude-pilot consumes the plan as the contract:

  > This plan was groomed and committed by the architect. It is the contract for this implementation. If any acceptance criterion is unclear or unsatisfiable (e.g., conflicts with an existing parser, breaks a downstream consumer, depends on something that doesn't exist), **send_message** to mika-dev surfacing the ambiguity — do not silently scope-reduce. Do not write a new plan file in `docs/plans/`. The existing plan-on-branch is the single source of truth.

- **If the callout is absent OR `<path>` does not exist in the worktree:** fall back to the current flow — run `/ce:plan $ARGUMENTS` (Step 1) followed by `/ce:work` (Step 2).
```

#### `.claude/commands/mika.md` — Pipeline Steps 1-2 (lines 74-75, current)

```markdown
1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments) — **skip when a plan-on-branch was detected above; jump straight to Step 2 with `/ce:work <PLAN_PATH>`**
2. `/ce:work` — when a plan-on-branch was detected, invoke as `/ce:work <PLAN_PATH>` with the contract framing from the Issue linking section above
```

#### `skills/bundled/self-dev/system_prompt.md` — Pipeline failure handler (lines 180-185, current)

```markdown
**On pipeline failure (callback contains "PIPELINE FAILURE:"):**

1. Extract metadata (Session, Cost, Turns, Duration) from the lines after the PIPELINE FAILURE prefix.
2. Check `pipeline_retry_count` in task metadata (default 0). Call `check_task(task_id)`.
3. If `pipeline_retry_count >= 2`: escalate — notify Vincent "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries." Proceed to Step 6 with `blocked`.
4. If retries remain: notify Vincent "Pipeline produced no commits for {repo}#{issue_number} — retrying ({n}/2)." Call `update_task_status` with same status `in_progress` and `metadata: {"pipeline_retry_count": <current + 1>}`. Verify persistence via `check_task`. Then call `run_claude_pilot` with the same `repo#number` and `task_id`. If the call returns `{"status": "deferred", "deferred": true}`, the retry has been automatically enqueued and will fire as a fresh session — do NOT retry again. Proceed to Step 6 with status `in_progress` and note "pipeline retry deferred — engine will auto-dispatch when dispatch slot is free."
```

## Key Technical Decisions

- **Prompt hardening as mitigation, structural fix as root cause (F3):** Structural guards (EndTurn post-conditions) are the preferred enforcement layer per institutional learnings, but they run in Mika's agent loop, not in Claude Code. The `/mika` command prompt is the only enforcement layer available inside claude-pilot sessions. This is WHY the structural fix (`dispatch-lib.sh` detecting plan-on-branch and passing `--command "/ce:work"` directly) is the committed follow-up — it eliminates the model's opportunity to narrate rather than act. The prompt hardening in Unit 1 is mitigation to reduce failure probability until the structural fix ships.

  **Why this hardening is different from prior failures:** Prior prompt enforcement failures (N=10+) attempted to prevent the model from doing something it was capable of doing. This fix prevents the model from reaching `/ce:plan` at all by removing Step 1 from the pipeline when `PLAN_PATH` is set and making Step 2's tool-call requirement explicit. The key mechanism is not "MANDATORY/PROHIBITED" language alone (which prior failures show is unreliable) but the combination of: (a) removing the opportunity to invoke `/ce:plan` by marking Step 1 as structurally absent, (b) making Step 2 the entry point of the pipeline with explicit Skill tool invocation syntax, and (c) the recovery retry (Unit 2) as a safety net for when the hardening fails despite (a) and (b).

- **Recovery-mode prompt prefix over new skill:** On pipeline failure retry, modifying the prompt is simpler and requires no infrastructure changes. A new `dev-work` skill mapping to `/ce:work` would be cleaner but adds a skill, handler, and dispatch-lib case -- disproportionate for a retry path.

- **Single retry with recovery prompt, then escalate:** Instead of 2 blind retries with the same prompt (current behavior), use 1 recovery-mode retry with a targeted prompt, then escalate. This cuts worst-case retry cost from ~$2.40 to ~$1.50 while being more effective. On retry exhaustion, escalation follows the existing path: notify Vincent with "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries," proceed to Step 6 with `blocked`.

## Open Questions

### Resolved During Planning

- **Q: Can we dispatch `/ce:work` directly via a new skill?** Resolution: No -- disproportionate infrastructure for a retry path. Modify the prompt instead. The structural fix (Unit 3) addresses this at the `dispatch-lib.sh` level.
- **Q: Should we modify `/ce:plan` to detect groomed plans and refuse to run?** Resolution: No -- `/ce:plan` is third-party compound-engineering plugin code. The fix belongs in `/mika` which controls invocation.

### Deferred to Implementation

- Exact prompt wording will be tuned during implementation -- the plan specifies the enforcement shape and verbatim drafts, but final phrasing may be adjusted based on testing.

## Implementation Units

- [ ] **Unit 1: Strengthen plan-on-branch enforcement in `/mika` command (MITIGATION)**

**Goal:** Reduce the probability that the model invokes `/ce:plan` or narrates a confidence check when a plan-on-branch is detected. This is mitigation — prompt hardening with a documented failure rate. The structural fix is Unit 3.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `.claude/commands/mika.md`

**Approach:**

Three prompt changes, all modifying existing sections (not adding parallel blocks):

1. **Add a gate between plan-on-branch detection and Pipeline.** After the plan-on-branch detection block (line 28, before `## Worktree isolation`), insert:

   Verbatim replacement for the end of the plan-on-branch detection section:

   ```markdown
   **Plan-on-branch gate (MANDATORY when PLAN_PATH is set):** When `PLAN_PATH` is set, the Pipeline below starts at Step 2. Step 1 does not exist for this run. Your FIRST Skill tool invocation in the Pipeline MUST be `/ce:work`. Do NOT invoke `/ce:plan` — the plan is already architect-validated. Do NOT perform a confidence check or plan review. Do NOT output narrative text describing what you will do before invoking the Skill tool — invoke it.
   ```

   This block sits between plan-on-branch detection and Worktree isolation, so the model reads it before encountering any pipeline steps.

2. **Rewrite Step 1 with hard prohibition.** Replace current line 74:

   Current:
   ```
   1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments) — **skip when a plan-on-branch was detected above; jump straight to Step 2 with `/ce:work <PLAN_PATH>`**
   ```

   Replacement:
   ```
   1. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments) — **PROHIBITED when PLAN_PATH is set. When PLAN_PATH is set, this step does not exist. Do NOT invoke `/ce:plan`. Do NOT perform confidence checks or plan reviews. Proceed directly to Step 2.**
   ```

3. **Add explicit tool-call enforcement to Step 2.** Replace current line 75:

   Current:
   ```
   2. `/ce:work` — when a plan-on-branch was detected, invoke as `/ce:work <PLAN_PATH>` with the contract framing from the Issue linking section above
   ```

   Replacement:
   ```
   2. `/ce:work` — when a plan-on-branch was detected, invoke the Skill tool: `skill="compound-engineering:ce-work"` with `args="<PLAN_PATH>"` and the contract framing from the Issue linking section above. This MUST be an actual Skill tool invocation, not narrative text about invoking it.
   ```

**Patterns to follow:**
- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` -- merge enforcement into existing steps, put format requirements in the action step

**Test scenarios:**
- Happy path: `/mika #N` with plan-on-branch callout in issue body -> model skips `/ce:plan`, invokes `/ce:work` as first pipeline tool call
- Edge case: `/mika #N` with plan callout but plan file missing from worktree -> falls back to normal flow (runs `/ce:plan`)
- Edge case: `/mika #N` with no plan callout -> runs `/ce:plan` normally (no regression)

**Verification:**
- The plan-on-branch gate block exists between plan detection and Worktree isolation
- Step 1 contains PROHIBITED directive when PLAN_PATH is set
- Step 2 contains explicit Skill tool invocation syntax

- [ ] **Unit 2: Add recovery-mode retry for HEAD-unchanged pipeline failures**

**Goal:** When pipeline failure fires with HEAD unchanged, retry with a modified prompt that forces `/ce:work` invocation instead of re-running the full `/mika` flow.

**Requirements:** R3, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**

Modify the "On pipeline failure" handler (line 180+). The verbatim replacement for lines 180-185:

```markdown
**On pipeline failure (callback contains "PIPELINE FAILURE:"):**

1. Extract metadata (Session, Cost, Turns, Duration) from the lines after the PIPELINE FAILURE prefix.
2. Check `pipeline_retry_count` in task metadata (default 0). Call `check_task(task_id)`.
3. **Detect groomed plan.** Run `run_gh("issue view {issue_number} --repo senara-solutions/{repo} --json body --jq '.body'")`. Search the body for the plan callout pattern `> - **Plan:** \`docs/plans/<file>\``. If found, extract the plan path.
4. If `pipeline_retry_count >= 1`: escalate — notify Vincent "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries (recovery attempted: {had_plan_path})." Proceed to Step 6 with `blocked`.
5. If retries remain:
   a. Notify Vincent "Pipeline produced no commits for {repo}#{issue_number} — retrying with recovery mode ({n}/1)."
   b. Call `update_task_status` with same status `in_progress` and `metadata: {"pipeline_retry_count": <current + 1>}`. Verify persistence via `check_task`.
   c. Compose the retry prompt:
      - **If plan path was found (step 3):** `"RECOVERY_MODE: Previous /mika attempt on {repo} issue#{issue_number} produced zero commits. The groomed plan exists at {plan_path}. Your FIRST pipeline action MUST be invoking /ce:work {plan_path}. Do NOT invoke /ce:plan. Do NOT perform confidence checks or plan reviews. {repo}#{issue_number}"`
      - **If no plan path found:** same prompt as original dispatch (`"{repo}#{issue_number}"`)
   d. Call `run_claude_pilot` with the composed prompt and same `task_id`. If the call returns `{"status": "deferred", "deferred": true}`, the retry has been automatically enqueued — do NOT retry again. Proceed to Step 6 with status `in_progress` and note "pipeline retry deferred."
```

**Patterns to follow:**
- Existing `run_claude_pilot` prompt composition in self-dev (line 65-66, 277, 655)
- Existing plan-on-branch callout parsing pattern from `.claude/commands/mika.md` lines 14-20

**Test scenarios:**
- Happy path: Pipeline failure on groomed ticket -> retry prompt includes RECOVERY_MODE prefix with plan path -> model invokes /ce:work directly
- Edge case: Pipeline failure on non-groomed ticket (no plan callout) -> retry with normal prompt (current behavior preserved)
- Edge case: Pipeline failure after 1 recovery retry -> escalates to Vincent with "recovery attempted: true" note instead of retrying again
- Error path: Plan callout present but plan file deleted -> RECOVERY_MODE still sent (model discovers missing file and reports error, better than narrating again)

**Verification:**
- Pipeline failure handler runs `run_gh` to check issue body for plan callout before composing retry prompt
- Recovery prompt includes RECOVERY_MODE prefix with explicit /ce:work instruction when plan path found
- Retry cap is 1 (2 total attempts including original)
- Escalation message includes whether recovery was attempted
- Non-groomed ticket retry behavior is unchanged (same prompt, same cap)

- [ ] **Unit 3: File follow-up ticket for structural fix in dispatch-lib.sh**

**Goal:** Track the root-cause structural fix as a committed follow-up, not aspirational prose.

**Requirements:** R5

**Dependencies:** None (can be done in parallel with Units 1-2)

**Files:**
- No source changes — this unit creates a GitHub issue

**Approach:**

File a ticket on `senara-solutions/mika` with:
- Title: `fix(skill): dispatch-lib.sh should pass --command "/ce:work" for groomed plans`
- Body: When `dispatch_claude_pilot` detects a groomed plan-on-branch (plan callout in issue body), pass `--command "/ce:work <PLAN_PATH>"` instead of `--command "/mika"` to `claude-pilot`. This eliminates the model's opportunity to narrate instead of act — the structural fix for the narrate-then-exit failure class documented in mika#1072 and 10+ prior prompt-enforcement failures.
- Labels: `bug`, `p2-normal`
- Reference: `Related: #1072`

**Test expectation:** none -- this unit is a ticket, not code.

**Verification:**
- Issue exists on `senara-solutions/mika` with the title and labels above
- Issue body references #1072 as the parent fix

## System-Wide Impact

- **Interaction graph:** The `/mika` command is invoked by claude-pilot via `dispatch-lib.sh`. The prompt changes affect all claude-pilot sessions dispatched via `dev-pilot` skill. No other entry points invoke `/mika` (it has `disable-model-invocation: true`).
- **Error propagation:** Pipeline failure detection in `dispatch-lib.sh` is unchanged. The retry logic in `self-dev` only changes the prompt composition and cap -- the callback flow is preserved.
- **State lifecycle risks:** None -- the changes are prompt-level. No database, config, or state changes.
- **API surface parity:** The meta-repo `mika-platform/.claude/commands/mika.md` has a different pipeline structure (self-targeting) and does not have plan-on-branch detection. No parity change needed.
- **Unchanged invariants:** The `validate_head_unchanged` check in `dispatch-lib.sh` is not modified. The `ENTRY_COMMAND="/mika"` mapping in `dispatch-lib.sh` is not modified. The `run_claude_pilot` tool interface is not modified.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Prompt hardening has a documented ~10+ failure rate for this enforcement class | Explicitly framed as mitigation. Unit 2 (recovery retry) provides a safety net. Unit 3 (structural fix) is the committed root-cause resolution. The specific mechanism here (removing Step 1 from pipeline + explicit tool syntax in Step 2) is different from prior failures which tried to prevent actions the model was capable of; this one removes the opportunity to invoke `/ce:plan` entirely. |
| Recovery-mode prefix might confuse the model on non-plan-related failures | Only applied when plan callout is detected in issue body. Non-groomed ticket retries use the existing prompt. |
| Reducing retry cap from 2 to 1 might escalate too aggressively on transient failures | Transient failures (network, rate limit) produce different callback patterns (non-zero exit, not HEAD-unchanged), so they are not affected by this cap change. |

## Sources & References

- Related issue: senara-solutions/mika#1072
- Related code: `.claude/commands/mika.md`, `skills/bundled/self-dev/system_prompt.md`
- Institutional learnings: `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md`, `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`, `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md`
- Empirical evidence: Sessions `4c5bb02a-1bf6-4cd6-925b-e3ce3ad1713c` and `4594a1fb-f4ff-4572-8678-6e5eef57a9b5`
