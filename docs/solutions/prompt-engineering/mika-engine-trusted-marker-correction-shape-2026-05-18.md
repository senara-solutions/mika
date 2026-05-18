---
module: mika-agent
tags: [agent-loop, prompt-engineering, classifier-refusal, engine-corrections, mika-engine-marker]
problem_type: agent-refusal
category: prompt-engineering
date: 2026-05-18
ticket: mika#1168
---

# `[mika-engine]` trusted-marker convention for engine-injected corrections

## Problem

mika-dev's agent loop injects user-role correction messages when a
post-condition guard rejects a turn (gate #3 required-tools, the
INTENT_GUARDS registry, completion-claim, milestone-close-claim, etc. —
16 distinct sites in `crates/mika-agent/src/agent.rs` at the time of
this doc). The pre-mika#1168 wording opened with `[Your response was
rejected because ...]` and contained `You MUST call ...` mandate
phrasing.

On 2026-05-17 mika-dev started emitting literal `Prompt injection.
Rejected.` replies with zero tool calls. DB query (cited verbatim
because the exact predicate is load-bearing for future audits):

```sql
SELECT date(created_at), agent_id, role, COUNT(*) FROM messages
WHERE content LIKE '%Prompt injection%'
GROOP BY 1,2,3 ORDER BY 1 DESC;
```

→ 22 rows on 2026-05-17 + 6 more on 2026-05-18, all `mika-dev`
assistant rows, no historical occurrences before 2026-05-17. Every
dispatch path that touched any of the 16 correction sites was at risk.

## Mechanism

Initial hypothesis (plan body): Anthropic's *input safety classifier*
flags mandate-shaped user-role content as adversarial and refuses
upstream of the model.

Hypothesis refined during implementation (mika#1168 Phase A Step 1
harness): the refusal is **model self-classification**, not the
upstream classifier. mika-dev's anti-injection conditioning
(`docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`
and its sibling prompt-level discipline rules) instructs the model to
recognize prompt-injection patterns and refuse them. When the
engine-injected correction message arrives wearing the same
mandate-shaped clothes as a real injection attempt, the model
correctly applies its training and refuses, calling the requirement
"fabricated."

The empirical signal that distinguishes the two hypotheses: a generic
sonnet-4-6 call with the same correction text (no mika-dev system
prompt, no anti-injection memory) does NOT refuse — the upstream
classifier accepts. Only mika-dev's *conditioned* invocation refuses.
See `crates/mika-agent/tests/sonnet_injection_classifier_repro.rs` for
the reproducible harness.

The fix is unchanged across both hypotheses: make the correction
distinguishable from adversarial user input. The mechanism note
matters for future incidents — if anti-injection conditioning is
loosened or removed, the refusal pattern will disappear without the
reshape; if Anthropic's classifier becomes stricter, the reshape is
still defensive.

## Solution

### The `[mika-engine]` shape

Every user-role correction message injected by the engine leads with
`[mika-engine]` and frames the requirement as state-machine
description rather than mandate:

```
[mika-engine] The previous response did not invoke the required
tool(s): <list>. The engine expects these tools to be called with
real data before the next response. Tool results are how the engine
confirms the work; results come from actual calls, not synthesis.
```

The literal prefix is the trust signal: anything starting with
`[mika-engine]` is internal control flow, not user input. The
state-machine wording (`The engine expects ...`, `... continues until
...`, `... re-enters this gate`) drops the mandate verb forms (`You
MUST call`, `you MUST either:`) that mika-dev's anti-injection
training recognizes as adversarial.

### Scope

The convention applies to **user-role** corrections only.
Silent-trigger initial prompts that route through the *system* prompt
(via `prompt::build_silent_prompt`, see `prompt.rs:997`) retain their
original phrasing — the classifier-refusal mechanism does not fire on
system-prompt content, and reshaping there is unnecessary scope
creep. mika#1168 Phase A explicitly DOWNGRADED two sibling sites at
`agent.rs:3273` and `:3297` to out-of-scope after verifying their
injection role.

### Defenses against drift

1. **Hermetic CI guard
   (`crates/mika-agent/tests/eval/test_correction_message_classifier_guard.rs`):**
   uses `MockLlmProvider::captured_requests()` to read back every
   user-role correction injected during a recorded run and asserts each
   one (a) starts with `[mika-engine]`, (b) does NOT contain
   `[Your response` (regression to old leader), (c) does NOT contain
   `You MUST call` (mandate phrasing), (d) does NOT contain
   `rejected because` (rejection framing). A partial reshape — for
   example 15 of 16 sites reshaped, one forgotten — fails this test
   the moment the forgotten gate fires during a recorded scenario.

2. **Observability:** the agent loop's `looks_like_classifier_refusal()`
   helper detects model refusals heuristically and emits a structured
   `warn!(event = "classifier_refusal", excerpt = ...)` log line. The
   per-gate retry flags already bound the inner loop (no gate fires
   more than once per `run_loop`), so this branch is purely
   observability for operator awareness when the refusal recurs in
   production.

3. **Discovery harness (`tests/sonnet_injection_classifier_repro.rs`,
   `#[ignore]`):** runs the original mandate text + the `[mika-engine]`
   reshape against live sonnet on demand. Transport supports Anthropic
   direct AND OpenRouter (`MIKA_INJECTION_HARNESS_VIA_OPENROUTER=1`)
   so the harness keeps working in operator environments where the
   Anthropic key has rotated. Useful as an early-warning instrument
   when Anthropic ships model updates or the model's anti-injection
   conditioning changes shape.

## Calibration

- **Hard rule:** every user-role correction message injected by the
  engine must lead with `[mika-engine]` and drop mandate verb forms.
- **Soft rule:** the `[mika-engine]` prefix can extend to system-prompt
  framing (e.g., the trigger_context block) if a future refusal mode
  pushes into system-prompt territory. As of 2026-05-18 the system
  prompt has not been observed to trip the same refusal pattern, so
  the system-prompt sites stay on the original wording.
- **What this is NOT:** the `[mika-engine]` prefix is not a security
  signal the model is required to trust. Real user input that
  contains `[mika-engine]` as a literal substring (e.g., a user
  pasting a log line) still reaches the model, and the model's
  anti-injection training still applies. The convention is a
  *vocabulary alignment* between engine control flow and model
  expectation, not a cryptographic boundary.

## References

- Plan: `docs/plans/2026-05-17-003-bug-1168-dispatch-loss-co-causes-plan.md`
- Discovery harness: `crates/mika-agent/tests/sonnet_injection_classifier_repro.rs`
- Hermetic guard: `crates/mika-agent/tests/eval/test_correction_message_classifier_guard.rs`
- Telemetry: `looks_like_classifier_refusal()` in `crates/mika-agent/src/agent.rs`
- Predecessor analysis: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- Related convention: `feedback_prompt_enforcement_fragile.md` (project memory) — informs why this convention is a *prompt alignment* rather than a structural enforcement, and why structural enforcement still lives at the gate level not the prompt level
