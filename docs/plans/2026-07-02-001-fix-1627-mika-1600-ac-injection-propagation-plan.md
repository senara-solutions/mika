# Plan — fix(commands): propagate mika#1600 U1 AC-injection to autonomous-loop groom commands (mika#1627)

- **Ticket:** `mika issue#1627`
- **Type:** fix (Tier 1 — loop-breaker)
- **Date:** 2026-07-02
- **Target repo:** `senara-solutions/mika-platform` (see § Repo routing — this is **not** a mika-repo change)

---

## Repo routing (READ FIRST — cross-repo, likely mis-routed)

This ticket's code lives in **`mika-platform/.claude/commands/`**, not in the `mika`
repo. The autonomous-loop groom command files (`mika-groom-plan-only.md`,
`mika-groom-milestone.md`, `mika-groom-ticket.md`) are **tracked only in the meta-repo**.
In the `mika` sub-repo they exist merely as seeded, `info/exclude`-shielded copies
(worktree slash-command seeding, mika#1415) — editing them here would touch an untracked
file that the next dispatch overwrites, so the fix **cannot** be implemented from a `mika`
worktree.

Evidence:
- `git ls-files .claude/commands/` in the mika repo tracks only `mika.md`,
  `mika-doc-audit.md`, `mika-issue.md`, `mika-issues.md` — **no** groom commands.
- The prior groom of this ticket (2026-06-30, plan `mika-platform/docs/plans/2026-06-30-008-…`,
  branch on `senara-solutions/mika-platform` @ `d6e429e`) correctly routed it to the
  meta-repo.
- This re-dispatch derived a `mika` worktree from the ticket ref `mika#1627` (issue on the
  `mika` repo), which is where the mis-route enters.

**Consequence for the pipeline:** a dev-pilot implementation in this `mika` worktree will
have nothing valid to change. The command-file edits must be applied on a
**`mika-platform`** branch. The architect / operator should re-route implementation to the
meta-repo (or apply the two small edits directly via the cross-repo primary+direct
strategy). This plan documents the fix so that re-routing is a one-step operator action.

---

## Current state (2026-07-02 — the ticket description is partially stale)

The ticket was filed 2026-06-29. Since then, part of the fix landed. Verified today:

| Command file (in `mika-platform/.claude/commands/`) | AC-injection step present? |
|---|---|
| `mika-groom-plan-only.md` | ✅ **YES** — step 5b, added 2026-06-29, cites mika#1600/#1627 |
| `mika-groom-ticket.md` | ❌ **NO** — 0 matches for "Acceptance criteria" as an injection step (the ticket's 2026-06-29 "HAS" claim is now stale) |
| `mika-groom-milestone.md` | ❌ **NO** — and its per-sub-issue flow (step 3a) delegates plan drafting to `mika-groom-ticket.md`'s phases, which also lack it |

So the residual gap is **two files**, not the "both MISSING" pair the ticket named:

1. `mika-groom-milestone.md` — the named-in-scope file, still missing the step.
2. `mika-groom-ticket.md` — **newly in-scope**: milestone delegates its per-sub-issue plan
   drafting here, so an AC-injection added only to `mika-groom-milestone.md` would not
   actually reach the sub-issue plans (they are drafted by the delegated `mika-groom-ticket`
   phases). Fixing milestone-produced plans requires the step to exist where the plan is
   drafted — i.e. in `mika-groom-ticket.md`. This file is also the operator-direct groom
   entry point, so the fix restores consistency across all three loop-groom commands.

`mika-groom-plan-only.md` requires **no change** (idempotent — already correct).

---

## Requirements

R1. Every autonomous-loop-groomed plan (per-issue, milestone-child, and milestone
sequencing per-sub-issue) must gain a `## Acceptance criteria` section, matching the U1
prose already present in `mika/.claude/commands/mika.md` (mika#1600) and
`mika-platform/.claude/commands/mika-groom-plan-only.md` (step 5b).

R2. The injected prose must preserve the three U1 rules verbatim in intent:
- Transcribe the issue's `## Acceptance criteria` verbatim when present.
- Otherwise derive concrete, testable criteria from Requirements + Verification Contract.
- Never rename `## Definition of Done`; both sections coexist.

R3. The fix must not regress `mika-groom-plan-only.md` (already correct) and must not touch
U2 (`mika/scripts/verify-pipeline.sh`) — U2 is the correct backstop and stays.

R4. Defense-in-depth (was "optional" in the ticket; **re-scoped**): a static check that all
autonomous-loop groom command files contain the AC-injection prose. **This check must live
in `mika-platform`, not `mika/tests/`** — the command files are not present in the mika
repo's checkout, so a mika Rust/CI test cannot read them. Implement as a shell assertion
(e.g. `mika-platform/scripts/` grep-gate) wired where meta-repo command hygiene is checked.
The ticket's original "static test in `mika/tests/`" suggestion is superseded by this
correction.

## Approach

Step 1 — `mika-groom-ticket.md` (meta-repo): insert a U1-style "Ensure `## Acceptance
criteria` section exists" step immediately after the `/ce:plan` draft step and before the
stage/commit step, carrying the three R2 rules. This is the load-bearing edit — it fixes
both operator-direct grooms and milestone-delegated per-sub-issue plans.

Step 2 — `mika-groom-milestone.md` (meta-repo): add an explicit reminder in the per-sub-issue
delegation (step 3a) that the delegated groom must produce the AC section, and add the same
U1-style step to the milestone **sequencing-record** draft path (Phase 5 / step 18) if the
sequencing record is itself subject to U2. (Verify whether the sequencing record is a "plan"
under U2's glob before adding — if U2 only globs `docs/plans/*-plan.md` and the sequencing
file is `*-sequencing.md`, the sequencing record is out of U2 scope and only the delegation
reminder is needed.)

Step 3 — `mika-platform/scripts/` grep-gate (R4): assert each of
`mika-groom-plan-only.md`, `mika-groom-ticket.md`, `mika-groom-milestone.md` contains the
canonical AC-injection marker string. Fail non-zero if any is missing. Wire into the
meta-repo's existing check surface (deploy preflight or a CI hook).

## Verification Contract

- **VC1:** `grep -c "Acceptance criteria" mika-platform/.claude/commands/mika-groom-ticket.md`
  ≥ 1 as an injection step (not merely a mention), and the same for
  `mika-groom-milestone.md` (as a delegation reminder or its own step).
- **VC2:** `mika-groom-plan-only.md` is byte-unchanged (no regression).
- **VC3:** The new grep-gate script exits 0 when all three files carry the marker and
  exits non-zero when any marker is removed (test by temporary deletion).
- **VC4:** A dry-run autonomous-loop groom of any ticket via `/mika-groom-plan-only` and a
  milestone via `/mika-groom-milestone` produces a plan containing `## Acceptance criteria`
  — confirming U2 (`verify-pipeline.sh`) no longer fires at PR time on loop-authored plans.

## Definition of Done

- `mika-groom-ticket.md` and `mika-groom-milestone.md` carry the U1-style AC-injection /
  delegation prose (R1, R2).
- `mika-groom-plan-only.md` untouched (R3).
- U2 backstop untouched (R3).
- A meta-repo grep-gate asserts AC-injection presence across all three loop-groom commands
  (R4).
- Changes applied on a **`mika-platform`** branch (§ Repo routing).

## Acceptance criteria

The ticket body (mika#1627) has no `## Acceptance criteria` section; the following are
derived from the Requirements and Verification Contract above (per step 5b, second rule):

- **AC1:** `mika-platform/.claude/commands/mika-groom-ticket.md` contains a post-`/ce:plan`
  step that instructs adding a `## Acceptance criteria` section to the drafted plan, with
  the three U1 rules (verbatim-transcribe-when-present / derive-otherwise /
  never-rename-Definition-of-Done).
- **AC2:** `mika-platform/.claude/commands/mika-groom-milestone.md` guarantees its
  per-sub-issue and (if in U2 scope) sequencing plans include the `## Acceptance criteria`
  section — either via its own step or an explicit delegation reminder that the delegated
  `mika-groom-ticket` flow injects it.
- **AC3:** `mika-groom-plan-only.md` is unchanged and `mika/scripts/verify-pipeline.sh` (U2)
  is unchanged.
- **AC4:** A meta-repo defense-in-depth grep-gate (in `mika-platform/scripts/`, **not**
  `mika/tests/`) exits non-zero if any of the three loop-groom command files loses its
  AC-injection marker, and is wired into a check surface (deploy preflight or CI).
- **AC5:** After the fix, an autonomous-loop-authored plan (per-issue and milestone) passes
  `verify-pipeline.sh`'s U2 `## Acceptance criteria` check without operator hand-editing.

## Out of scope

- Changing U2 (`mika/scripts/verify-pipeline.sh`) — it is the correct gate.
- The interactive `/mika` command (`mika/.claude/commands/mika.md`) — already fixed by
  mika#1600 U1.
- Re-fixing `mika-groom-plan-only.md` — already correct.

## Risks / notes

- **Primary risk:** implementing in this `mika` worktree is a no-op — the target files are
  not tracked here (§ Repo routing). The architect should ESCALATE on repo-mismatch or the
  operator should re-route to a `mika-platform` branch before a dev-pilot dispatch.
- **Scope creep guard:** `mika-groom-ticket.md` is newly pulled into scope (the ticket named
  only plan-only + milestone). This is justified: milestone delegates plan drafting to
  ticket, so ticket is the actual injection site for milestone-produced plans. Without it,
  fixing milestone alone leaves the delegated sub-issue plans still AC-less.
