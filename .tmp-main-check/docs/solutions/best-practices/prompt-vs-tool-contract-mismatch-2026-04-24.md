---
title: "Prompt-vs-tool contract mismatch: when skill prompts instruct forbidden tool calls"
module: agent-framework
date: 2026-04-24
problem_type: bug_class
component: skill-prompts-and-tool-dispatch
severity: high
applies_when:
  - "Authoring a new skill prompt that instructs the LLM to call a specific tool in a specific turn context"
  - "Adding a runtime guard to tool dispatch (long_running, session-kind restrictions, forbidden subcommand patterns)"
  - "Reviewing prompt admonitions of the form 'never call X' or 'always use Y'"
  - "Any prompt that says 'then call <long_running tool>' where the surrounding context could be a callback or webhook turn"
tags:
  - prompt-design
  - tool-contract
  - framework-enforcement
  - self-dev
  - incident-pattern
  - policy-table
---

# Prompt-vs-tool contract mismatch: when skill prompts instruct forbidden tool calls

## Context

Milestone #17 autonomous execution surfaced two independent incidents on the same day that share a single root cause: a skill prompt instructs the LLM to do something the framework categorically refuses. The LLM follows the prompt; the framework rejects or silently allows an improvised fallback; downstream state goes wrong. This compound doc names the bug class, captures the two concrete cases, explains why prompt-only enforcement keeps failing, and records the structural response (policy table + tagged-union tool returns) that was agreed during peer review.

This is the second occurrence in ~16 days of the same class (prior: mika#485 on 2026-04-08). Prompt admonitions alone are insufficient — they decay the moment a novel context appears that the prompt author didn't anticipate, and they provide no enforcement when the LLM improvises.

## The bug class

A skill prompt and a framework-level tool guard are *independently* expressing the same contract, and they disagree. Three shapes exist:

- **Shape A — prompt instructs, framework refuses.** The prompt says "call tool X in situation S." The framework rejects X in S (typically via a runtime guard). The LLM tries, gets `success=0`, improvises a fallback. Correctness then depends on the LLM's improvisation, which is a dice roll.
- **Shape B — prompt forbids, framework allows.** The prompt says "never call X." The framework has no guard. The LLM, under novel prompting pressure or a malformed tool response, calls X anyway. The tool executes. Downstream state goes wrong.
- **Shape C — prompt expects structured response, tool returns string error.** The prompt branches on `action: "pass"` / `"auto_merge_enabled"` / `"blocked"`. The tool hits an unmodeled failure mode and returns a string like `"Failed to fetch check statuses: gh exit code 1"`. The prompt has no branch for that; the LLM string-matches (badly) or improvises.

All three have the same root cause: **the contract between prompt and tool is implicit, not co-designed and not machine-enforced.** Each side evolves independently; each evolution opens a gap the other side doesn't know about.

## Two concrete cases

### Case 1 — Shape A: self-dev retry instructs `run_claude_pilot` in callback turns

**Prompt** (`crates/mika-agent/src/skills/bundled/self-dev/system_prompt.md:125-130`):

> **On pipeline failure (callback contains "PIPELINE FAILURE:"):**
> ...
> 4. If retries remain: notify Vincent... Then call `run_claude_pilot` with the same `repo#number` and `task_id`.

**Framework guard** (tool dispatch, agent crate):

```
Tool 'run_claude_pilot' is declared long_running but cannot run in the current context
(callback turn, silent mode, or CLI test). Long-running tools require a conversation-mode
turn with an active task engine.
```

**Incident** (2026-04-24, task `0ee3287c-...`): mika issue#786's first claude-pilot session hit 201-turn SDK limit. Callback arrived. mika-dev followed the prompt — set `pipeline_retry_count: 1`, called `run_claude_pilot("mika#786", task_id)`. Tool refused. mika-dev improvised: `send_message("Reply 'retry 786' to dispatch a fresh claude-pilot session.")`. Vincent manually replied `retry 786`; a second claude-pilot session completed in 81 turns, $8.62, PR merged.

The improvisation happened to be correct. Future model snapshots might improvise differently. Correctness depended on a dice roll.

### Case 2 — Shape B+C: Rule 6 violation on `pr_merge_with_gate` gate-error

**Prompt** (`crates/mika-agent/src/skills/bundled/self-dev-webhook-qa/system_prompt.md:114-118`):

> ### Rule 6 — Always use pr_merge_with_gate for PR merges
> Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR.
> **Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate.

**Framework guard:** none. `run_gh` accepts any subcommand.

**Tool behavior** (`pr_merge_with_gate`): returns unstructured string errors when check fetching fails (no tagged-union response variant for "gate errored").

**Incident** (2026-04-24, PR senara-solutions/mika#792): #787's branch diverged from main after #786 merged (both edit `migrate_v26_to_v27` in `crates/mika-agent/src/db.rs`). PR landed `mergeable: CONFLICTING, mergeStateStatus: DIRTY, statusCheckRollup: []`. QA approved. mika-dev on `pull_request_review.submitted` webhook turn:

1. Step 0: called `pr_merge_with_gate({pr_number: 792, repo: "senara-solutions/mika"})` → returned string error `"Failed to fetch check statuses: gh exit code 1: no checks reported on the branch"`. Tool never inspected `mergeable` state; conflated "no CI yet" with "CI can't run because conflicts exist."
2. Step 1: called `run_gh pr merge 792 --squash --delete-branch --auto` — **direct Rule 6 violation**. GitHub accepted the auto-merge arming even on a conflicted PR; merge will never fire because (a) conflicts block merge, (b) no CI exists to trigger armed merge.
3. Task sat `in_progress` indefinitely. mika-dev had no visibility into the conflict state.

Vincent manually resolved the conflict via rebase, force-push, and re-arm. Recovery: ~3 minutes of operator time, no data loss.

The Shape C component: the tool's string error had no branch in the prompt. Shape B component: the prompt's "never call X" admonition was the ONLY enforcement layer, and it failed — for the second time (mika#485 was the first).

## Why prompt-only enforcement keeps failing

Prompt admonitions are rules expressed to an LLM as natural-language instructions. They have no runtime teeth. They fail in three ways:

1. **Novel-context decay.** The prompt author anticipates contexts A, B, C. Context D arrives (unmodeled tool failure, mid-session state the author didn't consider). The LLM interpolates. The interpolation doesn't honor the admonition because the admonition was written against known contexts.
2. **Improvisation correctness is dice.** When the prompt-instructed path fails, the LLM reasons about what to do instead. Sometimes it picks the right fallback (Case 1's "ask Vincent for retry" was correct). Sometimes it picks the wrong one (Case 2's "use run_gh pr merge --auto" directly violated the admonition). Correctness is path-dependent and snapshot-dependent.
3. **No discoverability at author time.** A new skill author can't easily discover what the framework will refuse. There's no type system saying `run_claude_pilot` is forbidden in callback turns, or that `run_gh pr merge` is forbidden in webhook-qa sessions. The mismatch is only discoverable via incident.

## The structural response

Two paired changes, co-designed so prompt and framework speak the same language:

### 1. Tools return tagged-union responses, not strings

Every tool whose prompt has a branching response shape should return a structured variant type, not a free-form string. For `pr_merge_with_gate`:

```
enum PrMergeGateResult {
    Pass,
    AutoMergeEnabled,
    Blocked { reason: BlockReason, failing_checks: Vec<String> },
    GateError { kind: GateErrorKind, detail: String },
}

enum BlockReason { MergeConflict, RequiredCheckFailed, MissingApproval, ... }
enum GateErrorKind { NoChecksReportedYet, GhCliFailure, NetworkError, ... }
```

The prompt branches on the variant. No string-matching. The tool owns the taxonomy of outcomes; the prompt owns the response policy. The `mergeable` state check folds into the tool's preflight — a single `gh pr view --json mergeable,mergeStateStatus,statusCheckRollup` call replaces the current check-fetching-first approach.

### 2. Framework-level policy table for tool-invocation forbids

A single declarative rule set enforces "in session kind K, tool T with arg_pattern P is forbidden." Seeded from today's grep audit of `crates/mika-agent/src/skills/bundled/`:

| session_kind | tool | arg_pattern | action | reason |
|--------------|------|-------------|--------|--------|
| self-dev, self-dev-webhook-ci, self-dev-webhook-qa | `run_gh` | `pr merge *` | Deny | "Use pr_merge_with_gate for CI-gated merges" |
| build-mika | `run_shell` | `*` | Deny | "Use build_mika handler for backup/rollback" |
| deploy-mika | `run_shell` | `*` | Deny | "Use deploy_mika handler for backup/rollback" |
| qa-review, qa-review-build-callback | `run_gh` | `pr comment *` | Deny | "Use gh pr review for branch-protection compliance" |
| skill-review | `update_skill` \| `write_agent_file` | `*` | Deny | "Use review_skill — handles symlinks correctly" |

Implementation shape: `Vec<PolicyRule>` + `check_policy(session_context, tool_call) -> Allow | Deny(reason)`. ≤30 lines of dispatch-layer Rust plus the declarative rule list. Each rule's `reason` surfaces to the LLM as the refusal explanation, so the LLM doesn't have to guess what to do next.

### PR-review heuristic for schema creep

At rule review, check: is `arg_pattern` encoding literal argument shape, or encoding a semantic condition ("fetches CI status", "modifies a protected file") via clever matching? The latter is schema creep and should either split into multiple rules or escalate to schema extension. Same discipline on `session_kind`. A single comment explaining what a row "really means" beyond what the tuple expresses is the canary.

## Scope boundary — what the policy table is NOT

The policy table is `(session_kind, tool, arg_pattern) → Deny(reason)`. Two classes of guard do NOT fit and should stay in other layers:

- **Concurrency guards** ("don't call X while X is running"): require runtime state about whether another instance is active. Not argument-shape; belongs with the existing `long_running` framework primitive or an adjacent "single-inflight" mechanism.
- **Scope / output-sequencing guards** ("ZERO-NARRATION RULE", "handle ONLY the callback task", "don't scan backlog in webhook turns"): govern turn-sequencing and output shape, not tool-availability. Belong with turn-type-aware prompt assembly or output validation, not the policy table.

Attempting to cram either into the policy table is schema creep by definition — the tuple doesn't express the invariant.

## When to reach for this pattern

1. **Authoring a new skill prompt.** Before writing `"never call X"` or `"always use Y"`, ask: can this be a policy-table row instead? If yes, file both the prompt change and the rule. If no, document why (probably one of the two out-of-scope classes above).
2. **Adding a new framework-level tool guard.** Before writing imperative Rust in the dispatcher, ask: does this fit `(session_kind, tool, arg_pattern)`? If yes, it's a new row. If no, it's a new mechanism — and that's a bigger decision.
3. **Reviewing a tool that branches on response shape.** If the prompt has a `match action { "pass" => ..., "blocked" => ... }` structure and the tool can fail in ways the prompt doesn't enumerate, that's a tagged-union-return candidate. Make the tool's failure modes first-class variants, not string errors.
4. **Post-incident review of "LLM did the wrong thing."** Before concluding "the LLM hallucinated," check whether a prompt-vs-tool contract mismatch put the LLM in a position where no correct action was reachable. The incident may belong to this class rather than to LLM-behavior class.

## Related incidents and tickets

- **mika#485 (2026-04-08):** first Rule 6 violation — prompt-only admonition insufficient. PR merged with required CI check in FAILURE state.
- **mika#792 (2026-04-24):** second Rule 6 violation — same root cause, same prompt, same missing framework enforcement. Plus Shape C component (unstructured gate error).
- **Milestone #17 retry path (2026-04-24):** Shape A manifestation — self-dev retry instruction vs `long_running` callback guard.
- **Ticket (to be filed): `pr_merge_with_gate` tagged-union return + webhook-qa `gate_errored` branch.** Structural response to Case 2's Shape C component.
- **Ticket (to be filed): policy-table framework for tool-invocation forbids.** Structural response to Shape B. Seeded with 5 rules from grep audit.
- **Ticket (future milestone): framework-owned retry for pipeline-failure callbacks.** Structural response to Shape A — moves retry dispatch out of the LLM's tool-call responsibility into the task engine.

## Guidance recap

- Prompt admonitions are advisory, not enforceable. Treat `"never call X"` as documentation; never assume it prevents anything.
- Co-design every prompt-tool pair. The tool owns the taxonomy of outcomes (response variants, refusal reasons); the prompt owns the response policy (what to do per variant).
- String errors are a code smell at the tool-prompt boundary. Tagged unions are the fix.
- When the same admonition pattern repeats across multiple skill prompts (Rule 6 appears in three), it's a policy-table row waiting to be factored out.
- Runtime guards need runtime enforcement. Move them from prompts into the dispatcher via declarative rules.
