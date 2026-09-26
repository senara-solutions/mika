---
title: "Milestone-close-claim guard: GitHub state enforcement"
category: architecture-patterns
date: 2026-05-17
module: self-dev
problem_type: best_practice
component: agent-loop
severity: medium
tags:
  - agent-loop
  - milestone-workflow
  - fabrication
  - guard
  - post-condition
  - re-prompt
  - verify-post-state
  - github-state
issue: 797
related_issues: [483, 308, 702, 788, 789]
related_docs:
  - docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md
  - docs/solutions/architecture-patterns/fabricated-action-claim-guard.md
  - docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md
  - docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md
  - docs/solutions/prompt-enforcement-structural-guards.md
---

# Milestone-close-claim guard: GitHub state enforcement

## Problem

mika-dev's milestone-completion workflow (self-dev Step M5) marked the milestone task `completed` in the local `~/.mika/data/mika.db` and emitted "Milestone#N closed, tasks reconciled, memory updated" — without ever calling the GitHub Milestones API. Result: local DB and GitHub state diverged silently, the autonomous loop reported success, and the milestone stayed `state: open` on GitHub indefinitely.

Triggering incident: milestone#17 (Knowledge Graph corpus dedup), 2026-04-24 → 2026-04-25. All 5 child issues had merged; mika-dev processed the M5 build+deploy follow-up overnight and reported clean completion at 08:00. `gh api /repos/senara-solutions/mika/milestones/17 --jq .state` at that time returned `"open"`. Vincent closed manually via `gh api -X PATCH ... -f state=closed`.

This is the same class as #483 (completion-claim without `update_task_status`) and #308 (fabricated action-claim with GitHub URL but zero tool calls) — extended to a third surface: **claimed change to a GitHub-API-side state that was never actually invoked**.

## Root Cause

Two coupled gaps:

1. **Prompt-level gap.** Step M5 (`skills/bundled/self-dev/system_prompt.md`) transitioned the parent task locally but had no explicit step instructing `run_gh api -X PATCH /repos/.../milestones/<n> -f state=closed`. The LLM treated child-issues-all-merged as sufficient signal for milestone closure — the same "intent signal vs completion signal" failure class documented in [`intent-signal-not-completion-signal-2026-04-24.md`](../best-practices/intent-signal-not-completion-signal-2026-04-24.md) (mika#789).

2. **Engine-level gap.** No structural backstop existed for fabricated milestone-close claims. The sibling `detect_completion_claim` guard (#483) catches `merged`/`deployed`/`completed`/`shipped` keywords but not `closed`. The fabricated-action-claim guard (#308) requires a GitHub resource URL and zero tool calls — neither condition matched the milestone#17 emission (which had several legitimate tool calls earlier in the turn, just not the close PATCH).

Both gaps were necessary. Fixing only the prompt would leave the prompt-enforcement-fragile failure mode in place (memory: `feedback_prompt_enforcement_fragile.md` — LLMs rationalize crossing prompt-level rules). Fixing only the engine would leave the agent without an instruction to call the right tool.

## Solution

Two-part fix, both in one PR (#797 → #1184):

### Part A — Self-dev prompt M5 step 3

Inserted before `update_task_status(completed)`:

```markdown
3. **Close the GitHub milestone (REQUIRED before marking the parent task complete):**

   3a. Issue the close PATCH:
       run_gh({"command": ["api", "-X", "PATCH",
                            "/repos/senara-solutions/<repo>/milestones/<n>",
                            "-f", "state=closed"]})

   3b. Read back the state:
       run_gh({"command": ["api",
                            "/repos/senara-solutions/<repo>/milestones/<n>",
                            "--jq", ".state"]})

   3c. Branch on the readback:
   - Output is exactly `"closed"` (with quotes — `--jq .state` emits JSON):
     proceed to step 4.
   - Output is `"open"` or anything else: STOP. Do NOT call
     update_task_status(completed). Notify Vincent. Mark task blocked.
   - 3a returns a non-2xx error: STOP. Notify Vincent with the gh error.
     Mark task blocked. Do NOT claim success.
```

The notification template also gained a verified-close marker (`Milestone closed on GitHub: ✓`) so the operator can distinguish a verified close from a claimed-but-unverified one.

### Part B — Structural guard at post-condition slot #4b

`crates/mika-agent/src/agent.rs`:

```rust
// Milestone-close-claim guard (#797): if the agent claims a
// GitHub milestone was closed but did not invoke run_gh with the
// close PATCH, reject and re-prompt once.
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !milestone_close_claim_retry_done
    && let Some(keyword) =
        detect_milestone_close_claim_without_patch(&text, &all_tool_summaries)
{
    milestone_close_claim_retry_done = true;
    // ... push assistant message + push user correction + continue
}

fn detect_milestone_close_claim_without_patch<'a>(
    text: &'a str,
    all_tool_summaries: &[ToolCallSummary],
) -> Option<&'a str> {
    if !text.to_lowercase().contains("milestone") { return None; }
    let caps = MILESTONE_CLOSE_CLAIM_RE.captures(text)?;
    let keyword = caps.get(1).map(|m| m.as_str())?;
    let has_patch_call = all_tool_summaries.iter().any(|s| {
        s.name == "run_gh"
            && s.input_summary.contains("\"api\"")
            && s.input_summary.contains("\"PATCH\"")
            && s.input_summary.contains("state=closed")
            && MILESTONE_API_PATH_RE.is_match(&s.input_summary)
    });
    if has_patch_call { None } else { Some(keyword) }
}
```

Regex window is 80 chars (`\bmilestone\b.{0,80}\b(closed|close)\b`), sized for canonical incident phrasings without spanning paragraph breaks.

## Why This Works

**Chain composition with #4 completion-claim guard.** Both guards can match the same text (e.g., "Milestone#17 completed and closed"). The post-condition chain evaluates serially with `continue` on first match — completion-claim (#4) fires first, single retry. If the agent corrects #4 by adding `update_task_status` but still omits the PATCH, the milestone-close guard (#4b) fires on the next EndTurn. Independent retry flags per failure class, intentional ordering: broader catch first, specialized catch second.

**Single-retry budget shared with sibling guards.** `milestone_close_claim_retry_done` is a `bool` on the `run_loop` body, same shape as `completion_claim_retry_done`. On second violation in the same `run_loop`, the guard emits a `warn!("Milestone close claim guard already fired this turn — accepting EndTurn with second violation (budget exhausted)")` and lets the EndTurn through. Grep-friendly observability without infinite re-prompt risk.

**Sibling-ticket sequencing.** Blocked by mika#788 (run_gh allowlist must include `"api"` before this PR's prompt could be invoked). Plan committed only after #788 merged; the dispatch-readiness `blockedBy` check (#713) would have refused implementation if attempted earlier.

**The `state=closed` substring check (added in /ce:review revision)** narrows the satisfying-call surface so milestones PATCH on non-state fields (e.g., `-f title=...`) does NOT satisfy the guard. Reduces substring-spoofing surface flagged by adversarial review.

## When to Apply This Pattern

Use this structural-guard pattern (companion to the prompt-level "verify post-state" discipline) when:

- The agent emits text claiming a state change to an **external system** whose canonical state lives outside mika's DB (GitHub, S3, K8s, third-party APIs).
- The state change is **detectable structurally** via a tool-call shape (specific tool name + specific argv pattern).
- A prompt-level instruction alone is insufficient because the LLM has historically rationalized crossing it (`feedback_prompt_enforcement_fragile`).

Do NOT add a new structural guard when:

- The state change is internal (DB write) — use a structured `tools_called` set-membership check like `detect_completion_claim` instead.
- The detector requires complex argv parsing that the substring shape can't express robustly — file a followup to add argv-parse support (mika#1182 tracks this for the milestone-close guard) rather than shipping a fragile guard.
- Fewer than three instances of the failure class exist — defer abstraction; clone-and-modify the sibling guard is correct at N=2 (this PR is N=3 if you count #483 + #308 + #797).

## Prevention Strategies

1. **Pair every external-state mutation prompt instruction with a structural guard.** If the prompt says "call X tool", add a post-condition check that fires when the LLM claims X happened without calling X. Don't trust prompt enforcement alone for irreversible-or-divergent state.

2. **Verify-post-state ≠ ack-tool-return.** A `run_gh` PATCH returning 2xx is intent-signal; reading back the state is completion-signal. The prompt's step 3c (readback branch) encodes this. Future similar fixes should follow the PATCH → readback → branch-on-state shape, not just PATCH → trust.

3. **Eval scenarios use frozen pre-fix fixtures.** The C2 regression replay locks the milestone#17 emission shape against future regression. New guards in this family should ship with both a happy-path scenario (C1) and a frozen-fixture regression-reproduction scenario (C2).

4. **Document chain-composition semantics in code AND CLAUDE.md.** The interaction between #4 and #4b is non-obvious. Both the guard's inline comment and `crates/mika-agent/CLAUDE.md § Post-Conditions` describe the ordering and independent retry flags.

5. **Cross-ticket coordination via `blockedBy`.** Sibling tickets that depend on each other (#788 allowlist → #797 PATCH usage) should use GitHub `blockedBy` edges so the dispatch-readiness guard (#713) refuses out-of-order implementation.

## Followups Filed

- **mika#1182** — argv-parse hardening for `has_patch_call` (addresses substring-spoofing, INPUT_SUMMARY_MAX truncation, cross-milestone number binding flagged by adversarial review).
- **mika#1183** — eval coverage gaps for the milestone-close guard (chain-ordering integration test, M5 step 3c open-readback eval, C2 invariant strengthening).

## Related Patterns

- [`completion-claim-guard-work-item-state-enforcement.md`](completion-claim-guard-work-item-state-enforcement.md) — sibling guard (#483) for internal task-state claims.
- [`fabricated-action-claim-guard.md`](fabricated-action-claim-guard.md) — sibling guard (#308) for GitHub-URL action claims with zero tool calls.
- [`intent-precondition-registry-guard-generalization-2026-04-21.md`](intent-precondition-registry-guard-generalization-2026-04-21.md) — registry pattern (#702) for trigger-based guards. The milestone-close guard is inline (not in the registry) because its predicate operates on assistant text plus turn-scoped tool summaries — same shape as #4 completion-claim. If a third instance lands with the same shape, consider extracting a shared `push_correction_and_retry()` tail helper, but not a `Guard` trait (predicates vary too much).
- [`intent-signal-not-completion-signal-2026-04-24.md`](../best-practices/intent-signal-not-completion-signal-2026-04-24.md) — best-practice doc (mika#789) that **explicitly anticipated** this fix as the structural-guard half of the verify-post-state discipline. This PR mechanizes that guidance.
- [`prompt-enforcement-structural-guards.md`](../prompt-enforcement-structural-guards.md) — the broader pattern of belt-and-suspenders prompt + engine enforcement.
- [`logic-errors/run-gh-allowlist-hallucinated-subcommands-2026-05-17.md`](../logic-errors/run-gh-allowlist-hallucinated-subcommands-2026-05-17.md) — sibling fix (mika#788) that added `"api"` to `GH_ALLOWED_SUBCOMMANDS` so this PR's PATCH calls can succeed.
