---
title: "fix: qa-review rescue-class verdict inconsistency"
date: 2026-06-29
sequence: 1618
type: fix
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
issue: mika#1618
---

# fix: qa-review rescue-class verdict inconsistency — same procedural shape, opposite verdicts

## Goal Capsule

Standardize dispatch-lib's auto-rescue PR body across both rescue classes (dirty-worktree and commit-pushed-no-pr) so qa-review produces consistent verdicts. Replace free-text boilerplate interpretation with a machine-readable HTML comment marker that tracks pipeline verification state.

---

## Summary

qa-review gives inconsistent verdicts on dispatch-lib auto-rescued PRs: dirty-worktree class PRs get APPROVED after operator un-drafts, while commit-pushed-no-pr class PRs get stuck in repeated `hold[review]` cycles because qa over-interprets the more procedural boilerplate language as an active gate. The fix standardizes the rescue boilerplate across both classes and introduces a machine-readable `<!-- rescue-pipeline-verified: yes/no -->` HTML comment marker that qa-review reads instead of interpreting free-text.

---

## Problem Frame

Two rescue classes in dispatch-lib (`dirty-worktree` at line ~2451 and `commit-pushed-no-pr` at line ~2457) produce different PR body text:

- **dirty-worktree**: "This is a draft PR requiring human review. The content has NOT passed `/ce:review`..."
- **commit-pushed-no-pr**: "This is a draft PR — operator should verify pilot's pipeline (/ce:work, /ce:review, /ce:compound) completed before marking ready."

qa-review has no rescue-class-specific logic — it reads the PR body as free text. The `commit-pushed-no-pr` class's procedural language ("operator should verify... before marking ready") reads as a still-active gate, causing qa to hold even after the operator un-drafts and posts `/ce:review`. The `dirty-worktree` class's more generic framing doesn't trigger the same interpretation.

The root cause is qa keying off prose semantics rather than a structured signal for pipeline verification state.

---

## Requirements

- R1. Both rescue classes produce identical boilerplate structure in the PR body, differing only in the recovery-class metadata and a factual description of what happened.
- R2. A machine-readable HTML comment marker (`<!-- rescue-pipeline-verified: no -->`) is embedded in rescue PR bodies at creation time.
- R3. qa-review reads the marker to determine pipeline verification state rather than interpreting free-text boilerplate.
- R4. When the marker says `no` and the PR is still a draft, qa emits `hold[review]` with a clear reason.
- R5. When the marker says `yes` OR the PR is no longer a draft (operator un-drafted it), qa proceeds to normal substantive review — the rescue boilerplate is not an impediment.
- R6. Operator can update the marker to `yes` by editing the PR body (replacing `no` with `yes` in the HTML comment). This is the explicit opt-in signal that pipeline verification is complete.
- R7. Backward compatibility: PRs created before this change (no marker present) are treated as pipeline-verified (qa proceeds normally).

---

## Key Technical Decisions

**KTD1. HTML comment marker vs. label-based signal.**
HTML comment in the PR body is chosen over a GitHub label because: (a) it co-locates with the rescue boilerplate, (b) it's visible in the raw body for debugging, (c) it doesn't pollute the label namespace, (d) the operator's existing workflow of editing the PR body to remove rescue boilerplate naturally extends to flipping the marker. A label would require a separate `gh label add` step and qa would need label-reading logic.

**KTD2. Un-draft as implicit verification signal.**
When the operator marks the PR as "Ready for review" (un-drafts it), that is a stronger signal than any body marker — the operator has explicitly decided the PR is ready. qa-review should treat `isDraft=false` on a rescue PR as equivalent to `rescue-pipeline-verified: yes`, regardless of the marker value. This matches the observed working behavior for dirty-worktree class PRs and codifies it as the rule for all rescue classes.

**KTD3. Standardized boilerplate with class-specific factual note.**
Both classes share identical structure: a standard rescue header, the machine-readable marker, and a recovery metadata block. The only difference is a single factual sentence describing what happened (dirty files auto-committed vs. PR creation failed). This eliminates the semantic divergence that caused the inconsistency.

---

## Scope Boundaries

### In Scope
- dispatch-lib rescue PR body templates (both classes)
- qa-review system prompt: add rescue-class-aware pre-check before Step 2

### Deferred to Follow-Up Work
- Automatic marker flip when operator posts `/ce:review` comment (would require a webhook handler; the manual body-edit path is sufficient for now)
- Eliminating auto-rescue (out of scope per ticket; it's load-bearing — mika#1282)

---

## Implementation Units

### U1. Standardize dispatch-lib rescue PR body templates

**Goal:** Replace the two divergent rescue PR body templates with a single unified template that uses a machine-readable marker.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify lines ~2448-2484)

**Approach:**
Replace the if/else block that produces `_rescue_body_note` with a single template shared by both classes. The template includes:
1. A standardized header: `## Auto-rescued PR (dispatch-lib recovery, class: ${RECOVERY_CLASS})`
2. The machine-readable marker: `<!-- rescue-pipeline-verified: no -->`
3. A class-specific one-sentence factual description (what happened, not what the operator should do)
4. A uniform instruction: "**Auto-rescued PR.** Operator: verify pipeline completion, then either un-draft this PR or set the marker above to `yes`."
5. Recovery metadata block (unchanged)

The class-specific factual descriptions:
- `dirty-worktree`: "The pilot session wrote file changes but never committed. dispatch-lib auto-committed with `wip()` prefix."
- `commit-pushed-no-pr`: "The pilot session committed and pushed but `gh pr create` failed. dispatch-lib opened this PR from the existing branch."

Both end with the same instruction rather than divergent procedural language.

**Patterns to follow:** The existing rescue template structure at lines ~2463-2484. The `RECOVERY_CLASS` variable and metadata block are preserved as-is.

**Test scenarios:**
- dirty-worktree rescue produces body containing `<!-- rescue-pipeline-verified: no -->` and the standardized instruction text
- commit-pushed-no-pr rescue produces body containing `<!-- rescue-pipeline-verified: no -->` and the standardized instruction text
- Both classes produce identical structure except for the one-sentence factual description and `Recovery class:` metadata value
- The `Closes #${ISSUE_NUM}` line is preserved

**Verification:** After the change, both `_rescue_body_note` paths produce bodies that match the same structural template. `grep -c 'rescue-pipeline-verified'` on the PR body returns exactly 1.

---

### U2. Add rescue-class pre-check to qa-review system prompt

**Goal:** Add a rescue-class detection and marker-reading step to qa-review that runs before Step 2 (pipeline compliance checks), so qa-review handles rescue PRs consistently regardless of boilerplate wording.

**Requirements:** R3, R4, R5, R6, R7

**Dependencies:** U1

**Files:**
- `skills/bundled/qa-review/system_prompt.md` (modify — add new step between Step 1 and Step 2)

**Approach:**
Insert a new **Step 1.5 — Rescue-class PR detection** between the existing Step 1 (Extract and confirm PR context) and Step 2 (Pipeline compliance checks). This step:

1. **Detect rescue PR.** Check the PR body (already fetched in Step 1 via `qa_pr_view`) for the presence of `## Auto-rescued PR (dispatch-lib recovery, class:`. If not found, skip this step — proceed to Step 2 normally.

2. **Read the marker.** Search the PR body for `<!-- rescue-pipeline-verified: yes -->` or `<!-- rescue-pipeline-verified: no -->`.

3. **Evaluate verification state.** The PR is considered pipeline-verified if ANY of these conditions hold:
   - The marker reads `yes`
   - The PR `isDraft` field is `false` (operator un-drafted it)
   - No marker is found at all (backward compatibility — R7)

4. **Route based on verification state:**
   - **Verified:** Note "Rescue PR (class: `<class>`), pipeline verified — proceeding to standard review." Continue to Step 2 normally. The rescue boilerplate text is not treated as a review gate.
   - **Not verified** (marker is `no` AND PR is still draft): Emit `hold[review]` with reason: "Auto-rescued PR (class: `<class>`) is still in draft with pipeline-verification marker set to `no`. Operator must verify pipeline completion and either mark the PR as Ready for Review or edit the body to set `<!-- rescue-pipeline-verified: yes -->`." End the review.

This step is deliberately simple — it reads structured data (HTML comment + isDraft boolean) rather than interpreting free-text semantics, which is the root cause of the inconsistency.

**Patterns to follow:** The existing Step 2 bypass patterns (pipeline-exempt label, Pipeline-Exempt trailer, tactical-surface auto-detect) which all follow the same detect → evaluate → route structure.

**Test scenarios:**
- Rescue PR with marker `no` and isDraft=true → `hold[review]`
- Rescue PR with marker `no` and isDraft=false → proceeds to Step 2 (un-draft is implicit verification)
- Rescue PR with marker `yes` and isDraft=true → proceeds to Step 2
- Rescue PR with marker `yes` and isDraft=false → proceeds to Step 2
- Rescue PR with no marker at all (pre-change PR) → proceeds to Step 2 (backward compat)
- Non-rescue PR (no `## Auto-rescued PR` header) → skips Step 1.5 entirely, proceeds to Step 2
- Both rescue classes (dirty-worktree and commit-pushed-no-pr) → same verdict behavior when marker and isDraft state are identical

**Verification:** Given the same marker state and isDraft value, both rescue classes produce identical qa-review behavior. The qa-review system prompt no longer references or interprets the descriptive text in rescue PR bodies.

---

## Verification Contract

1. **Marker presence:** Both rescue classes embed `<!-- rescue-pipeline-verified: no -->` in auto-rescued PR bodies.
2. **Verdict consistency:** Given identical marker and isDraft state, qa-review produces the same verdict regardless of rescue class.
3. **Backward compatibility:** Pre-change rescue PRs (no marker) are treated as verified and proceed to normal review.
4. **Un-draft signal:** Setting a rescue PR to "Ready for Review" (isDraft=false) is sufficient for qa-review to proceed, regardless of marker value.

---

## Definition of Done

- [ ] dispatch-lib produces standardized rescue PR body with `<!-- rescue-pipeline-verified: no -->` marker for both rescue classes
- [ ] qa-review system prompt includes Step 1.5 rescue-class detection that reads the marker
- [ ] No test regressions (`cargo test`)
- [ ] Existing dispatch-lib tests pass (if applicable: `test-dispatch-lib.sh`)

---

## Acceptance criteria

- [ ] AC1: Both rescue classes (dirty-worktree and commit-pushed-no-pr) produce PR bodies containing the HTML comment `<!-- rescue-pipeline-verified: no -->`.
- [ ] AC2: Both rescue classes produce PR bodies with identical structure (same headers, same instruction text), differing only in the one-sentence factual description and recovery-class metadata value.
- [ ] AC3: qa-review detects rescue PRs by checking for the `## Auto-rescued PR` header in the PR body.
- [ ] AC4: qa-review reads the `<!-- rescue-pipeline-verified: yes/no -->` marker to determine pipeline verification state.
- [ ] AC5: When the marker is `no` and the PR is still a draft, qa-review emits `hold[review]`.
- [ ] AC6: When the PR is no longer a draft (isDraft=false), qa-review proceeds to substantive review regardless of marker value.
- [ ] AC7: When no marker is found (pre-change PR), qa-review proceeds to substantive review (backward compatibility).
- [ ] AC8: Given identical marker and isDraft state, both rescue classes produce the same qa-review verdict.

---

## Open Questions

None — the fix is well-bounded. The marker-flip automation (webhook on `/ce:review` comment) is explicitly deferred.

---

## Sources & Research

- mika#1618 — founding ticket with hard evidence (PR #1610 vs PR #135)
- mika#1282 — dirty-worktree auto-rescue origin
- mika#1396 — commit-pushed-no-pr rescue origin
- `skills/bundled/_shared/dispatch-lib.sh` lines ~2428-2498 — current rescue PR body templates
- `skills/bundled/qa-review/system_prompt.md` — current qa-review logic (no rescue-class handling)
- `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md` — content/workflow split contract
