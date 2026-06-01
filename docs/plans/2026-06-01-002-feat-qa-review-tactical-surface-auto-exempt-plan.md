---
title: "feat: auto-detect tactical-surface PRs as Pipeline-Exempt by changed-path-set"
type: feat
status: completed
date: 2026-06-01
---

# Auto-Detect Tactical-Surface PRs as Pipeline-Exempt

## Summary

Add a third bypass path to the qa-review skill's pipeline-complete gate (Step 2) that auto-detects tactical PRs by changed-path-set. PRs whose changes are confined entirely to infrastructure/operational paths (`.github/workflows/`, `Dockerfile.*`, `skills/bundled/_shared/`, `os/`, `scripts/`) with no source code under `crates/` are auto-exempted from the plan-doc requirement — no label or trailer needed.

---

## Problem Frame

The pipeline-complete gate correctly requires a `docs/plans/*.md` file for feature PRs, with label and trailer escape hatches. But tactical hot-fix PRs — CI yaml fixes, Dockerfile patches, dispatch-lib fixes — legitimately don't have plans. These PRs get stuck on `VERDICT: block[pipeline]` until an operator manually applies the `pipeline-exempt` label. PR#1369 (a 4-file Dockerfile fix) sat blocked for 42 minutes despite green CI, waiting for an admin merge.

The gap is structural: a class of PRs exists that will never have plans, and the gate has no path-based auto-detection to recognize them.

---

## Requirements

- R1. When ALL changed files in a PR match the tactical-surface allowlist AND no files under `crates/` are changed, the pipeline-complete gate auto-exempts the PR without requiring a label or trailer.
- R2. The tactical-surface allowlist covers: `.github/workflows/`, `Dockerfile.*`, `skills/bundled/_shared/`, `os/`, `scripts/`. Files under `docs/` are neutral (neither tactical nor product — they don't disqualify a tactical PR).
- R3. The verdict body explains the auto-exemption decision, listing which path prefixes matched.
- R4. When a PR mixes tactical-surface files with product source code (files under `crates/`), the gate falls through to normal behavior (require plan or explicit label) and the verdict body cites which file forced the requirement.
- R5. The bypass precedence order is: label → trailer → tactical-surface auto-detect → checks 1-3.

---

## Key Technical Decisions

- **Prompt-only change, no Rust code:** The bypass is implemented entirely in `system_prompt.md` using the same `run_gh` tool calls as the existing label and trailer bypasses. No changes to `verdict.rs`, `verdict_handler.rs`, or `skill.toml` — the existing `VERDICT: pass` token already covers this path.

- **`crates/` as the product-source gate:** The negative check uses `crates/` presence as the sole signal for "product source code." This matches the repo structure where all Rust source lives under `crates/`. Files outside both the tactical allowlist and `crates/` (e.g., `Cargo.toml`, `Makefile`, `dashboard/`, `packages/ui/`) are treated as non-tactical but also non-blocking — if a PR touches only `Makefile` and `Dockerfile.agent`, it auto-exempts because there are no `crates/` files. This is intentionally permissive for the infrastructure surface.

- **`docs/` is neutral:** Documentation files alongside tactical changes don't disqualify auto-exemption (a Dockerfile fix that also updates `docs/solutions/` should still pass). This mirrors the label bypass's source-change check which already excludes `docs/` from the "has source changes" filter.

- **Two-command detection pattern:** One grep-inverse for non-tactical/non-docs files, one positive grep for `crates/`. Both must pass for auto-exemption: the first confirms all changes are in recognized surfaces, the second confirms no product code is touched. The two-command pattern avoids a single complex regex that would be hard to maintain in a prompt.

---

## Implementation Units

### U1. Add tactical-surface auto-detect bypass to system_prompt.md

**Goal:** Insert the third bypass path between the trailer bypass (line ~139) and check 1 (line ~141), following the established bypass pattern.

**Requirements:** R1, R2, R3, R4, R5

**Files:**
- `skills/bundled/qa-review/system_prompt.md` (modify)

**Approach:**

Insert a new section after line 139 ("If the `git log` grep returns no match...") and before line 141 ("1. **Plan doc exists**..."). The section follows the same structure as the label and trailer bypasses:

1. **Detection commands** — two `run_gh` calls:
   - First: check if any changed file falls outside the tactical allowlist AND outside `docs/`:
     ```
     run_gh("pr diff <PR_URL> --name-only | grep -vE '^(\\.github/workflows/|Dockerfile\\.|skills/bundled/_shared/|os/|scripts/)' | grep -vE '^docs/' | head -1")
     ```
   - Second (only if first is empty): check for product source code:
     ```
     run_gh("pr diff <PR_URL> --name-only | grep -E '^crates/' | head -1")
     ```

2. **Decision logic:**
   - If the first result is empty (all files are tactical or docs) AND the second result is empty (no `crates/` files): auto-exempt. Skip checks 1-3 and Step 2.5. Note the matched path prefixes in the bypass message.
   - If the first result is non-empty: not a pure tactical PR — fall through to checks 1-3 normally.
   - If the first result is empty but the second is non-empty (unlikely given the allowlist, but defensive): note "tactical-surface paths detected but PR also contains source code under `crates/` — requiring plan." Fall through to checks 1-3 and cite the first `crates/` file.

3. **Bypass note format:** "Tactical-surface auto-exempt: all changes confined to [`.github/workflows/`, `Dockerfile.*`, ...] with no source under `crates/` — skipping pipeline checks and plan-AC verification."

**Patterns to follow:** The label bypass block (lines 98-108) and trailer bypass block (lines 110-139) are the direct templates. Mirror their structure: detection command → conditional logic → bypass note → skip/continue instruction.

**Test scenarios:**
- Dockerfile-only PR with no plan and no label → auto-exempts with `pass` verdict and tactical-surface note in body
- PR touching only `.github/workflows/ci.yml` and `scripts/check-byte-slices.sh` → auto-exempts
- PR touching `skills/bundled/_shared/dispatch-lib.sh` and `docs/solutions/some-doc.md` → auto-exempts (docs are neutral)
- PR touching `Dockerfile.agent` AND `crates/mika-agent/src/main.rs` → falls through to normal checks, verdict cites the `crates/` file
- PR touching only `Cargo.toml` (no `crates/`, not in tactical allowlist) → auto-exempts (no `crates/` files, non-tactical-non-docs file is filtered by grep-inverse but `Cargo.toml` doesn't match the allowlist... wait — this would NOT auto-exempt because `Cargo.toml` would be output by the first grep. Correct behavior.)
- PR touching `Makefile` only → does NOT auto-exempt (Makefile is outside the tactical allowlist, first grep returns it)
- PR with `pipeline-exempt` label → label bypass fires first (R5 precedence), tactical-surface never checked

**Verification:** Deploy the updated skill (`make deploy`). The change takes effect on next mika-qa review invocation — no Rust rebuild needed since skills are prompt-only.

### U2. Add verdict output example for tactical-surface auto-exemption

**Goal:** Add an example verdict block near the existing bypass examples (lines ~509-570) showing the expected output shape when tactical-surface auto-exemption fires.

**Requirements:** R3

**Dependencies:** U1

**Files:**
- `skills/bundled/qa-review/system_prompt.md` (modify — same file, different section)

**Approach:**

Insert a new example block after the `Pipeline-Exempt: code-only` trailer example (line ~569) and before the `block[ac]` example (line ~572). The example shows:

```
When tactical-surface auto-exemption was applied (infrastructure-only PR with no plan):

VERDICT: pass
DEPTH: code-level
REASON: Tactical-surface PR; auto-exempt — all changes confined to infrastructure paths, no source under crates/.

DIFF ANALYSIS:
Files reviewed: 2
Key changes:
- Fixed Docker Build tag validation in Dockerfile.agent
- Updated CI workflow timeout in .github/workflows/ci.yml

PLAN-AC VERIFICATION: skipped (tactical-surface auto-exempt — changes confined to .github/workflows/, Dockerfile.*)

BUILD VERIFICATION: skipped (tactical-surface auto-exempt — no source changes)

VERDICT: pass
DEPTH: code-level
REASON: Tactical-surface PR; auto-exempt — all changes confined to infrastructure paths, no source under crates/.
```

**Patterns to follow:** The existing examples for label bypass (lines 509-528) and trailer bypass (lines 530-569) — mirror the structure with the verdict-first format, DIFF ANALYSIS, PLAN-AC VERIFICATION skip literal, and BUILD VERIFICATION skip.

**Test scenarios:**
- Test expectation: none — this unit adds an example block to the prompt, not behavioral logic

**Verification:** Read the updated prompt to confirm the example is consistent with the bypass note format from U1 and the pre-termination self-check invariant 3 (line 440).

### U3. Update pre-termination self-check invariant for tactical-surface bypass

**Goal:** Add the tactical-surface skip literal to the pre-termination self-check invariant 3 (line ~440) so the LLM validates its own output includes the correct skip format.

**Requirements:** R3

**Dependencies:** U1

**Files:**
- `skills/bundled/qa-review/system_prompt.md` (modify — same file, invariant 3 section)

**Approach:**

Extend invariant 3's list of valid `PLAN-AC VERIFICATION:` skip literals to include the tactical-surface variant:
```
PLAN-AC VERIFICATION: skipped (tactical-surface auto-exempt — changes confined to <matched prefixes>)
```

The `<matched prefixes>` is dynamic (lists only the prefixes actually matched in the PR), but the invariant can check for the `tactical-surface auto-exempt` substring.

**Patterns to follow:** Invariant 3 already lists the label and trailer skip literals — add the tactical-surface variant in the same format.

**Test scenarios:**
- Test expectation: none — this is a self-check instruction update, not behavioral logic

**Verification:** Read invariant 3 to confirm all four bypass paths (label, docs-only trailer, code-only trailer, tactical-surface) have corresponding skip literals.

---

## Scope Boundaries

### Deferred to Follow-Up Work

- Extending the tactical-surface allowlist to cover `dashboard/`, `packages/ui/`, or other non-Rust surfaces. The current allowlist targets infrastructure paths where plans are structurally unnecessary. Dashboard/UI changes may warrant plans even without `crates/` changes.
- Auto-merge UX for tactical PRs (orthogonal — auto-merge works once the gate passes).

---

## Acceptance Criteria

- [ ] mika-qa's pipeline-complete gate auto-detects tactical surfaces: changes confined to `.github/workflows/`, `Dockerfile.*`, `skills/bundled/_shared/`, `os/`, `scripts/` AND no source code under `crates/` → auto-treats as Pipeline-Exempt without requiring a label
- [ ] Verdict body explains the auto-exemption decision when applied (lists matched path prefixes)
- [ ] For ambiguous cases (mix of tactical + product changes), gate falls back to current behavior and verdict body cites which file forced the requirement
- [ ] Spawns/operators opening tactical PRs no longer need to remember to apply the label
- [ ] Regression test path: a Dockerfile-only PR + no plan + no label → mika-qa returns `pass[pipeline]` not `block[pipeline]`
