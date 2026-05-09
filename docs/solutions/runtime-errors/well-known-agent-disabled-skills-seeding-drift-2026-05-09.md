---
module: mika-agent
tags: [well-known-agents, skill-overrides, seeding, drift, relay, dev-groom, required-suffix-line]
problem_type: runtime_error
category: runtime-errors
ticket: mika#1041
---

# Well-Known Agent disabled_skills Seeding Drift

## Problem

When a new skill is added to a well-known agent's `disabled_skills` denylist after the agent was first provisioned, the skill remains enabled because `seed_well_known_skill_overrides()` early-returns when any `skill_overrides` rows already exist. The original seeding-once path at the bottom of the function never runs for the new entries.

**Concrete impact:** mika-relay's denylist gained `dev-groom` (PR #845) after initial provisioning. On v0.10.0, `dev-groom`'s keyword `"groom"` substring-matched in mika-relay's permission payloads (which contain the parent task label "Groom mika#854"). The Required-suffix-line guard (#864) then rejected mika-relay's permission JSON for lacking a `Verdict:` suffix, causing kimi-k2.6 to respond with prose objections, which claude-pilot's relay parser couldn't parse as JSON. Every tool call auto-denied, producing $2.80 burned with zero commits.

## Root Cause

The seeding function had two branches:
1. **Overrides exist** (early-return branch): reconciled LLM overrides only, skipped `disabled_skills`
2. **No overrides** (first-creation branch): wrote both `disabled_skills` and LLM overrides

When `disabled_skills` was extended post-provisioning, branch 1 always ran and never wrote the new entries.

## Fix

Added a `disabled_skills` reconciliation pass inside the existing "overrides exist" branch, structurally parallel to the LLM-override reconciliation. For each skill in `spec.disabled_skills`, if no `enabled=false` row exists, one is written via `set_skill_enabled()`. Per-row error handling with `warn!` log on failure (fail-soft, same as LLM reconciliation). Summary `info!` log when any rows are reconciled.

**Reverse direction NOT reconciled:** If a skill is removed from the spec's denylist, its existing `enabled=false` row is preserved. This is intentional — operator manual disables (via `mika skills disable <name>`) must not be reverted on deploy. Operators can re-enable with `mika skills enable <name>`.

## Key Decisions

1. **Reconciliation over migration:** No DB migration needed. Uses existing `skill_overrides` table semantics (schema v24). The reconciliation runs on every startup and is idempotent.

2. **One-directional reconciliation:** Only positive-direction drift (new skill in denylist, not yet in DB) is corrected. Negative-direction drift (skill removed from denylist, still disabled in DB) is out of scope to protect operator intent.

3. **No changes to the guard itself:** The Required-suffix-line guard (`agent.rs:1472-1530`) and `collect_required_suffix_lines()` are correct — they're properly skill-scoped. The bug was in the seeding layer, not the enforcement layer.

## Verification

```bash
# Unit test
cargo test -p mika-agent well_known_agents

# Post-deploy DB state check
sqlite3 ~/.mika/data/mika.db \
  "SELECT skill_name, enabled FROM skill_overrides
   WHERE agent_id = 'mika-relay' AND skill_name IN
   ('dev-groom', 'mika-arch-groom-ticket', 'mika-arch-second-review',
    'mika-arch-groom-milestone');"
# Expect: 4 rows, all with enabled=0

# Confirm no guard re-prompts
grep "Required-suffix-line guard" ~/.mika/agents/mika-relay/logs/mika.log.* | tail -10
# Expect: zero hits since deploy timestamp
```

## Failure Class

**Seeding-once drift.** Any time a well-known agent spec evolves after initial provisioning and the seeding function uses an "exists → skip" pattern, new entries silently fail to apply. The reconciliation pattern added here is the general fix: compare spec against DB, write the delta.

## Related

- mika#864 — Required-suffix-line guard (the guard that exposed the drift)
- mika#845 — PR that added dev-groom to MIKA_RELAY.disabled_skills
- `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — provisioning architecture
- `docs/solutions/runtime-errors/mika-arch-opus-transient-error-chain-deadline-exceeded-2026-05-02.md` — prior LLM-override reconciliation (same pattern, different drift class)
