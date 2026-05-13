---
title: Required-finding-list guard for conditional-disclosure-evasion prevention
date: 2026-05-13
category: architecture-patterns
module: agent-core, skills-engine
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - A skill requires the agent to emit structured findings (F1:, F2:, etc.) in its final response
  - The agent persists findings to memory but emits only a thin summary in the assistant message
  - Downstream consumers depend on in-band findings emission rather than memory lookups
tags: [engine-guard, structural-enforcement, skill-output, finding-list, conditional-disclosure, mika-arch, grooming]
---

# Required-finding-list guard for conditional-disclosure-evasion prevention

## Context

The mika-arch grooming skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) allow the architect to perform multi-turn investigation and emit findings (F1, F2, ..., Fn). However, the agent consistently drifted toward persisting findings to memory via `store_fact`/`update_core_memory` while emitting only a thin acknowledgement in the final assistant message. This is the **conditional-disclosure-evasion** failure class (N=8 observed incidents as of 2026-05-13).

The downstream operator (`/mika-groom-ticket` Phase 4 step 11) depends on in-band findings to iterate on plans. Without them, the operator must make follow-up LLM calls or query the agent DB directly, costing ~$1-2 per ticket in additional Opus spend.

Prompt-level enforcement ("MUST emit findings") drifted repeatedly under cognitive load, consistent with the `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` precedent.

## Guidance

### Engine guard pattern

Add a `required_finding_list_prefixes` field to the `[output]` section of `skill.toml`, mirroring the existing `required_suffix_lines` pattern from mika#864:

```toml
[output]
required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]
required_finding_list_prefixes = ["F1:", "F2:", "F3:", "F4:", "F5:", "F6:", "F7:", "F8:", "F9:", "F10:"]
```

The engine guard (`agent.rs`) scans the assistant's message body for any line starting with a declared prefix. The scan range is from message start up to (exclusive of) the suffix-line landmark (e.g., `Disposition: ITERATE`). This composes with the existing suffix-line guard — the suffix line is the scan terminator, not a parallel parameter.

### Conditional enforcement

The guard fires only on **terminal dispositions** (ITERATE, ESCALATE, Verdict: ESCALATE). Non-terminal dispositions (READY, GROOMED) are exempt — per operator spec, short messages are acceptable when no iteration is needed. `is_terminal_disposition()` detects the disposition by checking the last 3 non-empty lines against both the skill's declared suffix lines and a `TERMINAL_DISPOSITIONS` constant.

### Prefix matching

Uses literal `starts_with` matching against a closed-alphabet `Vec<String>` — no regex, per the anti-regex precedent from mika#864 ("regex is a footgun — silent failure to fire when pattern is malformed"). The F1:-F10: bound is observably sufficient; future bump is a discoverable sentinel.

### Single-retry semantics

Matches mika#864's pattern: one corrective re-prompt per turn. If the model fails twice, the second response is accepted. The retry flag (`required_finding_list_retry_done`) is independent of the suffix-line retry flag.

## Why This Matters

Prompt-level "MUST emit findings" rules drift under cognitive load — this is the eighth documented incident of the same failure class. Structural engine-side enforcement eliminates the rationalization vector entirely. The guard composes cleanly with the existing post-condition chain and requires no new DB schema or API surface.

## When to Apply

- When a skill requires structured enumerated output (F-list, checklist, per-item verdict) in the final assistant message
- When downstream consumers parse the assistant's response for structured content
- When prompt-level enforcement has failed 3+ times (the structural-ratchet threshold from `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`)

## Examples

### Skill.toml declaration

```toml
[output]
required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
required_finding_list_prefixes = ["F1:", "F2:", "F3:", "F4:", "F5:", "F6:", "F7:", "F8:", "F9:", "F10:"]
```

### Guard-satisfying response (Verdict: ESCALATE)

```
F1: (BLOCKING) Prior finding F2 unresolved.
   Concern: Revision defers to follow-up ticket.
   Change required: Resolve in this plan.
   Citation: review-guide.md § Single Responsibility

Verdict: ESCALATE
```

### Guard-exempt response (Verdict: GROOMED)

```
All prior findings resolved. The plan is ready for implementation.

Verdict: GROOMED
```

## Related

- mika#901 — the issue this fix addresses
- mika#864 — the `required_suffix_lines` guard this pattern mirrors
- `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the structural-ratchet precedent
- `crates/mika-agent/src/agent.rs` — guard implementation (`is_terminal_disposition`, EndTurn chain position 9)
- `crates/mika-agent/src/skills/manifest.rs` — `Output` struct with `required_finding_list_prefixes` field
