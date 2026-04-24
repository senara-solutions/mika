---
title: "Intent signal is not completion signal — verify post-state before advancing"
date: 2026-04-24
category: best-practices
module: self-dev
problem_type: best_practice
component: development_workflow
severity: high
applies_when:
  - Agent workflows that chain state transitions (merge → deploy → next ticket)
  - Any prompt step that reacts to a tool output suggesting "success" vs reading the post-state
  - Milestone or multi-phase flows where completion of phase N gates phase N+1
tags:
  - verify-post-state
  - state-transitions
  - self-dev
  - milestone-workflow
  - grounding
  - agent-discipline
  - intent-vs-completion
---

# Intent signal is not completion signal — verify post-state before advancing

## Context

Agent workflows that chain state transitions routinely fail the same way: the agent reads a tool response announcing that a transition has been **authorized, enabled, or initiated**, and advances as if the transition has already **completed**. The two signals look similar in natural language — "auto-merge has been enabled" vs "PR merged"; "parent task updated to completed" vs "GitHub milestone closed"; "deploy triggered" vs "deploy succeeded" — but they mean different things, and conflating them causes downstream work to start against a state that hasn't actually landed yet.

Two instances of this failure mode were fixed on 2026-04-24 in the self-dev milestone workflow (mika#789):

- **M4** (milestone execution loop, formerly mika#727): after the structural verdict handler enabled auto-merge on a QA-passed PR, the LLM read "auto-merge has been enabled for PR #N" and immediately dispatched the next ticket. CI later failed on the queued merge, the PR never actually landed on `main`, and the next ticket's claude-pilot session ran against code that didn't exist on main yet.
- **M5** (milestone close-out, new gap surfaced 2026-04-24): after `update_task_status(milestone_wi, completed)` on the internal task record, the workflow treated the milestone as "done" and stopped. The GitHub milestone resource itself was never closed — 4 children merged, `milestone.closed_at == null` for hours, until a human ran the close call manually.

Both are the same class: **an intent signal was treated as a completion signal.** The fix in both cases was the same shape — verify post-state before advancing.

## Guidance

When a workflow step announces a state transition, identify whether the tool output reflects:

| Signal | Example strings | Correct treatment |
|---|---|---|
| **Intent** (authorized, initiated, enabled, triggered) | `"Auto-merge has been enabled for PR #781"`, `"Deploy started"`, `"Task submitted (long-running)"`, `"Task {X} marked complete"` (when X is a local task record, not the external resource) | **Hold.** Wait for the post-state to confirm before advancing. |
| **Completion** (verified in the external system's authoritative state) | `gh pr view --json state` returns `"MERGED"`; `gh api /repos/.../milestones/{N}` returns `state=closed`; deploy health check returns `200 OK` | **Advance.** The transition has landed. |

The discipline is an explicit post-state verification step between the transition trigger and any downstream work:

```
Trigger transition → [INTENT SIGNAL] → Verify post-state → [COMPLETION SIGNAL] → Advance
                                        ↑
                                        this step is the bug when it's missing
```

For each state transition in a workflow prompt, the verification step should:
1. Query the authoritative external state (not a cached tool-output message)
2. Treat anything other than the expected completion value as a **hold**, not a failure — advancing would be premature, but failing would be noise
3. Only advance when the external state matches the expected completion value

### Concrete code shape

For M4 (merge verification), the prompt now reads:

```
After the verdict handler enables auto-merge, MUST call:
  run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")

Only treat `state == "MERGED"` as success. `auto-merge enabled` is a HOLD state:
the child task stays `in_progress`, no next-ticket dispatch happens, and the LLM
returns control to the webhook-handling path. The webhook chain
(`check_suite.completed(success)` → `ci_success_handler` → `pull_request.closed(merged)`)
is what fires the eventual completion signal.
```

For M5 (milestone close), the prompt now reads:

```
After `update_task_status(milestone_wi, completed)`, MUST close the GitHub milestone itself:
  run_gh(command=["api", "--method", "PATCH",
                  "/repos/{owner}/{repo}/milestones/{number}",
                  "-f", "state=closed"],
         repo=None)

where {owner}/{repo} is spelled literally inside the URL path (not via --repo flag),
{number} is the milestone number captured at M1 time.
```

Both verifications query the external system's authoritative state. Neither infers completion from an earlier message claiming the transition was initiated.

## Why This Matters

### 1. The LLM's default is to trust the previous tool output

LLMs are optimized for narrative coherence. When a tool output says "auto-merge enabled," the natural narrative completion is "therefore merged, therefore advance." The model has to be explicitly instructed to distrust the intent signal and go re-verify the post-state — otherwise it takes the shortest path from intent to advance.

This is the same class of failure as `feedback_mika_dev_llm_fabricates_tool_errors` (auto memory [claude]): the LLM generates the most coherent continuation of the conversation rather than grounding in actual verified state. The mitigation is identical in shape: force an explicit verification tool call before any claim or advance.

### 2. Intent and completion diverge in expected failure modes

The whole reason intent signals exist is that the transition has a latency or a failure mode. `gh pr merge --auto` enables but doesn't merge because CI hasn't passed yet. `update_task_status` updates internal state but doesn't touch the external GitHub resource. If intent and completion always coincided, the distinction wouldn't matter — the bug is specifically that they don't coincide, and the cases where they don't are the exact cases the workflow needs to handle correctly.

### 3. Prompt-level verification is fragile but cheap; structural guards are the real fix

Per `feedback_prompt_enforcement_fragile` (auto memory [claude]): LLMs rationalize around prompt rules. A prompt step saying "verify state before advancing" is necessary but not sufficient — the model will sometimes skip it under compaction or when the previous tool output feels decisive.

The belt-and-suspenders pattern:

- **Prompt-level verify-post-state** rule in the workflow prompt (cheap to ship, catches most cases)
- **Engine-level structural guards** where the cost of LLM skipping the rule is high — e.g., the dispatch-readiness guard (mika#525), the intent-precondition registry (mika#702), `pr_merge_with_gate` as a tool that refuses to merge when CI isn't green
- **Webhook chains** that re-drive the correct completion path regardless of what the LLM did — e.g., `ci_success_handler` + `verdict_handler`

The prompt is the first line of defense. The structural guards are the backstop. Both layers are needed; neither alone is sufficient.

### 4. `feedback_qa_advisory_ci_gate_on_dev` was already recorded — mechanization was the gap

The M4 version of this bug was actually noted in auto memory before 2026-04-24: *"QA is advisory; mika-dev enforces CI gate client-side, uses `gh pr merge --auto` when checks pending"* (auto memory [claude]). The recorded guidance was correct. The failure was translating that guidance into a concrete verification step inside the prompt's M4 section. Recording a principle isn't the same as mechanizing it. Compound docs need to include the specific prompt/code shape, not just the conceptual rule.

## When to Apply

The verify-post-state pattern applies any time a workflow step involves:

1. **A tool output that uses authorization vocabulary** — "enabled," "triggered," "initiated," "submitted," "queued," "marked," "scheduled." These words are reliable tells that the signal is intent, not completion.
2. **A multi-step transition across systems** — local state update followed by external resource mutation (internal task status + GitHub milestone; local commit + remote push; DB write + downstream webhook processing).
3. **A workflow phase that gates downstream work** — phase N's completion must be real before phase N+1 can safely run. (This is specifically what milestone workflows, deploy pipelines, and dependent-ticket dispatches are.)

It does **not** need to apply when:

1. The tool is synchronous and its output is the authoritative completion signal (a successful `SELECT` returning data; `create_task` returning a task ID).
2. The downstream work can safely proceed speculatively and will harmlessly re-drive if the upstream state wasn't actually reached (rare in stateful workflows, common in idempotent retries).
3. Intent is the only thing the workflow cares about (notification dispatch where "sent to gateway" is the accepted contract).

## Examples

### Example 1: M4 auto-merge (before → after)

**Before (intent treated as completion):**

```
# M4 loop — check child outcome
1. Read verdict handler output.
2. If output contains "auto-merge has been enabled" → mark child merged, dispatch next.
3. Else if output contains "VERDICT: block" → hold milestone, notify Vincent.
```

**Failure mode:** PR #726 had `cargo-audit` flagging `rand 0.8.5 RUSTSEC-2026-0097` after auto-merge was enabled. Auto-merge never fired. Next ticket dispatched against code that wasn't on main. mika#740's claude-pilot session worked against a stale base.

**After (verify post-state):**

```
# M4 loop — check child outcome
1. Read verdict handler output.
2. If output contains "auto-merge has been enabled":
   a. Call run_gh(["pr", "view", <num>, "--json", "state,mergedAt"]).
   b. If state == "MERGED" → mark child merged, dispatch next.
   c. If state == "OPEN" → hold; log "PR #N: auto-merge pending CI. Holding milestone until merge confirms."
      Wait for the check_suite.completed(success) → ci_success_handler → pull_request.closed(merged) webhook chain.
3. Else if output contains "VERDICT: block" → hold milestone, notify Vincent.
```

### Example 2: M5 milestone close (before → after)

**Before (local-state update treated as completion):**

```
# M5 close-out
1. Gather stats from children.
2. Build + deploy.
3. update_task_status(milestone_wi, completed).  # internal task record
4. store_fact(...).
5. Notify Vincent.
```

**Failure mode:** mika milestone#15 had 4 children closed. Internal task marked completed. GitHub milestone resource left at `state=open, closed_at=null` for hours until a human ran `gh api PATCH` manually.

**After (verify post-state — which here means *create* the external state):**

```
# M5 close-out
1. Gather stats from children.
2. Build + deploy.
3. update_task_status(milestone_wi, completed).  # internal task record
4. Close the GitHub milestone itself:
   run_gh(command=["api", "--method", "PATCH",
                   "/repos/{owner}/{repo}/milestones/{number}",
                   "-f", "state=closed"],
          repo=None)
5. If the close call fails, surface the error and hold. Do NOT swallow and proceed.
6. store_fact(...).
7. Notify Vincent.
```

The phrasing "verify post-state" is slightly awkward for M5 because the step is really "*create* the external state and only then consider the phase complete." Same underlying principle: don't treat internal state transitions as completion of external-system transitions.

## Related

- mika#789 (this fix): consolidated verify-post-state pattern for M4 and M5
- mika#727 (closed, subsumed by #789): original M4 premature-advance report
- mika#732 (closed, subsumed by #789): current_priorities memory hygiene (separate concern, same section)
- mika#788: `run_gh` allowlist fix — blocked the M5 call until `api` was added to `GH_ALLOWED_SUBCOMMANDS` (and removed hallucinated `milestone`/`project` entries)
- Auto memory: `feedback_qa_advisory_ci_gate_on_dev`, `feedback_verify_before_claiming`, `feedback_prompt_enforcement_fragile`, `feedback_mika_dev_llm_fabricates_tool_errors`
- Engine-level structural guards that apply the same principle: mika#525 (dispatch-readiness guard), mika#702 (intent-precondition registry), `pr_merge_with_gate` tool
