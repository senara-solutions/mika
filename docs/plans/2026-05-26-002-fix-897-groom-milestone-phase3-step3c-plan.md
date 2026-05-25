# Plan: fix(commands) — mika-groom-milestone Phase 3 step 3c ITERATE-aborted-second-pass must not record READY

**Ticket:** mika issue#897
**Type:** bug fix
**Scope:** `.claude/commands/mika-groom-milestone.md` (this repo) + `mika-platform/.claude/commands/mika-groom-milestone.md` (canonical copy)

## Problem

Phase 3 step 3c's idempotency short-circuit gates on two conditions:
1. Per-ticket worktree exists with a committed plan
2. Issue body has `> - **Branch:**` callout

When both are true, it records the sub-issue as `READY` and skips re-grooming.

The bug: a sub-issue that went through ITERATE, had revisions committed, but whose second-pass was aborted (crash, timeout, organic LLM callout write before formal GROOMED verdict) can satisfy both conditions without ever reaching GROOMED. Recording it as READY corrupts the Phase 4 milestone-level brief input.

## Root Cause

The gating conditions are necessary but not sufficient. The `> - **Branch:**` callout can appear without GROOMED in two scenarios:
- The LLM pilot wrote it organically before the formal callout writer fired (documented gap in dispatch-lib line ~828)
- A previous run wrote it during finalization but crashed before the GROOMED metadata was fully recorded

## Chosen Approach

**Option 1 from the ticket** (simpler, sufficient): add a third gating condition requiring the `second-pass (GROOMED)` marker in the issue body. This marker is written only by `_write_canonical_callout` on verified GROOMED success, so its presence is a reliable signal.

Option 2 (read actual disposition from grooming-history callout) would be more general but adds parsing complexity for a case that's already bounded — the `GROOMED` marker is the authoritative signal, and without it the short-circuit should simply not fire (the sub-issue re-enters the per-ticket flow, which is idempotent itself).

## Changes

### Step 1 — Fix `mika/.claude/commands/mika-groom-milestone.md` step 3c

**File:** `.claude/commands/mika-groom-milestone.md` line 73

**Current text:**
```
**3c.** Short-circuit for already-groomed sub-issues (R5 idempotence): if the per-ticket worktree already exists with a committed plan AND the issue body already has the `> - **Branch:**` callout from a prior groom run, skip the per-ticket flow for that sub-issue. Record its prior disposition as READY (it was groomed in a previous run) and move on. This is the linkage R5 inherits from the per-ticket flow.
```

**New text:**
```
**3c.** Short-circuit for already-groomed sub-issues (R5 idempotence): if ALL THREE of the following are true, skip the per-ticket flow for that sub-issue and record its prior disposition as GROOMED:
  1. The per-ticket worktree already exists with a committed plan
  2. The issue body has the `> - **Branch:**` callout from a prior groom run
  3. The issue body has a `second-pass (GROOMED)` marker (written only by `_write_canonical_callout` on verified architect approval)

If conditions 1–2 are met but condition 3 is not, the sub-issue is mid-iteration (ITERATE with committed revisions but no architect GROOMED verdict). Do NOT short-circuit — re-enter the per-ticket flow so second-pass can complete. This is the linkage R5 inherits from the per-ticket flow.
```

Key changes:
- Three-condition gate (was two)
- Records disposition as `GROOMED` (was `READY` — accurate since condition 3 proves GROOMED)
- Explicit handling of the partial-state case (conditions 1–2 without 3)

### Step 2 — Propagate to `mika-platform/.claude/commands/mika-groom-milestone.md`

The canonical-propagated-prose-pair discipline (`mika-platform/docs/solutions/best-practices/canonical-propagated-prose-pair-discipline-2026-04-29.md`) requires the mika-platform copy to be updated in lockstep.

**However:** The issue notes that mika-platform#65 will merge the canonical copy. This fix targets the mika-side file. The mika-platform copy will be updated either:
- As part of mika-platform#65's merge (if it hasn't merged yet), or
- As a follow-up direct commit on mika-platform after this fix lands

**Decision:** This PR fixes `mika/.claude/commands/mika-groom-milestone.md` only. The mika-platform copy is out-of-scope per the ticket's "Affected files" note that mika-platform#65 handles the canonical copy.

## Verification

1. Read the updated step 3c and confirm the three-condition gate is clear and unambiguous
2. Confirm the ITERATE-aborted scenario is explicitly called out as "do NOT short-circuit"
3. Confirm no other references to step 3c elsewhere in the file need updating (check Phase 4's references to per-sub-issue dispositions)

## Risk Assessment

- **Low risk** — prose-only change to a slash command spec file, no Rust code
- **No behavioral regression** — the fix makes the short-circuit stricter (fires less often), never more permissive
- **Idempotent re-groom** — sub-issues that re-enter the per-ticket flow will either complete GROOMED (normal path) or ESCALATE (if the plan is fundamentally wrong), both correct outcomes
