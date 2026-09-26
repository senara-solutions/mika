---
module: mika-agent/server
tags: [intent-guards, ready-label, dispatch, pre-digest, structural-enforcement, assert-grounded, completion-claim]
problem_type: guard_interaction
category: best-practices
---

# Engine-Injected Pre-Digests Must Be Guard-Safe on BOTH the User-Message and Assistant-Text Surfaces

## Problem

The structural ready-label dispatch handler (`server/ready_label_handler.rs`)
intercepts a `[GitHub] Issue labeled ready on <repo>#<n>` webhook **before** the
LLM turn and replaces the user message (`req.text`) with a pre-digest. mika#1384
shipped this as the "pragmatic" Option Α (#1571): a *prescriptive* pre-digest
that names the exact tool + args + pre-created `task_id` and tells the LLM to
make the `run_claude_pilot[_groom]` call.

It did not bind: **0 dispatches across 5 fresh post-deploy webhooks** on
`claude-sonnet-4-6`, n≥6 incidents historically. The instinct ("the
`webhook_ready_label_dispatch` INTENT_GUARD fires repeatedly, so the LLM keeps
being told to comply") was half-right and hid the real failure.

## Root cause — a prefix change silently disarmed the safety guard

`is_ready_label_dispatch_marker` (the `webhook_ready_label_dispatch` guard's
trigger) is a pure `starts_with(READY_LABEL_DISPATCH_MARKER)` predicate, and the
post-handler guard reads the **replaced** `req.text` (`handlers.rs`:
`user_message: &req.text`). #1571's pre-digest starts with
`<ready_label_handler>`, **not** the marker — so when the handler *succeeds*, the
guard cannot fire. The LLM receives a prescriptive instruction, silently skips
the tool call, and EndTurns with nothing to catch it. The "guard fires
repeatedly" evidence came only from the **degraded passthrough paths** (no
github token / body-fetch fail / parse fail), where `req.text` stays the raw
marker. #1571 thus simultaneously (a) prescribed the call and (b) removed the
post-hoc safety net — the worst of both.

This is another datapoint for the standing substrate lesson: **prompt-level
enforcement of a structural requirement is fragile; make the requirement a
property of the engine, not the LLM's choice.** (See
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.)

## Fix — engine-side dispatch (mika#1572, full Option Α)

The handler now spawns the dev-pilot/dev-groom subprocess **directly**
(`SkillRegistry::resolve_tool_by_name` -> `validate_dispatch_readiness` ->
`build_callback_task` -> `spawn_long_running_exec`) before the LLM turn, then
injects a *post-dispatch* pre-digest (`VerdictAction::Dispatched`) that says "the
dispatch already fired — acknowledge and EndTurn." LLM compliance becomes
irrelevant. On any precondition failure it falls back to #1571's prescriptive
pre-digest (so the degraded path is unchanged).

## The non-obvious trap: two distinct guard surfaces

A pre-digest is the **user message**, but it is consumed by an LLM whose **reply**
is then scanned by a *different* class of guards. An engine-injected pre-digest
must be safe on **both**:

1. **User-message / marker-keyed INTENT_GUARDS** (`webhook_ready_label_dispatch`,
   `webhook_zero_tools`, `webhook_no_unauthorized_dispatch`,
   `callback_trigger_active`). These are all `starts_with` predicates on
   `req.text`. **Starting the digest with `<ready_label_handler>` disarms all of
   them by construction** — no flag threading, no `AgentParams` field. (Verified
   against every predicate; the `[GitHub]` substring on line 2 is inert because
   the checks are `starts_with`, not `contains`.)

2. **Assistant-text guards** that scan the LLM's *acknowledgment* on the next
   turn. The digest's *wording* can prime the reply into a claim these catch,
   costing a wasted (self-healing) retry on every dispatch:
   - **completion-claim guard (#483)** — `detect_completion_claim` matches
     `\b(merged|deployed|completed?|shipped)\b`. A header reading "DISPATCH
     COMPLETE" primes the model to echo "complete". Use "DISPATCH **FIRED**".
   - **assert_grounded guard (#1331)** — matches affirmative issue-state claims
     ("issue #N is ready/groomed"). Its correction tells the model to call
     `run_gh` to ground the claim — which **directly contradicts** a dispatch
     digest's "MUST NOT call run_gh". Steer the optional `send_message` ack to a
     **grounded action statement** ("a dispatch subprocess was launched for this
     ticket") and explicitly forbid asserting issue state.

The fix removes the priming in the digest text and pins it with unit tests
(header carries no completion keyword; digest contains the no-state-assertion
line).

## Takeaways

- A guard whose trigger is `starts_with` on a message the handler **rewrites** is
  trivially (and sometimes *accidentally*) disarmed by the rewrite. When you
  change what a pre-handler injects, re-check every guard keyed on that text.
- Engine-injected pre-digests have a **second** review surface: the wording
  primes the assistant-text guards (completion-claim, assert_grounded,
  fabricated-action) on the reply turn. Word them to avoid completion keywords
  and affirmative resource-state claims.
- When replicating a dispatch path engine-side, the callback child must be
  **structurally identical** to the LLM tool-call path — extract a shared builder
  (`build_callback_task`) and bind the same inputs (notably
  `estimated_duration_secs`, not a hardcoded default, or the callback's
  `timeout_at` silently halves and a healthy long run is killed mid-flight).
