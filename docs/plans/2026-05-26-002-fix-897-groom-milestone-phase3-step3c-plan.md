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

**Option 1 from the ticket** (simpler, sufficient): add a third gating condition requiring the `second-pass (GROOMED)` marker in the issue body. This marker is written only by `_write_canonical_callout` (`skills/bundled/_shared/dispatch-lib.sh`, lines 812–913) on verified GROOMED success, so its presence is a reliable signal.

**Marker integrity (F2 citation):** `_write_canonical_callout` writes the marker string `second-pass (GROOMED)` in the `history_line` variable (dispatch-lib.sh lines 845, 848) as part of the `> - **Grooming history:**` callout line. The function is called exclusively from `_iterate_groom_loop` (lines 980–986 for ready-to-groomed, lines 1029–1035 for iterate-to-groomed) only when `verdict == GROOMED` — all non-GROOMED branches (lines 988–992, 1037–1045) skip the call. The function is the sole structural writer of this marker, as documented in dispatch-lib.sh lines 814–822: the Class D recovery shim (`_verify_and_write_body_callout`) was retired in sub-PR 7b. The idempotency guard (lines 874–883) also checks for this exact string before writing, confirming the marker string is stable. No other codepath writes `second-pass (GROOMED)` to the issue body.

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

**Disposition value validity (F3 citation):** `GROOMED` is already a valid per-sub-issue disposition in `mika-groom-milestone.md`. Step 3b (line 68) lists valid dispositions as "READY, ITERATE, ESCALATE, or GROOMED after second-pass." Step 10 (line 78) accepts `GROOMED` alongside `READY` for the all-good aggregate: "All sub-issues READY or GROOMED → milestone disposition READY." Phase 4's step 11 (line 97) passes per-sub-issue dispositions as informational fields in the milestone-level brief — they are not pattern-matched as an enum by the architect skill. The `READY` → `GROOMED` change aligns with the value already documented in step 3b.

**Partial-state re-entry path (F4 citation):** When conditions 1–2 are met but condition 3 is not, re-entry targets the full per-ticket groom flow via step 3a ("directly following the phases in `.claude/commands/mika-groom-ticket.md`"). The mika-groom-ticket flow itself handles first-pass vs. second-pass routing internally: it detects the existing branch, existing committed plan, and existing first-pass session state to determine whether to run a fresh first-pass or continue to second-pass. Specifically, the per-ticket flow's Phase 2 checks for an existing worktree + plan and skips plan creation if found; Phase 3 checks for an existing first-pass disposition and routes to second-pass if the prior was ITERATE with committed revisions. The re-entry via step 3a is therefore correct — mika-groom-ticket.md's own phasing handles the routing, not step 3c.

### Step 2 — Propagate to `mika-platform/.claude/commands/mika-groom-milestone.md`

The canonical-propagated-prose-pair discipline (`mika-platform/docs/solutions/best-practices/canonical-propagated-prose-pair-discipline-2026-04-29.md`) requires the mika-platform copy to be updated in lockstep.

**F1 correction:** mika-platform#65 is CLOSED (confirmed via `gh_read issue_view`). The original plan deferred the canonical copy fix to mika-platform#65, but that deferral is stale — the issue is closed and cannot carry new work. The mika#897 issue body is unambiguous: "Affected files (BOTH need fixing)."

**Decision (revised):** This PR fixes BOTH copies in lockstep:
1. `mika/.claude/commands/mika-groom-milestone.md` — the propagated copy (Step 1 above)
2. `mika-platform/.claude/commands/mika-groom-milestone.md` — the canonical copy

The canonical copy at `mika-platform/.claude/commands/mika-groom-milestone.md` receives the identical step 3c text change. Since `mika-platform` is a workspace directory (not a sub-repo with its own PR flow), the canonical copy is updated via a direct commit on `mika-platform` main after the mika-side PR lands — per the canonical-propagated-prose-pair discipline's "canonical leads, propagated follows" update order. The mika-side PR description must include a reminder to apply the canonical-side update.

**Implementation:** Apply the same three-condition gate text (from Step 1's "New text" block) to `mika-platform/.claude/commands/mika-groom-milestone.md` at the corresponding step 3c location. The text is identical — no adaptation needed.

## Verification

1. Read the updated step 3c and confirm the three-condition gate is clear and unambiguous
2. Confirm the ITERATE-aborted scenario is explicitly called out as "do NOT short-circuit"
3. Confirm no other references to step 3c elsewhere in the file need updating (check Phase 4's references to per-sub-issue dispositions)
4. Confirm the mika-platform canonical copy receives the identical step 3c change (F1)
5. Confirm `GROOMED` appears in step 3b's disposition list and step 10's aggregate logic (F3 — already true, no change needed)

## Risk Assessment

- **Low risk** — prose-only change to a slash command spec file, no Rust code
- **No behavioral regression** — the fix makes the short-circuit stricter (fires less often), never more permissive
- **Idempotent re-groom** — sub-issues that re-enter the per-ticket flow will either complete GROOMED (normal path) or ESCALATE (if the plan is fundamentally wrong), both correct outcomes

## Revision history

- rev 2 (2026-05-26): addressed F1 by replacing the stale deferral to closed mika-platform#65 with a lockstep two-copy fix (canonical + propagated); addressed F2 by citing `_write_canonical_callout` implementation (dispatch-lib.sh lines 812–913) confirming exact marker string `second-pass (GROOMED)`, write-only-on-GROOMED-verdict timing, and sole-writer status; addressed F3 by citing step 3b (line 68) and step 10 (line 78) which already accept `GROOMED` as a valid per-sub-issue disposition; addressed F4 by documenting that step 3a's re-entry via mika-groom-ticket.md handles first-pass vs. second-pass routing internally (no step 3c specification needed).
