---
title: "fix: raise gateway PR-review truncation cap to 16k + reconcile docs/skills.md QA verdict contract"
type: fix
status: active
date: 2026-05-01
---

# fix: raise gateway PR-review truncation cap to 16k + reconcile docs/skills.md QA verdict contract

## Plan Contract

**Retroactive archaeological-record contract.** Implementation Units `[x]` complete because commits `f7691ccb` and `575044eb` (cherry-picked from alceops's PR #912 commits `aa39da84` and `b933bc2d`) already on branch. /ce:work dispatch is **SKIPPED** — implementation already exists and was approved by `mika-platform-qa` with `VERDICT: pass` on PR #912 (2026-04-30).

This plan's purpose:

1. Satisfy the plan-doc-check CI hook (mika-platform#64) for Knowledge Graph indexing
2. Author the compound-doc cross-references the community contributor couldn't produce without /mika tooling (Unit 3)
3. Provide a ratification surface for architect review

Squash merge applies `Co-authored-by: alceops <alce.ops@gmail.com>` trailer to preserve attribution.

This contract is distinct from the default forward-validation flow where /ce:work runs the implementation against current main. Choosing archaeological-record here is correct because alceops's code is already approved, cherry-picks preserve authorship, and the gap is genuinely meta-artifacts not implementation. Future community contributions following the same shape (no Claude Code locally, plan-doc-check hook blocks merge) should follow this archaeological-record pattern.

## Overview

Two coupled changes addressing the verdict-classification failure observed in mika#909 and mika#898:

1. Raise the `pull_request_review` body truncation cap in `mika-gateway/src/github.rs` from 2 KB to 16 KB so VERDICT tokens at the bottom of long QA review bodies survive the gateway transport hop.
2. Update `mika/docs/skills.md` § QA Verdict Contract to describe the actual hybrid review-state contract today (`pass → APPROVED`, `hold[*]/block[*] → COMMENTED`, never `CHANGES_REQUESTED`) instead of the stale "all COMMENTED" claim.

The change is scoped, additive, and behaviorally identical for any review under the previous 2 KB cap. A prompt-side defense-in-depth fix (`170148a2` — VERDICT-on-top in qa-review's body) already shipped to main on 2026-04-30 as a hot-fix; this plan delivers the structural fix.

This plan retroactively grooms an existing community contribution: alceops (alce.ops@gmail.com) authored both commits via PR #912. Their commits are cherry-picked onto branch `fix/911/review-truncation-cap`; they will receive `Co-authored-by` credit on the eventual PR.

## Problem Frame

**Symptom.** mika-qa's PR reviews silently failed verdict classification when the body exceeded 2,000 chars. The engine emitted `verdict_classification_failed` with `body_truncated: true`; mika-dev parked the PR with "wait for operator guidance" instead of routing the verdict.

| PR | Body size | VERDICT line offset | Failure mode |
|---|---|---|---|
| mika#909 | 3,302 chars | 2,758 | `state=APPROVED + VERDICT: pass` clipped at offset 2,000, mika-dev parked, manual unblock needed |
| mika#898 | similar | similar | same `body_truncated: true` shape, 2026-04-30 09:23:57 server.log |

**Root cause.** `crates/mika-gateway/src/github.rs` (pre-fix line 324):

```rust
let body = truncate_body(review.and_then(|r| r.body.as_deref()).unwrap_or(""), 2000);
```

The gateway clips `review.body` at 2,000 chars before forwarding to mika-agent. The engine's `verdict_handler` parses `(?mi)^VERDICT:\s*(.+)$` against the truncated body. mika-qa's body structure today is DIFF ANALYSIS + PLAN-AC VERIFICATION + BUILD VERIFICATION + (FINDINGS) + VERDICT + REASON — verdict at the bottom, the truncatable tail. Any non-trivial review crosses 2,000 chars and the verdict gets clipped.

**Documented anti-pattern.** This is the failure mode `feedback_transport_vs_workflow.md` warns against: a transport cap (2,000-char truncation) breaking a workflow contract (verdict-token-as-source-of-truth per `docs/skills.md` § QA Verdict Contract + mika#487).

**Documentation gap surfaced during diagnosis.** While auditing the QA Verdict Contract section in `docs/skills.md`, found that the doc still claims **all** mika-qa reviews use `state=COMMENTED`. That captured the pre-mika-skills#55 world and was never updated. The actual contract today is hybrid:

| Verdict | Review state | Routing |
|---|---|---|
| `pass` | `APPROVED` | Satisfies branch protection's "1 approval required" gate (mika-skills#55, 2026-03-30) so `pr_merge_with_gate` (mika-skills#119, 2026-04-11) clears without manual operator clicks |
| `hold[*]`, `block[*]` | `COMMENTED` | Stays advisory; preserves operator's "merge anyway" escape hatch |
| **never** | `CHANGES_REQUESTED` | Forbidden — conflates advisory verdicts with GitHub's review-required gate (mika#487 invariant) |

The doc's two existing claims need to be split: keep the load-bearing *"state field is NOT authoritative; the VERDICT: token in the body is"* sentence verbatim, and replace the stale *"Review state: COMMENTED (NOT APPROVED or CHANGES_REQUESTED)"* with the hybrid table.

## Requirements Trace

- **R1.** Gateway PR-review body truncation cap raised to 16 KB (review-body only; other body surfaces unchanged at 2 KB).
- **R2.** `mika/docs/skills.md` § QA Verdict Contract updated to describe the hybrid contract (`pass→APPROVED`, `hold/block→COMMENTED`, never `CHANGES_REQUESTED`) with cross-references to mika-skills#55 + mika-skills#119. The "state field is NOT authoritative" sentence preserved verbatim.
- **R3.** Behavioral test: long review body with `VERDICT: pass` near the top (post-`170148a2` shape) parses correctly through gateway → engine.
- **R4.** Behavioral test: long review body with VERDICT at the bottom (legacy shape, > 2 KB but ≤ 16 KB) parses correctly.
- **R5.** Existing `crates/mika-agent/src/server/verdict.rs` tests continue to pass.
- **R6.** Doc-sync CI job (`docs-sync` in `ci.yml`) passes — `docs/skills.md` change propagates via `scripts/sync-agent-docs.sh` to `crates/mika-agent/docs/`.
- **R7.** Compound doc lands at `docs/solutions/best-practices/gateway-truncation-cap-per-event-type-calibration-2026-05-01.md` capturing the institutional lesson — per-event-type cap calibration, mika#909/#898 incident reference, 16 KB derivation rationale, defense-in-depth pattern, config-flag follow-up forward-pointer. KG lexical ingestor (#689) + subject extractor (#690) consume on next agent restart.

## Scope Boundaries

- The prompt-side hot-fix (`170148a2` — qa-review system_prompt restructured to emit VERDICT + REASON as the first two body lines) is **already shipped to main**. This plan does NOT re-apply or modify that change.
- Only the `pull_request_review` body cap is raised. Issue bodies, PR bodies, and issue-comment bodies remain at the existing 2 KB cap — different concerns, different audiences.
- Magic-literal cleanup is in scope: replace the bare `2000` literal with named constants for clarity.

### Deferred to Separate Tasks

- **Configurable cap (env var).** If reviews routinely approach 16 KB in the future, a follow-up to make the cap configurable via `MIKA_GATEWAY_REVIEW_BODY_CAP` would be reasonable. Not needed today; QA verdicts run 3–5 KB typical with ~3× headroom at 16 KB.
- **Out-of-scope alternatives** (rejected during ideation, kept here for traceability): GitHub Check Runs as the verdict transport (over-engineered; introduces 6 new architectural concerns); PR labels for verdict taxonomy (mixes machine state into operator-curated `.github/labels.yml`); state+label decomposition (block path requires `CHANGES_REQUESTED` which IS the forbidden state per mika#487).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/github.rs` — `format_event_text` for `pull_request_review` events; the call site at line ~324 (pre-fix) calls `truncate_body(...)` with a hard-coded 2,000 char cap. Other body surfaces in the same file (issue, PR, comment) call `truncate_body(...)` with the same cap.
- `crates/mika-agent/src/server/verdict.rs` — VERDICT regex (`(?mi)^VERDICT:\s*(.+)$`) and first-match semantics. Test at `verdict.rs:203` confirms duplicate-VERDICT-line ordering.
- `crates/mika-agent/src/agent/messages/inbound.rs` — engine emits `verdict_classification_failed` with `body_truncated: true` when the truncated review body lacks a parseable VERDICT line.

### Institutional Learnings

- `feedback_transport_vs_workflow.md` (memory) — transport caps shouldn't compensate for workflow concerns. Direct ancestor of this fix.
- `feedback_prompt_enforcement_fragile.md` (memory) — prompt-only fixes drift; the gateway cap raise is the durable structural fix while VERDICT-on-top (in `170148a2`) is defense-in-depth.
- `mika/docs/solutions/best-practices/required-tools-gate-transport-contract-thin-final-turn-2026-04-29.md` — sibling pattern: transport-layer assumptions silently breaking agent contracts.

### External References

- GitHub PR review body render limit is well above 65 KB in the GitHub UI; the gateway transport cap, not GitHub's API, was the constraint.

## Key Technical Decisions

- **16 KB cap, not 64 KB or unlimited.** QA verdicts run 3–5 KB typical; 16 KB gives ~3× headroom over typical and survives long PLAN-AC sections on plan-shaped PRs. Keeping a finite cap preserves transport sanity for pathological cases.
- **Named constants over magic literals.** Two new module-level `const` declarations (`DEFAULT_GITHUB_BODY_TRUNCATION_CHARS = 2_000` and `GITHUB_REVIEW_BODY_TRUNCATION_CHARS = 16_000`) make the asymmetry self-documenting and make a future config-flag follow-up a 1-line change.
- **Review-body-only cap raise; other body paths unchanged.** Issue bodies, PR bodies, and issue-comment bodies have different audiences and different growth patterns; keeping them at 2 KB avoids unbounded transport surface expansion.
- **Hybrid-table doc reconciliation, not re-narrative.** The existing "state field is NOT authoritative" sentence is load-bearing and stays verbatim. Only the stale "Review state: COMMENTED (NOT APPROVED or CHANGES_REQUESTED)" claim is replaced — surgically.
- **mika#909 / mika#898 incidents named in test docstrings.** Tests cite the incident PR numbers so future readers understand why the regression bound exists at 16 KB and why the test exists at all.
- **Compound doc captures the per-event-type cap-calibration lesson.** See Unit 3. The institutional lesson — gateway truncation caps must be calibrated per-event-type, not globally, because body-shape pressure differs between operator-curated surfaces (issue/PR/comment) and agent-emitted structured surfaces (pull_request_review) — belongs in the searchable doc surface (`docs/solutions/best-practices/`), not in the issue body or PR description where it decays. Cross-referenced from the file path `docs/solutions/best-practices/gateway-truncation-cap-per-event-type-calibration-2026-05-01.md`.

## Open Questions

### Resolved During Planning

- *"Should we make the cap configurable?"* → Not now; deferred to a follow-up if real review bodies start approaching 16 KB. Keeping the change small.
- *"Should the prompt hot-fix `170148a2` be reverted now that the structural fix is in?"* → No. Belt-and-braces; if a future review somehow exceeds 16 KB, VERDICT-on-top guarantees the routing token survives. Two layers of defense are appropriate for the QA→merge contract.

### Deferred to Implementation

- (none — alceops's commits are already cherry-picked; the plan retrospectively grooms them.)

## Implementation Units

- [x] **Unit 1: Gateway cap raise + named constants**

**Goal:** Raise the `pull_request_review` body truncation cap from 2 KB to 16 KB, encoded as named module-level constants for clarity and future configurability.

**Requirements:** R1, R3, R4, R5

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-gateway/src/github.rs` (existing)
- Test: `crates/mika-gateway/src/github.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add two module-level constants at the same location as the existing `WEBHOOK_SKILL_DENYLIST`:
  - `DEFAULT_GITHUB_BODY_TRUNCATION_CHARS: usize = 2_000`
  - `GITHUB_REVIEW_BODY_TRUNCATION_CHARS: usize = 16_000`
- Replace the four `truncate_body(..., 2000)` call sites: issue, PR, comment branches all use the default constant; the `pull_request_review` branch uses the review constant.
- No other logic changes.

**Patterns to follow:**
- Module-level constant declarations match `WEBHOOK_SKILL_DENYLIST` placement and naming style.
- `truncate_body` signature unchanged.

**Test scenarios:**
- Happy path — `test_format_event_text_pr_review_preserves_verdict_past_legacy_cap`: construct a review body of ~3,300 chars (220×`"review detail. "`) with `VERDICT: pass` near the top; assert `VERDICT: pass` is present in the formatted event text and `[truncated]` is absent.
- Edge case — `test_format_event_text_pr_review_still_truncates_above_review_cap`: construct a review body > 16 KB; assert `[truncated]` is present (upper bound still enforced) AND `VERDICT: pass` is still extractable from the early portion of the body.
- Failure-shape regression — `test_format_event_text_pr_review_verdict_at_bottom_clips_when_body_exceeds_cap`: synthesize a > 16 KB body with `VERDICT: pass` at offset > 16,000 (end of the body); assert the verdict line is clipped from the formatted event text. This is the regression-fixture for the original mika#909/#898 failure shape — defends the structural cap bound. If a future cap raise becomes needed, this test fixture documents the failure mode that drove the prior raise.
- Existing tests for other body surfaces (issue, PR, comment) continue to pass with the renamed constant — no behavioral drift.

**Verification:**
- `cargo test -p mika-gateway` passes including the two new tests.
- Diff is single-file scope: `crates/mika-gateway/src/github.rs` only.

- [x] **Unit 2: docs/skills.md QA Verdict Contract reconciliation**

**Goal:** Replace the stale "all COMMENTED" claim in `docs/skills.md` § QA Verdict Contract with the hybrid table reflecting today's actual state-routing contract; preserve the load-bearing "state field is NOT authoritative" sentence verbatim.

**Requirements:** R2, R6

**Dependencies:** None (independent of Unit 1).

**Files:**
- Modify: `docs/skills.md` (lines ~1145–1170, § QA Verdict Contract)
- Modify: `crates/mika-agent/docs/skills.md` (auto-synced via `scripts/sync-agent-docs.sh`; verified by CI `docs-sync` job)

**Approach:**
- In `docs/skills.md` § QA Verdict Contract:
  - **Preserve verbatim:** the sentence *"The state field is NOT authoritative. The VERDICT: token in the body is."*
  - **Replace:** *"Review state: COMMENTED (NOT APPROVED or CHANGES_REQUESTED)"* with the hybrid table:
    - `pass` → `APPROVED` (cross-ref mika-skills#55 origin)
    - `hold[*]`, `block[*]` → `COMMENTED` (escape-hatch preservation)
    - **never** `CHANGES_REQUESTED` (mika#487 invariant)
  - Add cross-references to mika-skills#55 (2026-03-30) and mika-skills#119 (2026-04-11) for hybrid-contract origin.

**Patterns to follow:**
- Existing § QA Verdict Contract structure (heading, prose, table format) — surgical edit, not a rewrite.

**Test scenarios:**
- Test expectation: none — pure documentation change; covered by the `docs-sync` CI job confirming `docs/skills.md` propagates correctly to `crates/mika-agent/docs/skills.md`.

**Verification:**
- `docs-sync` CI job passes (catches any forgotten `scripts/sync-agent-docs.sh` run).
- Visual diff confirms the load-bearing sentence is unchanged.
- Cross-references to mika-skills#55 and mika-skills#119 are present and correctly numbered.

- [ ] **Unit 3: Compound doc — per-event-type cap-calibration**

**Goal:** Author a compound doc capturing the institutional lesson surfaced by mika#909 and mika#898: gateway truncation caps must be calibrated per-event-type, not globally, because body-shape pressure differs between operator-curated surfaces (issue/PR/comment, mostly short and human-authored) and agent-emitted structured surfaces (pull_request_review, structured-and-long with VERDICT tokens). This is the load-bearing lesson the /mika pipeline would have produced via /ce:compound; it doesn't exist in the repo because the community contributor lacked Claude Code locally.

**Requirements:** R7 (new — compound doc lands in `docs/solutions/best-practices/` for Knowledge Graph indexing).

**Dependencies:** None.

**Files:**
- Create: `docs/solutions/best-practices/gateway-truncation-cap-per-event-type-calibration-2026-05-01.md`

**Approach:**
- Frontmatter (YAML): `module: mika-gateway`, `tags: [transport, truncation, qa-verdict, per-event-calibration]`, `problem_type: workflow_issue`, `category: best-practices`, `applies_when` listing the conditions under which this lesson applies.
- Content sections:
  1. **Incident summary.** mika#909 (3,302-char body, VERDICT at offset 2,758, clipped) + mika#898 (same shape, 2026-04-30 09:23:57). Both produced `verdict_classification_failed` with `body_truncated: true`; mika-dev parked PRs awaiting operator unblock.
  2. **Root-cause analysis.** Single 2 KB cap applied uniformly across all `format_event_text` body surfaces. mika-qa's body shape (DIFF ANALYSIS + PLAN-AC VERIFICATION + BUILD VERIFICATION + (FINDINGS) + VERDICT + REASON, 3–5 KB typical) routinely placed the VERDICT line in the truncatable tail. Issue/PR/comment surfaces don't have the same growth pressure; the uniform cap was wrong for one event type.
  3. **Per-event-type calibration principle.** Caps should be sized to the *expected body shape and growth pressure* of each event type, not to a single global value. Operator-curated surfaces (mostly human-authored, short) tolerate small caps. Agent-emitted structured surfaces (long, machine-generated with required-token positioning) need larger caps OR position-independent parsing.
  4. **16 KB derivation rationale.** QA verdicts run 3–5 KB typical; ~3× headroom (matches mika#864 `MAX_REQUIRED_SUFFIX_LINES = 8` precedent for observed-typical 1-2 = 4× headroom multiplier shape). Bounded for transport sanity.
  5. **Defense-in-depth pattern.** Structural cap raise (durable bound) + prompt-side VERDICT-on-top (resilience against future model-output drift, position-independent regex captures first-match). Per `feedback_prompt_enforcement_fragile.md`, prompt rules drift; per general resilience, structural caps are durable but bounded. Both layers retained permanently.
  6. **Forward-pointer to config-flag follow-up.** If 3 documented incidents post-deploy involve QA verdicts exceeding 16 KB cap, escalate to `MIKA_GATEWAY_REVIEW_BODY_CAP` env var with 16 KB default. Until then, named constant suffices.
  7. **Cross-references.** mika#487 (verdict-token-as-source-of-truth contract), mika-skills#55 + mika-skills#119 (hybrid review-state contract), `feedback_transport_vs_workflow.md`, `feedback_prompt_enforcement_fragile.md`, `170148a2` (prompt hot-fix), this plan file.

**Patterns to follow:**
- Existing `docs/solutions/best-practices/*.md` files — frontmatter shape, section structure, citation density. Examples: `gateway-truncation-cap-per-event-type-calibration-2026-05-01.md` (this file) follows the structural-fix-after-incident pattern from `operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` and `required-tools-gate-transport-contract-thin-final-turn-2026-04-29.md`.

**Test scenarios:**
- Test expectation: none — pure documentation. Coverage is `docs-sync` CI job confirming propagation if the file appears in the agent-side mirror.

**Verification:**
- File exists at the named path with valid YAML frontmatter.
- Knowledge Graph lexical ingestor (#689) chunks the file on next agent restart; subject extractor (#690) builds graph entities. (Verification deferred to post-merge — KG ingestion happens at agent boot.)
- All cross-referenced files / issues / commits actually exist and the references are correctly numbered.

## System-Wide Impact

- **Interaction graph:**
  - `mika-gateway` → `mika-agent` (`/inbound` endpoint) — gateway forwards review bodies up to 16 KB; agent's `verdict_handler` regex parses position-independently, so it consumes any size up to the cap without code changes.
  - `mika-dev` → `pr_merge_with_gate` — once verdict classification stops failing, mika-dev's autonomous merge path resumes for non-trivial reviews. No code changes needed in mika-dev or mika-skills; the unblock is observed downstream.
  - `qa-review` skill → review body — body shape unchanged from `170148a2` (VERDICT-on-top); the structural cap raise is invisible to the skill prompt.
- **Error propagation:** No change. Engine still emits `verdict_classification_failed` if VERDICT line is missing or malformed; only the `body_truncated: true` flavor disappears for reviews ≤ 16 KB.
- **State lifecycle risks:** None. Gateway transport is stateless; cap is per-event.
- **API surface parity:** Gateway → agent transport contract is internal; no public API change.
- **Integration coverage:** A regression test driving a long review body through `format_event_text` covers the gateway side. The engine-side `verdict_handler` already has comprehensive tests at `crates/mika-agent/src/server/verdict.rs`; no new engine-side tests needed.
- **Unchanged invariants:**
  - The VERDICT-token-as-source-of-truth contract per mika#487 is preserved verbatim.
  - The "never `CHANGES_REQUESTED`" invariant is preserved.
  - The `state=APPROVED → pass` happy path (mika-skills#55) is preserved.
  - The `pr_merge_with_gate` skill's behavior is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Future review body exceeds 16 KB silently | Belt-and-braces: VERDICT-on-top hot-fix `170148a2` already in main; first-match regex + top-position survives any cap. Add Unit 1's upper-bound test (`test_format_event_text_pr_review_still_truncates_above_review_cap`) to assert behavior is well-defined at the cap. |
| Defense-in-depth durability | Prompt hot-fix `170148a2` (VERDICT-on-top) and structural cap raise are belt-and-braces by design. Per `feedback_prompt_enforcement_fragile.md`, prompt rules drift over time; structural cap is durable but bounded. **Permanent retention of both layers — do NOT plan a prompt revert.** Both layers already shipped; near-zero ongoing maintenance cost; resilience against future model-output drift. |
| Doc reconciliation drift in future | The doc names mika-skills#55 + mika-skills#119 explicitly so the hybrid contract has versioned provenance. Future changes to the contract should update the table and cross-refs together. |
| `docs-sync` CI job catches unforeseen propagation issues | Run `scripts/sync-agent-docs.sh` locally before commit; CI confirms. |
| alceops's authorship lost in cherry-pick / squash-merge | Cherry-pick preserves `Author:` field; squash-merge will combine commits — add `Co-authored-by: alceops <alce.ops@gmail.com>` trailer on the merge commit message to retain attribution. |

## Future Work

- **Config-flag escalation threshold.** If 3 documented incidents post-deploy involve QA verdicts exceeding the 16 KB cap, escalate to a configurable env var (`MIKA_GATEWAY_REVIEW_BODY_CAP` with 16 KB default). Until then, the named constant suffices. The named-constant refactor in Unit 1 makes this a 1-line change when the threshold is hit. Threshold rationale: matches the recurrence-required-for-structural-escalation pattern from `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` (architect persisted memory) — N=3 documented incidents establishes a real recurrence signal vs. a one-off.
- **`docs/skills.md` reconciliation discipline.** If additional `docs/skills.md` § QA Verdict Contract claims surface as stale during future incidents, file a separate reconciliation ticket. Surgical scope per `feedback_implementation_scope_bundling.md` — do not bundle full-section audits into incident-response work. The hybrid table reconciliation in Unit 2 sets the pattern.

## Documentation / Operational Notes

- **Compound doc to write at /ce:compound time:** `mika/docs/solutions/best-practices/gateway-transport-cap-vs-workflow-contract-2026-05-01.md` — institutional knowledge on the 2 KB → 16 KB transport cap raise pattern, citing mika#909 + mika#898 incidents and the `feedback_transport_vs_workflow.md` parent. Captures the named-constant convention so the next transport asymmetry has a precedent to follow.
- **No deployment-side change.** Gateway hot-reload on next deploy carries the new cap; no migration, no env var, no infrastructure update.
- **Knowledge Graph ingestion.** The plan doc + compound doc both land in `docs/plans/` and `docs/solutions/` respectively, where mika-arch's lexical ingestor (#689) chunks them and the subject extractor (#690) builds graph entities. This is the contributor's path-via-/mika-pipeline that PR #912 lacked.

## Sources & References

- Existing PR #912 (alceops): https://github.com/senara-solutions/mika/pull/912 — community contribution being retroactively groomed via this plan
- Cherry-picked commits: `f7691ccb` (gateway cap) + `575044eb` (docs reconciliation), originating from alceops's `aa39da84` + `b933bc2d` on PR #912's head branch
- mika#911 issue body — full AC list and rejected alternatives
- mika#487 — original incident establishing `state≠CHANGES_REQUESTED + VERDICT-token-in-body` contract
- mika-skills#55 (2026-03-30) — established `pass → --approve` for branch-protection-approval gate
- mika-skills#119 (2026-04-11) — `pr_merge_with_gate` depends on the approval being there
- mika#909 — first incident (live, manual-merge unblock applied)
- mika#898 — second incident (same `body_truncated` pattern, 2026-04-30 09:23:57)
- `170148a2` — prompt hot-fix already shipped (VERDICT-on-top defense-in-depth)
- `feedback_transport_vs_workflow.md` — institutional anti-pattern memory
- `docs/architecture/review-guide.md` — review principles mika-arch evaluates against
