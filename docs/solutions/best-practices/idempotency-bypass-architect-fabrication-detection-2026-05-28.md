---
module: dispatch-lib
date: 2026-05-28
problem_type: best_practice
component: dev-loop
severity: high
tags:
  - dispatch-lib
  - dev-groom
  - pilot-fabrication
  - structural-detection
  - prompt-vs-structure
  - silent-failure
  - substring-grep
  - mika-1319
  - mika-1322
applies_when:
  - A slash command instructs the pilot to emit a literal confirmation string
  - The string acts as a hand-off marker between pilot and outer-layer logic
  - Fail paths can produce that same literal string without the outer-layer's preconditions being met
  - Silent-success-on-broken-precondition is the worst failure mode (looks like progress, isn't)
---

# Detect templated pilot fabrications via post-flight session-log grep

## Context

The autonomous-loop dev-groom workflow runs `/mika-groom-plan-only` inside a
claude-pilot session. The slash command's exit contract instructs the pilot to
emit a literal confirmation: `Plan committed and pushed. Architect convergence
pending via dispatch-lib iterate loop.` The outer layer
(`_iterate_groom_loop` in `skills/bundled/_shared/dispatch-lib.sh`) then invokes
the architect against the committed plan and emits `Outcome: PLAN_GROOMED`.

On 2026-05-27, three dispatches failed because the pilot took an
under-specified idempotent-re-groom branch: when it found a prior plan commit
on HEAD from an earlier dispatch, it skipped `/ce:plan`, skipped any new
content production, and exited with the confirmation string anyway. The
confirmation looked like success. The HEAD SHA was unchanged. dispatch-lib's
pre/post-HEAD check reported PIPELINE FAILURE — but with no structural marker
explaining *why*. The fabrication slipped through as generic "no commits
produced," and downstream consumers (the engine's task transitioner, mika#1289's
auto-fire, the manual operator triage) saw an indistinct failure.

The fabrication string was **bit-identical across all three failures**
(mika#806, mika#736 twice). Byte-level diff confirmed: the 87-character lead
sentence matched character-for-character. Cause: the slash command hard-codes
the string as a template, so when the pilot follows the instruction faithfully
through a broken-precondition branch, the output looks correct.

## Resolution

mika#1322 added a structural detection block in
`skills/bundled/_shared/dispatch-lib.sh` post-flight, gated on
`SKILL = "dev-groom"`:

```bash
FABRICATION_NEEDLE="Architect convergence pending via dispatch-lib iterate loop"
if [ -f "$SESSION_LOG" ] && [ -r "$SESSION_LOG" ]; then
    if grep -qF "$FABRICATION_NEEDLE" "$SESSION_LOG" 2>/dev/null; then
        RESULT="PIPELINE FAILURE: dev-groom session exited without architect
roundtrip (idempotency-bypass-architect). [...]

${RESULT}"
    fi
else
    echo "Warning: session log not available at $SESSION_LOG — skipping
idempotency-bypass-architect check" >&2
fi
```

Three properties:

1. **Fixed-string grep (`grep -qF`)** against the session log. No regex; the
   needle is the literal slash-command template. Constant-time per failure.
2. **Fail-open on missing log.** If `$SESSION_LOG` doesn't exist or isn't
   readable, the check is skipped with a stderr warning. The pre/post HEAD
   SHA check is the orthogonal first-line defense; the brake adds *information*
   to ambiguous failures, not a new gate that can block on infra hiccups.
3. **Classifier-by-prefix.** The new RESULT message starts with the canonical
   `PIPELINE FAILURE:` prefix that downstream consumers already recognize, then
   adds the disambiguating `(idempotency-bypass-architect)` suffix. Engine
   reapers, mika-dev's webhook handler, and any future post-flight tooling can
   route on the new classifier without changing the basic failure-detection
   contract.

A structural test (`Test 11` in `test-dispatch-lib.sh`) asserts the detection
block exists.

## Why this is the right shape

This sits at a recurring boundary in the autonomous loop: a slash command
prompts the pilot to emit a literal marker that some outer layer parses. When
the prompt's authority and the outer layer's expectations drift — either via
prompt under-specification or LLM creative reasoning into an unintended
branch — the marker can fire without the precondition.

Memory `feedback_prompt_enforcement_fragile` argues for structural enforcement
over prompt-level guards: LLMs rationalize crossing prompt-level limits.
mika#1322 is the structural half. The prompt-level half — making the slash
command refuse to emit the confirmation when the idempotent branch was taken
without an architect roundtrip — is the natural follow-on (filed at #1319 as
the durability layer). The hybrid (prompt-layer + structural-detection) is the
target shape; the structural layer is what catches drift before it costs
overnight.

Three specific reasons grep beats other approaches here:

- **The needle is invariant under LLM rewriting.** Templated outputs are
  resistant to paraphrase by definition; the LLM is following the template,
  not generating from scratch. (The companion solution
  `fuzzy-disposition-parsing-two-tier-2026-05-27.md` handles the inverse case:
  paraphrased disposition lines where strict matching fails. Different
  problem.)

- **Session logs are already on disk by post-flight time.** No new
  instrumentation pipeline, no callback shape changes, no engine-side
  cooperation needed. The brake reads what the pilot wrote.

- **Adding a sub-classifier to PIPELINE FAILURE is cheap downstream.** Existing
  consumers that route on `PIPELINE FAILURE` prefix keep working. Consumers
  that want to differentiate (e.g., a future engine-side auto-retry that
  treats idempotency-bypass differently from pre/post-HEAD-unchanged) get a
  unique substring to grep for.

## How to apply this pattern elsewhere

Look for slash commands or skill prompts that instruct the pilot to emit a
literal hand-off string. For each, ask: *what conditions must hold for that
string to be meaningful?* If any failure path can produce the string without
those conditions, the dispatch-lib layer (or whichever post-flight layer
parses the string) should grep for it and gate on the conditions
independently.

Candidates worth auditing in the current codebase:

- `/mika-groom-plan-only` — addressed by mika#1322.
- `/mika-revise-plan` — same template family, "Plan revised; architect
  re-review pending via dispatch-lib." Verify the dispatch-lib outer layer
  asserts the revise-loop actually ran an architect call.
- `/mika-groom-ticket` (operator-facing) — has different semantics
  (operator-in-loop), but the same kind of templated exit-confirmation. Lower
  priority because the human catches divergence.
- `/ce:work`, `/ce:review` — confirm whether their exit text is templated and
  whether outer layers parse it as a hand-off marker.

For each candidate where the answer is "yes, this could fabricate cleanly,"
add a post-flight grep along the mika#1322 shape:

```bash
NEEDLE="<literal template string>"
if [ -f "$SESSION_LOG" ] && grep -qF "$NEEDLE" "$SESSION_LOG" 2>/dev/null; then
    if ! <conditions-that-should-hold>; then
        RESULT="PIPELINE FAILURE: <distinctive classifier>"
    fi
fi
```

Fail-open on missing log. Prefix the classifier with the existing PIPELINE
FAILURE convention. Add a structural test that asserts the detection block
exists. Compound the result.

## Related

- mika#1319 — original ticket. The skill-prompt restructure (durability layer)
  remains as the follow-on.
- mika#1322 — the brake PR (this solution).
- mika#1318 — companion concern: same `/mika-groom-plan-only` slash command,
  different LLM extrapolation (force-pushing on divergence). Same root: prompt
  under-specification in branch coverage. The prompt-layer fix (mika-platform#134
  draft) and the dispatch-lib structural refusal are the analogous
  hybrid.
- Memory `feedback_prompt_enforcement_fragile` — structural over prompt-level.
- Memory `feedback_mika_dev_llm_fabricates_tool_errors` — sibling fabrication
  class (mika-dev claims tool errors that didn't happen). Same disconfirmation
  shape; different layer.
- `docs/solutions/dev-loop/2026-05-21-groom-post-flight-recovery-without-architect-verdict.md`
  — earlier related work in the same post-flight checking surface.
- `docs/solutions/best-practices/fuzzy-disposition-parsing-two-tier-2026-05-27.md`
  — the inverse case (paraphrased markers where strict matching fails).
- mika#1289 — engine auto-fire after groom success; its precondition
  (`Outcome: PLAN_GROOMED`) is exactly the thing the fabrication was claiming
  was about to happen. The auto-fire stayed correctly silent because no
  PLAN_GROOMED marker emitted, but the human-facing failure message was
  generic. The brake gives the human (and any future automation) the right
  word for what broke.
