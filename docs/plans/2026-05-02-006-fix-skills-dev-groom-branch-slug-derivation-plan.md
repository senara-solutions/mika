---
title: "fix(skills/dev-groom): replace in-prompt branch slug derivation with invocation of canonical scripts/derive-branch-name"
type: fix
status: active
date: 2026-05-02
---

# fix(skills/dev-groom): replace in-prompt branch slug derivation with invocation of canonical scripts/derive-branch-name

## Overview

`mika/skills/bundled/dev-groom/system_prompt.md:14` instructs the model to derive branch slugs in-prompt using a deterministic recipe (`<type>/<n>/<sanitized-title>`) that diverges from the canonical script `scripts/derive-branch-name` invoked by every other dispatch path (operator-side `/mika`, `/mika-groom-ticket`, dev-pilot dispatcher). The two recipes produce different slugs for the same ticket — concrete N=1 example in ticket body for mika#927.

Fix: replace the in-prompt derivation block with explicit invocation of the canonical script. The skill's `<n>`-then-`<sanitized-title>` template AND the `<type>` label-mapping AND the conventional-commit-prefix extraction AND the truncation behavior ALL live in the script; the prompt should NOT re-implement them.

## Problem Frame

Per Vincent's `feedback_orthogonality_flag_semantics.md` and the meta-repo CLAUDE.md mandate (*"Every dispatch path that needs to derive a branch name MUST invoke the canonical script"*), all dispatch paths must converge on `scripts/derive-branch-name`. dev-groom is the lone violator — its system_prompt.md re-implements the recipe inline, producing slugs that diverge from the canonical path's output (different truncation, different sanitization edge cases, different priority ordering).

Surfacing context: ticket body's pre-flight check before first autonomous-groom dispatch on mika#927 found the divergence. Vincent's stated gate condition: *"If it re-derives, that's a bug — file it and fall back to operator-side for #927/#928 until fixed."*

Audit confirmed during plan-time research: only `dev-groom` has the pattern; `dev-pilot/system_prompt.md` does not re-derive (it relies on the dispatcher to provide the slug). So the fix is single-skill; no sibling-skill cleanup needed.

## Requirements Trace

- **R1.** `mika/skills/bundled/dev-groom/system_prompt.md` instructs the model to invoke `scripts/derive-branch-name` (with the canonical Bash invocation pattern) instead of in-prompt derivation.
- **R2.** The body-callout-takes-priority semantics from current line 13 are preserved.
- **R3.** The slug-immutability-after-worktree-creation semantics from current line 15 are preserved.
- **R4.** No other dev-* skill is modified (audit confirmed only dev-groom has the pattern).
- **R5.** After deploy: dev-groom-grooned ticket and operator-side `/mika-groom-ticket` produce IDENTICAL slug for the same ticket inputs.

## Scope Boundaries

- Only `mika/skills/bundled/dev-groom/system_prompt.md` is modified — the source-of-truth for the bundled skill.
- The deployed copy at `~/.mika/agents/mika-dev/skills/dev-groom/system_prompt.md` is updated via `make deploy`'s bundled-skill resync (per existing mika#923 mechanism — this fix doesn't change deploy plumbing).
- `scripts/derive-branch-name` itself is NOT modified — it's the canonical source.

### Deferred to Separate Tasks

- `mika#923` — `mika skills update` doesn't propagate `_shared/` files. Tangentially related (skill-install defect surfaces in same area) but distinct fix surface. Not bundled.
- Audit of operator-side slash commands for similar in-prompt derivation patterns (`/mika.md`, `/mika-groom-ticket.md`) — those already invoke the canonical script per their current state (verified during plan-time research). No follow-up needed.

## Context & Research

### Relevant Code and Patterns

- `mika/skills/bundled/dev-groom/system_prompt.md:13-15` — the file under repair (slug derivation block).
- `scripts/derive-branch-name` (in `mika-platform/scripts/`) — the canonical script. Inputs: `--title`, `--issue`, `--labels`, `--body-callout`, optional `--explicit`. Enforces priority order (body callout → conv-commit prefix → label override → default `feat`) and 40-char truncation.
- `mika-platform/.claude/commands/mika-groom-ticket.md` Phase 1 step 3 — the canonical Bash invocation pattern for the script (operator-side reference). Mirrors what dev-groom should do.
- `mika-platform/.claude/commands/mika.md` § Branch-name derivation — the canonical priority order documentation.

### Institutional Learnings

- `feedback_orthogonality_flag_semantics.md` — "A flag's semantics must not depend on another flag's value." The slug-derivation flag (label-vs-conv-commit-vs-callout) semantics are owned by ONE source (the script); duplicating them in a prompt creates orthogonality violations.
- mika#927/#928 fall-back-to-operator decision (Vincent's gate condition): cited in ticket body as the trigger for filing this ticket.
- `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — slug-as-callout discipline; this fix preserves it (R2).

### External References

None — repo-internal change.

## Key Technical Decisions

### Decision 1: Replace the derivation block with explicit script invocation

**Decision:** Replace `system_prompt.md` lines 13-14 (the body-callout-or-derive block) with prose that:
1. Preserves the body-callout-takes-priority instruction (R2).
2. Replaces "Otherwise, derive deterministically" with "Otherwise, invoke the canonical script" + a Bash invocation showing the exact pattern.

**Rationale:**
- Single source of truth for slug derivation (`scripts/derive-branch-name`).
- Eliminates prompt-vs-script divergence entirely.
- Future changes to the slug recipe (e.g., truncation tuning, new label mappings) only require editing the script; the prompt stays stable.

**Rejected alternatives:**
- **Mirror the script's logic in the prompt with a "must match script" annotation.** Doubles the maintenance surface; doesn't actually prevent drift.
- **Move the derivation to a Bash helper that the model invokes.** Adds an extra abstraction layer for no gain — the script IS that helper.

## Open Questions

### Resolved During Planning

- **Are sibling dev-* skills affected?** → No. Plan-time audit (`grep -lE "derive deterministically|<type>/<n>|conventional prefix" mika/skills/bundled/dev-*/system_prompt.md`) returned only dev-groom. Confirmed in Scope Boundaries.
- **Replacement text wording?** → Locked at SHAPE level (preserve callout-priority + invoke script with canonical Bash pattern). Exact wording deferred to /ce:work but constrained by R2/R3 invariants.

### Deferred to Implementation

- **Exact prose of the replacement block** — derived during /ce:work using `mika-platform/.claude/commands/mika-groom-ticket.md` Phase 1 step 3 as the pattern reference. Constraint: must include the canonical Bash invocation `BRANCH=$("$SCRIPTS_DIR/derive-branch-name" --title "$ISSUE_TITLE" --issue "$ISSUE_NUMBER" --labels "$LABELS" --body-callout "$ISSUE_BODY")`.

## Implementation Units

- [ ] **Unit 1: Replace dev-groom's branch-slug derivation block with canonical script invocation**

**Goal:** Edit `mika/skills/bundled/dev-groom/system_prompt.md` lines 13-14 (the body-callout-or-derive block) to invoke `scripts/derive-branch-name` instead of in-prompt derivation.

**Requirements:** R1, R2, R3, R5.

**Dependencies:** None.

**Files:**
- Modify: `mika/skills/bundled/dev-groom/system_prompt.md`

**Approach:**

1. Read current `system_prompt.md` lines 13-15.
2. Replace lines 13-14 with:
   - Line 13 (preserved): callout-takes-priority instruction (verbatim from current).
   - Line 14 (replaced): canonical script invocation prose. Mirror the pattern from `mika-platform/.claude/commands/mika-groom-ticket.md` Phase 1 step 3 — same `SCRIPTS_DIR=...` / `BRANCH=$(...)` invocation. Add a one-line note: "*Do NOT re-derive the slug in prompt logic — slug recipe is owned by the script and must match the meta-repo dispatcher and dev-pilot dispatcher.*"
3. Line 15 (preserved): immutability-after-worktree-creation instruction (verbatim from current).
4. Verify the deployed copy at `~/.mika/agents/mika-dev/skills/dev-groom/system_prompt.md` will pick up the change on next `make deploy` — no special propagation needed (bundled-skill resync mechanism handles it).

**Patterns to follow:**

- `mika-platform/.claude/commands/mika-groom-ticket.md` Phase 1 step 3 — the canonical invocation pattern.
- `mika/skills/bundled/dev-pilot/system_prompt.md` — sibling skill that does NOT re-derive; relies on dispatcher input. Reference pattern.

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | Dispatch dev-groom on a ticket with label `bug` and conventional-commit-prefixed title (e.g., `fix(server): ...`). Expected: dev-groom's claude-pilot invokes `scripts/derive-branch-name` and produces a slug matching what `/mika-groom-ticket` operator-side would produce for the same ticket. |
| Happy path | Dispatch dev-groom on a ticket with body callout `> - **Branch:** \`<slug>\``. Expected: callout slug used verbatim (R2 preserved). |
| Edge case | Dispatch dev-groom on a ticket with NO label, NO conv-commit prefix, NO callout. Expected: script defaults to `feat` per priority rule 4 (per `/mika.md` § Branch-name derivation). |
| Edge case | Dispatch dev-groom on a ticket with title >40 chars. Expected: script truncates to 40 chars (per the script's enforced policy). The OLD in-prompt derivation might have produced a >40 char slug — this fixture validates the divergence is closed. |
| Integration | Re-dispatch the same ticket via dev-groom AND `/mika-groom-ticket` operator-side. Expected: identical slug from both paths. Validates R5 end-to-end. |

**Verification:**

- `git diff skills/bundled/dev-groom/system_prompt.md` shows changes ONLY at lines 13-14 region.
- The replaced text contains the literal `derive-branch-name` script reference.
- Line 13 (callout-takes-priority) and Line 15 (immutability) are byte-identical pre/post (R2, R3 preserved).
- Post-deploy: a dev-groom dispatch on a test ticket produces a slug matching what `/mika-groom-ticket` would produce for the same ticket inputs.

## System-Wide Impact

- **Interaction graph:** `dev-groom` skill is loaded by mika-dev's claude-pilot when grooming via the autonomous loop. Edit propagates via bundled-skill resync on every agent session start.
- **Error propagation:** None affected. The model invokes a Bash script (already a tool available); script failures surface as Bash errors with the same handling as today.
- **State lifecycle risks:** None. The change is prompt-only.
- **API surface parity:** This change brings dev-groom into parity with operator-side `/mika-groom-ticket` and dev-pilot dispatcher for slug derivation. After this fix, all four dispatch paths converge on `scripts/derive-branch-name`.
- **Unchanged invariants:**
  - `scripts/derive-branch-name` itself unchanged.
  - `dev-pilot/system_prompt.md` unchanged.
  - `/mika.md`, `/mika-groom-ticket.md` operator-side commands unchanged.
  - dev-groom's other phases (Phase 2 worktree creation, Phase 3 architect review, etc.) unchanged.
  - Body-callout-priority semantics (R2).
  - Slug-immutability semantics (R3).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Replacement Bash invocation has a subtle quoting bug that breaks dev-groom dispatches. | /ce:work uses the proven invocation from `/mika-groom-ticket.md` Phase 1 step 3 (operator-side, verified working today). Live canary post-deploy confirms behavior. |
| Sibling dev-* skill has a similar pattern that the audit missed. | Plan-time audit was exhaustive (`grep -lE` across `dev-*/system_prompt.md`); only dev-groom matched. If a future dev-* skill is added with the same pattern, file separately. |
| The deployed skill at `~/.mika/agents/mika-dev/skills/dev-groom/system_prompt.md` is byte-identical to source per ticket — but bundled-skill resync mechanism (per mika#923) doesn't always sync `_shared/` files. This file is in `dev-groom/` directly, NOT `_shared/`, so the standard resync covers it. | Verified: `dev-groom/system_prompt.md` is in the standard resync path. |
| Plan-doc-check hook fails on PR open because the plan path isn't cited in the PR body or commit. | Manually cite the literal path `docs/plans/2026-05-02-006-fix-skills-dev-groom-branch-slug-derivation-plan.md` in the PR body or a commit body. |

## Documentation / Operational Notes

- **Rollout:** Standard skill change. PR merge → `make deploy` triggers bundled-skill resync → `~/.mika/agents/mika-dev/skills/dev-groom/system_prompt.md` updated on next mika-dev session start.
- **Verification timeline:** After merge + deploy: dispatch a test ticket via dev-groom, confirm slug matches operator-side derivation for the same inputs.
- **Pattern compounding (deferred):** This is the second instance of "in-prompt derivation diverges from canonical-script source-of-truth" (first instance: not yet — but worth flagging as N=1 forward-pointer). If a third instance surfaces, author a compound doc on the discipline (*"Skills that need to compute load-bearing values must invoke canonical scripts, not re-implement them in prompt logic."*).

## Sources & References

- **Ticket:** [mika#929](https://github.com/senara-solutions/mika/issues/929)
- **Source file:** `mika/skills/bundled/dev-groom/system_prompt.md:13-15`
- **Canonical script:** `mika-platform/scripts/derive-branch-name`
- **Pattern reference:** `mika-platform/.claude/commands/mika-groom-ticket.md` Phase 1 step 3 (operator-side invocation)
- **Sibling reference:** `mika/skills/bundled/dev-pilot/system_prompt.md` (does NOT re-derive)
- **Concrete drift example:** ticket body cites mika#927 with two divergent slugs.
- **Vincent's gate condition:** ticket body — *"If it re-derives, that's a bug — file it and fall back to operator-side."*
- **Related institutional knowledge:**
  - `feedback_orthogonality_flag_semantics.md` (orthogonality violations)
  - `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` (callout discipline)
  - mika-platform meta-repo CLAUDE.md § Branch-name derivation
