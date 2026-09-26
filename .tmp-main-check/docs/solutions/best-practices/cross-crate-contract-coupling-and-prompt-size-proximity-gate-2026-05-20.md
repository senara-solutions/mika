---
module: webhook-dispatch
date: 2026-05-20
problem_type: best_practice
component: tooling
severity: medium
tags:
  - cross-crate-contract
  - prompt-size-gate
  - structural-guard
  - ready-label-dispatch
  - shared-constant
  - ci-gate
applies_when:
  - Two crates share a string-literal contract (producer and consumer) with no compile-time coupling
  - A bundled skill prompt is growing toward its max_prompt_size cap
  - An engine-level intent guard has terminal rejection variants that future reviewers may misunderstand
---

# Cross-Crate Contract Coupling and Prompt-Size Proximity Gate

## Context

mika#852 hardened five structural weak spots in the webhook ready-label dispatch guard after mika#847 merged. The common thread: each was a case where a structural invariant existed only by convention (string literal duplication, manual cap monitoring, implicit assumptions) rather than by compiler or CI enforcement.

## Guidance

### 1. Share string-literal contracts via `mika-common`

When two crates must agree on a string prefix or format (producer emits it, consumer matches it), extract the constant to `mika-common` and have both sides import it. Then add an assertion in the producer's existing test that the formatted output `starts_with` the constant. This gives two failure modes on drift: compile error (symbol renamed) and test failure (format changed).

**Pattern:** `mika-common::github_event_format::READY_LABEL_DISPATCH_MARKER` is imported by both `mika-agent::webhook_dispatch` (consumer, via `pub(crate) use`) and asserted in `mika-gateway::github::tests` (producer). The format string in the gateway cannot be refactored without breaking the test.

**Key constraint:** `mika-agent` does NOT depend on `mika-gateway` (and should not). `mika-common` is the only valid shared surface for cross-crate constants between these two.

### 2. Gate prompt size at 95%, not just 100%

The `scan_skills_dir()` hard-skip at 100% of `max_prompt_size` is a cliff: the skill silently disappears from the agent's prompt, every keyword-matched handler section vanishes, and the agent reverts to bare-tool defaults. The 95% warn gate in `bundled_skills_load.rs` fires before the cliff, giving operators time to raise the cap, trim, or shard.

**Implementation detail:** The test walks `skills/bundled/` directly and reads each `skill.toml` to compute `effective_cap = min(manifest_max.unwrap_or(DEFAULT), CEILING)`. It does not go through `scan_skills_dir()` because that function only reports the failure side (skipped skills), not the proximity side.

### 3. Use dual-emission for counter-friendly log events

When adding a structured counter event to an existing human-readable log line, emit both: the counter-friendly event (stable name like `ready_label_dispatch_stall_total`) and the original message. This preserves backward compatibility for existing log-tailers and alert rules while enabling future `jq`-based aggregation.

### 4. Comment-harden predicates with exhaustive rejection lists

When a predicate's correctness depends on an unintuitive design choice (e.g., "attempts count, not successes"), enumerate every case that justifies the choice directly above the function body. A future reviewer reading the predicate will look at its doc comment first, not at a registry entry three modules away.

## Why This Matters

Each of these patterns defends against a specific class of silent regression: string-literal drift across crate boundaries, prompt size exceeding its cap unnoticed, missing aggregate signals for stall events, and cleanup PRs that "fix" an intentional design choice. The cost of each defense is minimal (a shared constant, a test, a log line, a comment) but the failure mode each prevents is a multi-hour incident.

## Examples

**Cross-crate constant (before/after):**

Before: Two crates each declare `"[GitHub] Issue labeled ready on "` as separate string literals.
After: One constant in `mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER`, imported by both.

**95% gate assertion:**

```rust
// In bundled_skills_load.rs
let ratio = actual as f64 / effective_cap as f64;
if ratio >= 0.95 {
    near_cap.push((skill_name, actual, effective_cap, ratio));
}
```
