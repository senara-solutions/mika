---
title: "Verification claims must include expected output shape, not just commands"
date: 2026-04-28
category: best-practices
module: mika-arch, plan-templates
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Authoring an implementation plan with a "verification" or "smoke test" section
  - Reviewing a plan that lists commands without expected outputs
  - Architect-pass review (mika-arch first/second pass) of a plan
  - Agent workflows that gate on "did the verification pass" without defining what passing looks like
related_components:
  - architect-review
  - plan-template
tags:
  - verification
  - plan-discipline
  - architect-review
  - expected-output
  - assertion
  - grounding
  - mika-arch
---

# Verification claims must include expected output shape, not just commands

## Context

Across one mika-arch grooming session in late April 2026, four separate plans (mika#52, mika#636, mika#665, mika#663) shipped with verification sections that listed shell commands without specifying what success looked like. Each plan contained lines like:

> **Verify:** run `cargo test -p mika-agent --test eval`

with no statement of what the test output should contain, what counts as "passing," or how a reviewer would distinguish a clean run from a silent failure. The architect surfaced the same finding on each pass: a command without an expected-output assertion is ritual, not verification.

The N=4 in a single session is what promoted the pattern from "occasional" to "systemic." Each instance individually felt like a small omission; collectively they revealed that the plan template itself didn't have a shape that forced verification claims to include their expected outputs.

## The Problem

A verification step is supposed to be a *check*: a statement that some observable signal will hold after the change lands. "Run `cargo test`" is not a check — it's an invitation to read the screen and decide for yourself whether the screen looks right. That works when the human runs it; it fails silently in three concrete ways:

1. **Silent regression.** A test suite that compiles but skips the relevant test, or runs and fails in a way that wasn't caught (warning instead of error, exit-zero on a logically broken state) passes the "run the command" gate without producing the actual signal.
2. **Agent-side execution.** When an agent (mika-dev, claude-pilot) executes the verification step, it has no model of what the output should look like. It runs the command, sees output, and *narrates* whether it looks successful — which is exactly the fabrication failure mode covered in `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` and `741-grounding-fabrication-regression-scenarios.md`. Without an expected-output shape, the agent's narration is unanchored to a check.
3. **Reviewer time-cost.** A PR reviewer reading "Verify: run `cargo test`" has to either run it themselves or accept the author's claim that it passed. Both are expensive. The expected-output assertion lets the reviewer read the PR and decide whether the assertion is *the right assertion* — a much faster check than re-executing.

## The Pattern

Every verification step in a plan must specify both:

- **The command** to run (what was always there).
- **The expected output shape** the command should produce on success — concrete enough to assert against, abstract enough not to depend on incidental output (timestamps, ordering of logs, etc.).

The expected output shape is the *assertion*. The command is the *experiment*. A plan that has experiments without assertions has no verification.

### Example shapes

**Bad** (command-only, ritual):

> Verify: `cargo test -p mika-agent --test eval`

**Good** (command + expected output shape):

> Verify: `cargo test -p mika-agent --test eval` → expected: final summary line `test result: ok. N passed; 0 failed; 0 ignored` where `N >= <baseline>`. Any `failed` count > 0 fails the check.

**Bad** (smoke test, no shape):

> Smoke: `mika ask --agent mika-dev "use run_gh api"` should not be rejected.

**Good** (smoke test, positive shape):

> Smoke: `mika ask --agent mika-dev "use run_gh api GET /repos/senara-solutions/mika"` → expected: tool result is a non-empty JSON object containing `name`, `full_name`, `id` fields. Absence of allowlist rejection is *necessary but not sufficient* — the call must reach gh and gh must return valid JSON.

**Bad** (post-deploy check, narrative):

> After deploy: confirm mika-spirit picked up the change.

**Good** (post-deploy check, observable):

> After deploy: `tail /var/log/mika/server.log | grep "skill_registry_loaded"` → expected: a single INFO line within 30s of restart with `loaded=N` matching the new skill count. Absence of the line OR `loaded` count regression fails the check.

## Why This Matters Beyond mika-arch

The discipline isn't architect-specific. It's the same shape as test assertions in code: an experiment without an assertion isn't a test, it's just code that runs. The reason it surfaces *first* in mika-arch's reviews is that the architect is reading plans before they execute — which is exactly the moment when the verification section is most malleable. By the time a plan is mid-execution and an agent is running the commands, the absence of expected-output shape has already become a fabrication-risk vector: the agent narrates "it worked" because nothing tells it what "worked" looks like.

The pattern composes cleanly with the broader grounding discipline (cite the doc, include the line number, state what the tool returned). All three are versions of the same rule: **claims must be checkable against observable evidence, not produced from narrative inference**.

## Application

When authoring a plan, every verification step gets two lines:

```
- Run: <command>
- Expected: <observable shape>
```

When reviewing a plan (architect-pass or PR review), reject verification steps that have only the first line. Treat the missing assertion as a hard finding, not a sharpening — without the assertion, the plan has no exit criterion for the verification phase.

When executing a plan as an agent, refuse to mark a verification step "passed" without quoting the actual observed output and showing how it matches the expected shape. If the expected shape is missing from the plan, halt and ask — don't infer.

## Citations

- mika#52 — verification step listed `cargo test` without output shape; architect first-pass surfaced.
- mika#636 — verification step listed deploy command without post-deploy log assertion; architect first-pass surfaced.
- mika#665 — verification step listed smoke test without expected JSON shape; architect first-pass surfaced.
- mika#663 — verification step gated on "no allowlist rejection" (negative-only assertion); architect second-pass surfaced that absence-of-rejection isn't a positive signal.
- `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — broader fabrication-under-load failure mode this pattern guards against.
- `mika/docs/solutions/741-grounding-fabrication-regression-scenarios.md` — eval scenarios for the fabrication class this pattern prevents.
