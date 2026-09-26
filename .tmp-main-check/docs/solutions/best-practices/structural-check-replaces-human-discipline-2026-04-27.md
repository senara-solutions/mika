---
title: When human-discipline mitigation fails N=3 for the same failure class, replace it with a CI-time structural check
date: 2026-04-27
category: best-practices
module: skills
problem_type: best_practice
component: development_workflow
severity: high
applies_when:
  - "A failure class has been mitigated with a 'review checklist' or 'always run wc -c' style human-discipline rule"
  - "The same failure class recurs after the discipline rule is documented"
  - "The failure mode is silent at runtime (warning, not error) and visible only in logs"
related_components:
  - testing_framework
  - tooling
tags:
  - structural-check
  - ci-gate
  - max_prompt_size
  - silent-drop
  - always_on
  - compound-discipline
---

# When human-discipline mitigation fails N=3 for the same failure class, replace it with a CI-time structural check

## Context

The bundled-skill `oversized prompt` silent-drop has now hit Mika three times. Each prior fix was a human-discipline rule that failed across the next round of growth:

| Date | Doc | Mitigation type |
|------|-----|-----------------|
| 2026-03-29 | `docs/solutions/integration-issues/skill-prompt-snippet-size-limit-configurable.md` | **Config**: introduced `max_prompt_size` per-skill override. Default 16KB, ceiling 64KB. (Necessary infrastructure — not a discipline rule by itself.) |
| 2026-03-29 | `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` | **Engine**: oversized `always_on` skills are now skipped at startup with an `error!` (loud-er, but still runtime-only). |
| 2026-04-17 | `docs/solutions/prompt-engineering/2026-04-17-always-on-skill-prompt-size-headroom.md` | **Discipline**: "set `max_prompt_size` with 40% headroom" + "review with `wc -c` vs `grep max_prompt_size` before merge". The doc itself flagged this: *"This is a footgun until we replace size policing at the config level."* |
| 2026-04-26 | (this incident, mika#827) | **Structural**: CI-time test asserts `scan_skills_dir(skills/bundled).skipped.is_empty()`. |

On the 2026-04-26 deploy, `self-dev` had grown from 33,868B (when the 04-17 doc was written) to 49,195B — past its declared 49,152B cap by 43 bytes. `qa-review` had grown to 35,008B against its declared 32,768B. `qa-review-build-callback` had no declared cap and was at 18,595B against the 16KB default. mika-dev came up without `self-dev` and mika-qa came up without `qa-review`. The autonomous loop ran in a degraded state until a human read the startup log.

The 04-17 `wc -c` review check was documented, lived in `docs/solutions/`, and was discoverable. It still didn't hold across nine days of prompt edits. (auto memory [claude] — `feedback_compound_infra_fixes.md` recommends compounding every infra fix and looking back for prior related fixes; this compound is that look-back made explicit.)

## Guidance

When a failure class has been mitigated with a human-discipline rule (review checklist, "always run X before merge", "monitor Y after every change") and the same failure class recurs, the next mitigation should be a **structural check that fires automatically** — not another, more careful, version of the same discipline.

Concretely for this class:

```rust
// crates/mika-agent/tests/bundled_skills_load.rs
use mika_agent::skills::index::scan_skills_dir;

#[test]
fn bundled_skills_load_without_oversized_prompts() {
    let scan = scan_skills_dir(&bundled_skills_dir());
    if !scan.skipped.is_empty() {
        // panic message names every skipped skill, its declared/default cap,
        // and the actual prompt size — so CI logs surface exactly which
        // skill.toml needs a max_prompt_size bump.
        panic!("...");
    }
}
```

Drives through the **same entry point production uses at startup** (`scan_skills_dir`), not internal helpers. Refactors that change the production scan path will refactor this test alongside them. That coupling is the point — the test must keep tracking the production behavior, not a snapshot of it.

The 40%-headroom advice from the 04-17 doc is **not** superseded — it's still good guidance for choosing the cap *value* when you bump it. What is superseded is the human-review enforcement of size compliance: that's what the structural check now does.

## Why This Matters

Human-discipline mitigations have a known half-life:

1. **They depend on review attention surviving each new contributor.** New people don't read `docs/solutions/` before their first PR.
2. **They depend on the failure being legible at review time.** A PR that adds 600 bytes of prompt text passes review easily; the cumulative-cap-crossing problem is invisible per-commit.
3. **They depend on the doc itself being current.** The 04-17 doc was current. It still didn't hold.

A structural CI check has none of those failure modes:
- Runs on every PR, every contributor, no exceptions.
- Failure message names the skill, the cap, and the actual size — no human inference required.
- Can't go stale; it tracks the production scan path by construction.

The cost of the structural check is one integration test (76 lines, single `scan_skills_dir` call, no API keys, no network). The cost of the discipline rule failing is the autonomous loop running degraded for an unknown window post-deploy.

Cite: `feedback_prompt_enforcement_fragile.md` (auto memory [claude]) — *"Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints."* Same principle applied one layer up — humans-reviewing-size-limits is the same shape of fragile-rule.

## When to Apply

- The same failure class has hit the codebase **two or more times after a mitigation was documented**.
- The current mitigation is a "remember to do X before merge" rule rather than an automated check.
- The failure surface is reachable from a production code path that can be exercised in a unit or integration test (i.e., the check doesn't require running the full system).
- The structural test would be cheap to write — single function call, no fixtures, no orchestration.

If the test would require building a fixture rig more elaborate than the bug, the discipline rule may still be the right call — but flag explicitly that the next recurrence should trigger a re-evaluation of "is the rig still too expensive?"

## Examples

**Before (discipline-based, from the 04-17 doc):**
```bash
# Reviewer's pre-merge check:
limit=$(grep '^max_prompt_size' skills/bundled/<name>/skill.toml | awk '{print $3}')
size=$(wc -c < skills/bundled/<name>/system_prompt.md)
[ "$size" -gt "$limit" ] && echo "OVER LIMIT"
```
Failure mode: reviewer doesn't run it, or runs it on the wrong skill, or a non-skill PR that nudges shared prose past the cap is reviewed without anyone running it at all.

**After (structural, this incident):**
```rust
// crates/mika-agent/tests/bundled_skills_load.rs (76 lines total)
let scan = scan_skills_dir(&bundled_skills_dir());
assert!(scan.skipped.is_empty(), /* descriptive message naming offenders */);
```
Failure mode: none discovered yet. CI runs on every PR; the assertion message tells the contributor what to fix.

**Bonus orthogonal lesson from this incident** (architect's pre-commit review on the plan): Unit 1's per-skill cap bumps should not encode Unit 3's ceiling-policy decisions. An earlier draft parked `self-dev` flush at the 64KB ceiling deliberately to "force the audit's hand" on ceiling policy. mika-arch flagged this as Unit 1 encoding Unit 3 assumptions — orthogonality violation per `docs/architecture/review-guide.md` § Orthogonality. Final cap landed at 57344 (~8KB headroom). The lesson generalizes: **don't use a Unit-1 fire-fix to pre-decide a Unit-N policy question, even if the policy question is in the same plan.** The fix is orthogonal to the policy.

**Bootstrap near-miss caught in this session:** the natural next step after grooming #827 was `mika ask --agent mika-dev "implement mika issue#827"`. But mika-dev is the agent broken by #827 (no `self-dev` skill loaded). Same shape as mika#825's PR body documents — *"the tool cannot fix itself; direct edits + commits + PR created manually."* When the agent that would normally implement a fix is broken **by** that fix, the dispatch path is closed; inline implementation in the worktree is the only path.

## Related

- `docs/solutions/integration-issues/skill-prompt-snippet-size-limit-configurable.md` — introduced `max_prompt_size` (2026-03-29)
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — engine-side loud failure for `always_on` (2026-03-29)
- `docs/solutions/prompt-engineering/2026-04-17-always-on-skill-prompt-size-headroom.md` — the now-superseded discipline check
- `crates/mika-agent/tests/bundled_skills_load.rs` — the structural replacement
- `crates/mika-agent/src/skills/index.rs:361` — `scan_skills_dir` entry point
- mika#825 — bootstrap lesson reference (*"the tool cannot fix itself"*)
- mika#827 — this incident
