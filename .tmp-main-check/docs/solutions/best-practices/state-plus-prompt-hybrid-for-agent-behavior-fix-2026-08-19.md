---
module: mika-agent
tags:
  - agent-behavior
  - prompt-engineering
  - state-vs-prompt
  - user-signal
  - structural-vs-fragile
problem_type: agent-behavior-defect
category: best-practices
issue: senara-solutions/mika#1813
created: 2026-08-19
---

# Hybrid state + prompt fix for durable agent behavior — the `stop_topic_*` preference pattern

## The class

An agent behavior gets asked to change by the user. A prompt-only rule tells the model to change ("respect stop signals"). Under context weight, the rule erodes — the model forgets, the behavior recurs, the user has to repeat themselves (Al B, family testeur, 2026-07-20: had to say "arrête" multiple times on the same subject).

`feedback_prompt_enforcement_fragile` names the anti-pattern. `feedback_hard_evidence_before_addressing_mika_prime` and `feedback_verify_pipeline_passes_without_the_fix` name the discipline.

## The fix shape (hybrid, structural half wins)

Persist the user's directive as **state** in a place that is re-injected into the prompt on every subsequent turn. Add prompt discipline to (a) tell the model to write the state, and (b) tell the model how to consult it. The state layer is the structural gate — the block re-appears on every future turn even if the model forgets in-turn. The prompt layer is the intent alignment.

For mika#1813 ("Mika sur-relance après stop") the state was a new `stop_topic_*` key prefix on the existing `preferences` table (Layer 2 memory, per `crates/mika-agent/CLAUDE.md` § Three-Layer Memory Model). Existing `store_fact(category='preference', ...)` tool writes it; `search_preferences(STOP_TOPIC_PREFIX)` at turn assembly reads it; both `PromptContext` and `SilentPromptContext` inject a `<stopped-topics>` block on every subsequent turn. See `crates/mika-agent/src/prompt.rs` `STOP_TOPIC_PREFIX` const and `agent_loop::load_agent_context`.

## Why not the alternatives

- **Prompt-only rule.** Same class as `feedback_prompt_enforcement_fragile` — one refusal in the conversation, N turns later the model forgets. No re-injection = no structural memory of the directive.
- **New DDL / new table** ("suppressed_topics"). Ontology drift (the user isn't cancelling a commitment, they're refusing re-mention). Requires migration, new tool, new UI. Wrong side of the effort/impact curve.
- **Filter at the commitments query layer** (e.g., `list_commitments("pending", exclude_stopped: true)`). The re-nag surface is proactive suggestions in general, not commitment-shaped only. A prompt-level respect rule generalises; a query-level filter doesn't.

## Detection heuristics for "state-not-prompt" candidates

Apply the hybrid pattern (state + prompt) when **all** of these hold:

1. The user directive is **cross-turn** (must persist across the next N turns / heartbeats / sessions).
2. The directive is **suppressive** (don't do X) or **routing** (do X differently) — pure informational updates are covered by memory / core memory.
3. There is an **existing reusable state carrier** (preferences, core_memory, tasks/status) with an existing write path (a tool) and an existing read path (turn assembly).
4. The re-injection cost is bounded (small block, dozens of entries max).

When only 1-2 hold, prompt-only may be sufficient. When 4 fails, the state layer needs its own subsystem (new table + tool) — that's a bigger investment; surface it as a separate ticket.

## The load-bearing test

The regression fixture must fail on `main` and pass on the fix. For prompt-shape tests: assert a specific string (the persisted directive's content) appears in the assembled prompt. `main` cannot pass because the `stopped_topics` field does not exist on the context struct; the fix wires it. See `test_silent_prompt_regression_stop_topic_visible_and_rule_present` in `crates/mika-agent/src/prompt.rs`.

## Related

- `crates/mika-agent/CLAUDE.md` § Three-Layer Memory Model — Preferences pattern precedent (`task_policy_*` prefix)
- `feedback_prompt_enforcement_fragile` — the anti-pattern this compounds against
- `feedback_verify_pipeline_passes_without_the_fix` — the test discipline
- `feedback_never_call_real_broadcast_in_tmux_test` — respected via prompt-shape unit tests (no MessageSender constructed)
