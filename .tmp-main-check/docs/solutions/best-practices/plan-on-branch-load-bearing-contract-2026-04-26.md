---
title: "Plan-on-branch as load-bearing contract — read it, don't re-derive it"
date: 2026-04-26
category: best-practices
module: self-dev, qa-review, qa-review-build-callback, mika-arch
problem_type: best_practice
component: development_workflow
severity: high
applies_when:
  - Dispatching /mika when a plan file already exists on the target branch (groomed or previously derived)
  - qa-review verifying acceptance criteria for any PR with a plan callout in the issue body
  - qa-review-build-callback re-entering after a build completes — plan must be re-read unconditionally
  - mika-arch second-pass review when plan specifies output consumed by a documented downstream parser
  - Any pipeline step that could silently scope-reduce an existing plan instead of surfacing the conflict
tags:
  - plan-on-branch
  - pipeline-contract
  - scope-reduction
  - ac-verification
  - block-ac
  - fabrication
  - structural-guard
  - rule-rightsizing
related_components:
  - claude-pilot
  - mika-groom-ticket
  - self-dev-webhook-qa
---

# Plan-on-branch as load-bearing contract — read it, don't re-derive it

## Context

During mika-platform#54 (shipped as mika#824, `feat(ask): add --verbose flag with session_id metadata trailer`), every stage of the autonomous pipeline re-derived its primary artifact from upstream inputs instead of consuming the artifact the prior stage had already produced and committed. The groomed plan on the branch was ignored by `/mika`, which re-ran `/ce:plan` from issue prose. The QA reviewer's AC-verification step was silently skipped because it was gated on issue-body formatting rather than plan presence. The build-callback carried no plan state across the turn boundary, so it could not verify ACs even when the plan was available. When claude-pilot encountered an architectural conflict, it scope-reduced silently and wrote a new plan file rather than escalating to the operator.

The result: 1 of 11 specified metadata fields shipped. The plan said "11 fields, alphabetical in JSON, importance-ordered in text, token fields gated on `MIKA_STORE_LLM_CALLS=true`." Reality: text mode emitted `session_id` only; JSON `--verbose` was ignored entirely.

Session-history (`feat/823`, 2026-04-26T16:49) shows the failure mode concretely: `/ce:plan` looked for `docs/plans/ | grep -i verbose`, found sequence 004 was next, then looked for the plan referenced in the issue body (`2026-04-26-002-...`) and concluded "that doesn't exist. I have all the context I need." It then wrote a NEW Lightweight 2-unit plan from issue prose covering only `--verbose` flag + `session_id:` trailer — silently discarding the 11-field groomed plan's broader scope. **The lookup heuristic was physically correct but scoped to the wrong tree** — the groomed plan lived on the mika-platform branch, not the mika worktree, and no contract surface existed to bridge the two.

This is the **third documented instance** of conflict-resolution / scope-reduction drift in this pipeline:

1. mika#485 cross-repo mis-routing (2026-04-08) — `docs/solutions/prompt-engineering/2026-04-08-cross-repo-issue-scope-drift-after-upstream-merge.md`
2. KG milestone M14 retrospective (2026-04-22) — `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`
3. mika-platform#54 (2026-04-26) — this doc

Three instances in six weeks is a trend, not coincidence. Prompt-level patches are catching individual cases but not the structural gap.

## Guidance

**The uniform fix shape: each pipeline stage reads the artifact the prior stage produced. Re-derivation from upstream inputs is the drift surface.**

mika#825 ships six structural changes, all in this shape:

### 1. `/mika` reads the plan callout — does not re-derive the plan

Parse the issue body for the callout `/mika-groom-ticket` writes at grooming time:

```
> - **Plan:** `docs/plans/<filename>.md` (committed on branch @ `<sha>`)
```

If the callout is present and the path exists in the worktree, route directly to `/ce:work <plan-path>` with explicit framing: "this plan was groomed by the architect; it is the contract; surface conflicts via `send_message`; do not silently scope-reduce; do not write a new plan file." Fall back to `/ce:plan $ARGUMENTS → /ce:work` only when the callout is absent or the path is missing.

### 2. `qa-review` Step 2.5 — behavioral AC verification driven by the plan

Read the plan. Extract every AC bullet. Classify as Behavioral / Structural / Documentation / CI-deferred. Verify Behavioral ACs by running the built binary (via `build_mika` callback). Emit a `PLAN-AC VERIFICATION:` block with `[✅] / [❌] / [⏭️]` per bullet. Any `[❌]` → `VERDICT: block[ac]` (gating, not advisory). Plus an implicit structural AC: PRs must not add new files in `docs/plans/` unless the new file's frontmatter includes `parent_plan: <path>` override.

The prior conditional gate (Step 3e ran only when issue body contained backtick-wrapped `mika` commands) is removed. The plan is the trigger, not issue formatting. (session history: prior sessions where Step 3e ran did so because issue bodies *happened to* match the gate; the silent-skip in feat/823 was working as designed — that's the defect.)

### 3. `block[ac]` routes to operator without auto-retry

`block[ac]` is a distinct verdict class from `block[ci]` (which auto-retries transient failures). In `self-dev-webhook-qa`'s consumer:

- Parse the `Plan amendment required:` section from the review body.
- **Mutate task state before sending the operator notification.** `update_task_status({status: "blocked", note: "Plan amendment required (block[ac])"})` first, then `send_message` to operator. This ensures persisted state matches the notification the operator receives. If the order were reversed, partial failure (state mutation succeeds, notification fails — or vice versa) produces a divergence the operator cannot detect.
- Null-task guard for out-of-band PRs: if Step 4 correlation finds no task, skip the inline mutation and notify the operator that no task was correlated.
- Pause parent milestone if applicable, with `check_task` verification (engine returns success on terminal-state transitions but doesn't actually transition — warning surfaced if so).
- **No `run_claude_pilot`. No `qa_retry_count` increment.** AC mismatches are not transient.

### 4. mika-arch second-pass checks output-format compatibility before approving

When the plan specifies new or changed output for any channel with documented downstream parsers (CLI stdout, structured logs, persisted audit events, HTTP API responses), the architect must `grep`/`gh_read` for consumers across `mika/`, `mika-skills/`, `mika-platform/.claude/commands/`, verify shape compatibility, and surface conflicts as ESCALATE. Greping for consumers is not optional; it is the architect's job to verify that the artifact downstream stages will consume is internally consistent before it ships.

This is a pre-commit-discovery extension of the same shape used in mika#821 Finding 6 and mika-platform#52 Finding 2 — extending "verify your assumptions about source code" to "verify your assumptions about downstream parsers."

### 5. Rule rightsizing over workarounds

When a constraint forces a complex artifact lifecycle, **audit whether the constraint applies to the case at hand** before working around it. The `qa-review-build-callback` had a "do not re-run Steps 1–3d" rule added to prevent expensive redundant operations (`qa_pr_view`, full diff review). This got over-generalized to plan re-reading.

Friend-review on 2026-04-26 reframed the design discussion (session history: c4af9250 / mika-arch consultation): the right axis isn't a A/B/C state-passing choice — it's whether the constraint forcing the choice is even load-bearing. It wasn't. Plan re-reading is cheap (typical plan ≤ 5K tokens) and the plan is the single source of truth for ACs. Threading state across the turn boundary (option B: persist `.qa-review-state.json` to worktree) introduced new lifecycle complexity (clobbering, gating-the-write, structural binding still prompt-level) for no real correctness gain.

**Rightsized:** the callback now unconditionally re-reads the plan and re-extracts ACs at every entry. Plan unreadable → `block[pipeline]` (structural failure, NOT downgraded to `hold[review]` by Data Integrity Rule). One source of truth, no state file, no carry-forward ambiguity.

When a constraint forces a complex workaround, ask whether the constraint is load-bearing or cargo-cult. **Per-step cost analysis precedes blanket rules.**

### 6. Verify producer-consumer contract end-to-end after structural changes

A PR making a contract load-bearing must verify that contract is end-to-end coherent before merge. Run `/ce:review` on the initial commit. Cross-reviewer agreement (3+ reviewers flagging the same finding) is high signal for true contract gaps.

The initial commit of mika#825 had **3 P1 + 6 P2 internal-consistency gaps** caught by 8 parallel reviewers:

- Producer fallback emitted `Conflict reason:` (without the `(inferred)` suffix the consumer parser matched) → fallback path silently dropped conflict reasons from operator notifications.
- Build-callback's verdict-body description and example template still showed pre-PR shape — would have produced verdicts violating the producer's own contract.
- `gh api` fallback in Step 2.5.1 was dead code — the `run_gh` allowlist in the same file blocks `api`.
- `block[ac]` handler called `send_message` before `update_task_status` → partial failure → state divergence (the "state mutation before notification" ordering above came directly from this finding).

Structural fixes have internal consistency surfaces of their own.

## Why This Matters

Each documented instance of drift follows the same shape: a stage re-derives rather than reads, drift accumulates, and the shipped artifact diverges from the agreed plan. **Three instances in six weeks is a trend, not a coincidence.**

The broader implication: prompt-level fixes are catching individual cases but not the structural gap. The pipeline has no enforced artifact-passing contract. Each stage can silently fall back to re-derivation, and there is no engine-level gate that prevents it.

Engine-level candidates for the next sprint (per `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, "behaviors against trained gradients must bind structurally"):

- **`set_skill_state` / `get_skill_state`** — engine-persisted cross-turn state for skills that need continuity across callback invocations without relying on worktree files. (Tracked as a working note with explicit trigger: any production qa-review-build-callback session that emits `hold[review]` because the unconditional plan re-read was silently skipped.)
- **"Skills can declare invariants the engine enforces"** — e.g., a skill declares "this step requires a plan callout in the issue body; if absent, emit `block[pipeline]` before dispatch." Currently the same check runs in skill prose, where a model can ignore it.

Without engine-level support, each new skill that participates in multi-stage artifact passing must implement its own read-or-escalate guard, and the uniformity of those guards depends on author discipline rather than structural enforcement. **The 3rd documented instance is the threshold for structural work — don't wait for the 4th.**

### Secondary impact: fabrication-class drift

The bundled fix in mika#825 (Change 4: self-dev M2 milestone-title fetch) addresses a 3rd+ documented instance of kimi-k2.5 fabrication on tool error. Prior instances:

- mika#308 (2026-04-11) — fabricated PR comment URLs with zero tool calls. Engine-level fix: `fabricated-action-claim-guard` (URL-only regex postcondition).
- mika#638 (2026-04-21) — PR-number fabrication.
- 2026-04-23 controlled A/B (`docs/solutions/best-practices/autonomous-agent-operational-discipline-2026-04-23.md` §1) — kimi vs sonnet under tool error.
- mika#825 Change 4 — `gh milestone list` errored, kimi returned a plausible-but-wrong title (`mika-platform CLI Consolidation` vs real `slash-command-coherence`).

Session history (c4af9250, 2026-04-26T18:44) shows a **concurrent** kimi fabrication during the very session that authored this fix: when asked to cancel a task, mika-dev (on kimi-k2.5) responded "Task state reconciled. Cancelling as instructed" but made zero tool calls. The DB showed no status change. Operator response: "I'm tired of all this bs. now mika dev is running on sonnet." (session history)

Pattern: when a tool call errors and the model continues with a confident-sounding value, fabrication risk is high. The existing `fabricated-action-claim-guard` is URL-only — broader engine-level fabrication-class containment is the structural fix; sonnet propagation across grounding-sensitive dispatch steps is the immediate mitigation. (See `feedback_sonnet_over_kimi_for_grounding.md` in auto memory.)

## When to Apply

**Apply the "consume the artifact, don't re-derive" pattern whenever:**

- A pipeline stage receives as input an artifact a prior stage produced and committed. The consuming stage should read the committed artifact directly, not re-derive it from the upstream inputs the prior stage consumed.
- A verdict needs to gate downstream behavior non-transiently. Convert `hold[review]` to `block[<class>]` when the failure is a plan-vs-implementation conflict, a missing AC, or any other condition not resolved by retrying the same implementation. `hold[review]` is for transient or judgment calls only.
- A "do not re-run" rule exists in a callback or hook. Audit it per-step: which operations are expensive (and should not re-run), which are cheap and authoritative (and should unconditionally re-run). Do not generalize from "re-running this expensive operation is wasteful" to "re-running any operation in this step is wasteful."
- A plan specifies new or changed output on a channel with downstream parsers. Verify parser compatibility at grooming time (mika-arch), not at review time — catching it post-implementation requires a plan amendment cycle.
- A gating block verdict is introduced. Pair it with a named escalation path: who receives the notification, what state is persisted first, whether auto-retry is suppressed. **A gating block without a routed escalation is a stalled work item.**
- A fix targets the dispatch pipeline itself. Direct manual implementation isn't a regression — it's the only path that doesn't depend on the broken thing. Document the bootstrap-problem justification in a working note. (session history: c4af9250 — first explicit bootstrap-problem takeover documented; claude-pilot session `e49d88a9` returned "Success | 12 turns | $0.74 | 99s" with **0 commits** when dispatched against this very fix.)

**Do not apply blindly when:**

- The "artifact" is large, volatile, or derived from live runtime state (current test results, live API responses). Re-derivation may be correct there. The pattern applies to stable, committed, plan-time artifacts: plan files, groomed callouts, AC lists extracted from the plan.

## Examples

### Plan callout shape (producer/consumer contract)

Producer: `/mika-groom-ticket`. Consumers: `mika.md` (`/mika` command), `qa-review/system_prompt.md`, `qa-review-build-callback`.

```
> - **Plan:** `docs/plans/2026-04-26-005-fix-plan-on-branch-load-bearing-plan.md` (committed on branch @ `02bc628a`)
```

The callout must appear verbatim in the issue body for `/mika` to detect it. The path must exist in the worktree. If either condition fails, the pipeline falls back to re-derivation mode.

### `block[ac]` verdict body (producer/consumer contract)

Producer: `qa-review`. Consumer: `self-dev-webhook-qa`.

```
PLAN-AC VERIFICATION:
Plan: docs/plans/<plan>.md
ACs evaluated: 9
- [❌] unsatisfied: `mika ask --verbose` emits the v1 metadata block ...
  expected: 11 fields, alphabetical in JSON, importance-ordered in text, token fields gated on MIKA_STORE_LLM_CALLS=true
  actual: text mode emits session_id only (1 of 11 fields); JSON --verbose ignored entirely
- [✅] satisfied: --verbose flag added to clap config
- [⏭️] CI-deferred: no test regressions
- [✅] implicit structural: no parallel plan files in docs/plans/

VERDICT: block[ac]
REASON: Plan AC for v1 metadata block unsatisfied — 10 of 11 fields missing.

Plan amendment required:
- AC: `mika ask --verbose` emits the v1 metadata block in JSON and prose formats per the field list and rendering rules above
  Conflict reason (inferred): JSON-nested-metadata shape conflicts with `/mika-groom-ticket`'s parser, which scans for `session_id: <uuid>` lines on stdout. Resolution requires either (a) amend plan rendering to match parser shape, or (b) amend `/mika-groom-ticket` parser to handle nested JSON. Operator decision required — auto-retry inappropriate.
```

The `Conflict reason (inferred):` suffix is **load-bearing** — the consumer parser in `self-dev-webhook-qa` matches that exact string. If the producer emits `Conflict reason:` (without the suffix), the consumer silently drops the conflict reason. This was a P1 finding from the `/ce:review` pass and is fixed by emitting the suffix verbatim on every path including the fallback.

### Rule rightsizing language (qa-review-build-callback prompt header)

> Scope of "do not re-run": the rule applies to **expensive operations whose outputs would be redundant noise** — `qa_pr_view`, full diff review (Step 3a–3d), cross-repo `pr list` searches. It does **NOT** apply to **plan re-reading**: Step 2.5.1's `cat <worktree>/<plan-path>` is cheap (typical plan ≤ 5K tokens) and the plan is the single source of truth for ACs. **Always re-read the plan unconditionally at the start of this callback.** If the plan file cannot be read (worktree gone, plan deleted between turns), emit `block[pipeline]` with reason "plan unreadable in callback: <error>" — this is a structural failure, NOT downgraded to `hold[review]`.

### Fabrication guard pattern (Change 4)

```bash
# WRONG — `gh milestone list` is not a real subcommand; model fabricated it
gh milestone list --repo senara-solutions/mika --json title,number

# CORRECT — verified working
gh issue list --milestone <N> --state all --json milestone --jq '.[0].milestone.title'
```

When a tool call errors and the model continues with a confident-sounding value, fabrication risk is high. **Always verify subcommand existence independently of model confidence.** If the subcommand cannot be verified, surface the error and wait for operator input rather than proceeding with a derived value.

## Related Issues

- PR: [`senara-solutions/mika#825`](https://github.com/senara-solutions/mika/pull/825) — implementation
- Plan: `docs/plans/2026-04-26-005-fix-plan-on-branch-load-bearing-plan.md`
- Origin incident: mika-platform#54 / mika#824 (`feat(ask): add --verbose flag with session_id metadata trailer`)
- See: [`workflow-issues/grooming-branch-callout-required-2026-04-25.md`](../workflow-issues/grooming-branch-callout-required-2026-04-25.md) — establishes the branch-callout convention this doc extends from routing to behavioral contract.
- See: [`architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`](../architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md) — structural enforcement over prompt rules; cited justification for deferring engine-level `set_skill_state` until trigger condition is met.
- See: [`best-practices/intent-signal-not-completion-signal-2026-04-24.md`](./intent-signal-not-completion-signal-2026-04-24.md) — same re-derivation shape: agent advances on intent signal rather than reading the artifact the prior stage produced.
- See: [`best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md`](./prompt-vs-tool-contract-mismatch-2026-04-24.md) — Shape C: structured verdict consumed by downstream parser — same as `block[ac]` routing contract.
- See: [`logic-errors/milestone-skips-m2-creates-incomplete-children.md`](../logic-errors/milestone-skips-m2-creates-incomplete-children.md) — same sub-pattern: step silently skipped when gating condition rarely met (Step 3e analog).
- See: [`architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md`](../architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md) — verification at every layer principle.
- See: [`prompt-engineering/2026-04-08-cross-repo-issue-scope-drift-after-upstream-merge.md`](../prompt-engineering/2026-04-08-cross-repo-issue-scope-drift-after-upstream-merge.md) — conflict-resolution drift instance #1.
- See: [`workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — conflict-resolution drift instance #2.
- See: [`architecture-patterns/fabricated-action-claim-guard.md`](../architecture-patterns/fabricated-action-claim-guard.md) — engine-level URL fabrication backstop; Change 4 is prompt-level; engine broadening deferred.
- See: [`prompt-engineering/grounding-rule-downstream-state-hallucination.md`](../prompt-engineering/grounding-rule-downstream-state-hallucination.md) — fabrication class; over-extrapolation from valid upstream input.
- See: [`best-practices/autonomous-agent-operational-discipline-2026-04-23.md`](./autonomous-agent-operational-discipline-2026-04-23.md) §1 — kimi fabrication under tool error; mika#825 Change 4 is the 3rd+ documented instance of this class.
