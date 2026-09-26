---
title: "fix: Honor pipeline-exempt label in qa-review docs-only block"
type: fix
status: active
date: 2026-05-10
---

# fix: Honor pipeline-exempt label in qa-review docs-only block

## Overview

The qa-review skill prompt hardcodes a `block[pipeline]` verdict for all docs-only PRs (Step 2 checks 1 and 2). No escape mechanism exists. mika-qa's LLM reads the orphaned `pipeline-exempt` label's description and fabricates an escape — advising operators to add the label — but the prompt never checks for it. The `pipeline-exempt` label itself is drift: it exists on the GitHub repo but not in `.github/labels.yml`, so EndBug/label-sync would not recreate it.

This fix implements the escape structurally: when a PR carries the `pipeline-exempt` label, Step 2's plan-doc and source-changes gates are bypassed, Step 2.5 (plan-AC verification) is skipped, and the review proceeds directly to Step 3 (diff review) for security/logic checks only.

## Problem Frame

Legitimate docs-only PRs exist: compound enrichments, doc refreshes after a fix ships, README updates, ADR additions. The autonomous loop should handle them without operator intervention. Three docs-only PRs (mika#1062, mika#1063, mika-platform#99) are currently stuck at REVIEW_REQUIRED because mika-qa structurally blocks them.

Two coupled defects:
1. `pipeline-exempt` label is drift — exists on repo but not in `.github/labels.yml`
2. qa-review Step 2 docs-only block has no honored escape mechanism

## Requirements Trace

- R1. Docs-only PR with `pipeline-exempt` label receives `VERDICT: pass` (not `block[pipeline]`)
- R2. Docs-only PR without `pipeline-exempt` label still receives `block[pipeline]` (current behavior preserved)
- R3. `pipeline-exempt` label is in `.github/labels.yml` (canonical for EndBug/label-sync)
- R4. Verdict text uses consistent `pipeline-exempt` casing (lowercase-kebab, not `Pipeline-Exempt`)
- R5. Post-merge verification: docs-only PR + `ready` + `pipeline-exempt` → autonomous merge path completes (integration — depends on verdict handler + merge automation)

## Scope Boundaries

- This fix covers docs-only PRs only. `pipeline-exempt` for non-docs-only cases (test-only PRs, dependency bumps) is a separate ticket if needed.
- No changes to GitHub branch protection requirements.
- No broader label drift audit — only canonicalizing `pipeline-exempt`.
- No changes to `qa_pr_view.sh` — labels are already in `SAFE_FIELDS` (line 34).

## Pinned Source

### Step 1 (data fetch — runs before the gate)

Step 1 fetches PR metadata via `qa_pr_view` and echoes `PR:`, `Size:`, `State:`. It is a pure data-fetch step with no gating logic. The gate is correctly placed after Step 1 because it needs PR labels from the `qa_pr_view` output.

### `qa_pr_view` output shape

`qa_pr_view.sh` (line 34) fetches: `title,body,additions,deletions,files,labels,state,headRefName,headRefOid,baseRefName,author`. The `labels` field is a JSON array of objects: `[{"name": "pipeline-exempt"}, {"name": "ready"}]`. The gate checks for `name == "pipeline-exempt"` in this array.

### Step 2 current text (lines 78–94)

```
**Step 2 — Pipeline compliance checks (hard blocks)**

Run these checks using `run_gh`. Combine into as few calls as possible. If ANY check fails, the verdict is a `block` sub-type (see below).

1. **Plan doc exists** — Check the PR diff for files matching `docs/plans/*.md`:
   run_gh("pr diff <PR_URL> --name-only | grep -q '^docs/plans/.*\\.md$'")
   If no match: `block[pipeline]` — "Missing plan document in docs/plans/"

2. **Source changes exist** — Check that the PR has changes beyond `docs/plans/`, `docs/solutions/`, and `.claude/`:
   run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/plans/' | grep -v '^docs/solutions/' | grep -v '^\\.claude/' | head -1")
   If empty: `block[pipeline]` — "No source changes beyond documentation"

3. **New external dependencies** — Review the diff for changes to `Cargo.toml` `[dependencies]` sections. ...
```

### Step 2.5 current text (lines 96–234)

Step 2.5 is "Plan-AC verification (gating)." It reads the plan-on-branch, extracts acceptance criteria, classifies each AC (Behavioral/Structural/Documentation/CI-deferred), and verifies against the diff or built binary. Every sub-step (2.5.1–2.5.8) depends on a plan document existing. Sub-step 2.5.1 explicitly blocks if no plan callout is found. Sub-step 2.5.2 blocks if the plan has no AC section. There are no documentation-applicable checks that would apply independently of a plan — the entire step is structurally inapplicable to docs-only PRs that have no plan.

### `.github/labels.yml` — State section (current)

```yaml
# ── State ─────────────────────────────────────────────────────────────────────
- name: ready
  color: "2ea44f"
  description: Approved for autonomous dispatch
```

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/qa-review/system_prompt.md` — Step 2 (lines 78–94) contains the pipeline checks. Step 2.5 (lines 96–234) contains plan-AC verification. The label check inserts at the top of Step 2, before check 1.
- `skills/bundled/qa-review/handlers/qa_pr_view.sh` — line 34 confirms `labels` is in the `SAFE_FIELDS` list; labels arrive as JSON array of `{name: string}` objects.
- `.github/labels.yml` — canonical label taxonomy synced by EndBug/label-sync. Currently missing `pipeline-exempt`.

### Institutional Learnings

- `docs/solutions/best-practices/verify-which-script-ci-actually-invokes-2026-04-28.md` — Prior incident where a `Pipeline-Exempt: docs-only` trailer was fabricated from the wrong script copy. Same class of bug: escape mechanism cited but not implemented.
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — Established that label-driven checks are the structural correlate for docs-only exemption, not trailers. This fix aligns: the `pipeline-exempt` label is the structural mechanism.
- `docs/solutions/prompt-engineering/block-verdict-classification-auto-retry.md` — QA verdicts must use sub-typed verdicts. The `pipeline-exempt` bypass emits `pass`, not a new sub-type.

## Key Technical Decisions

- **Gate position: top of Step 2, after Step 1.** Step 1 is a pure data-fetch step (PR metadata via `qa_pr_view`) with no gating logic — it must run because the gate needs PR labels. The gate is placed at the top of Step 2, before check 1, as a single check that short-circuits all three Step 2 checks AND Step 2.5.
- **Source-change guard on the gate.** The gate does NOT blindly honor `pipeline-exempt`. It confirms the diff actually contains only documentation files before bypassing pipeline checks. If `pipeline-exempt` is present but the PR has source changes (non-doc files), the label is ignored and normal Step 2 runs. This prevents misapplied labels from bypassing plan-AC verification on code PRs. Documentation files are defined as: `*.md`, files under `docs/`, `.claude/`, `.github/labels.yml`, and similar non-source paths. The existing Step 2 check 2 command already defines the "source changes" filter — the gate reuses the same `grep -v` pattern to confirm no source files exist.
- **Skip Step 2.5 entirely for pipeline-exempt PRs.** Step 2.5 (plan-AC verification) depends entirely on a plan document existing (2.5.1 blocks without plan callout, 2.5.2 blocks without AC section). Every sub-step is structurally inapplicable to docs-only PRs. The PLAN-AC VERIFICATION section in the verdict reads `PLAN-AC VERIFICATION: skipped (pipeline-exempt)`.
- **Step 3 diff review still runs.** Security checks (hardcoded secrets, SQL injection, eval/exec) apply to all PRs including docs-only. Step 4 (compound doc check) also still runs.
- **Verdict for passing pipeline-exempt PR.** The verdict is `pass` with reason "Docs-only PR; `pipeline-exempt` label honored — diff review clean." This is not a new verdict sub-type.

## Implementation Units

- [ ] **Unit 1: Add `pipeline-exempt` label gate to qa-review skill prompt**

  **Goal:** When `pipeline-exempt` label is present on a docs-only PR, bypass Step 2 pipeline compliance checks and Step 2.5 plan-AC verification entirely. Proceed directly to Step 3 diff review. Include a source-change guard to prevent misapplied labels from bypassing pipeline checks on code PRs.

  **Requirements:** R1, R2, R4

  **Dependencies:** None

  **Files:**
  - Modify: `skills/bundled/qa-review/system_prompt.md`

  **Approach:**
  - Insert a new sub-section at the top of Step 2, before check 1. Title: **Pipeline-exempt label bypass**.
  - The gate has two conditions (both must be true to bypass):
    1. PR labels from Step 1's `qa_pr_view` output contain `pipeline-exempt` (exact name match, lowercase-kebab)
    2. The diff contains only documentation files (reuse the Step 2 check 2 `grep -v` pattern to confirm no source files exist)
  - If both conditions met: note in response, skip Steps 2.1–2.3 and Step 2.5, jump to Step 3.
  - If `pipeline-exempt` present but source changes exist: note "pipeline-exempt label present but PR contains source changes — ignoring label, running normal pipeline checks." Proceed with Step 2 checks 1–3 normally.
  - If `pipeline-exempt` absent: no behavior change.
  - In the verdict template section, add a pipeline-exempt verdict example showing `PLAN-AC VERIFICATION: skipped (pipeline-exempt)` and `BUILD VERIFICATION: skipped (pipeline-exempt — no source changes)`.
  - Ensure the prompt never uses `Pipeline-Exempt` (capital case) — always `pipeline-exempt` (R4).

  **Verbatim gate text** (directional — the exact markdown to insert at the top of Step 2):

  ```markdown
  **Pipeline-exempt label bypass** — Before running checks 1–3, check the PR labels from Step 1's `qa_pr_view` output:

  If the labels include `pipeline-exempt`:
  1. Confirm the PR is docs-only by running the same source-change check as check 2:
     ```
     run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/plans/' | grep -v '^docs/solutions/' | grep -v '^\\.claude/' | grep -v '^\\.github/' | head -1")
     ```
  2. If the result is empty (no source files): skip checks 1–3 and Step 2.5 entirely. Note: "Pipeline-exempt: docs-only PR, skipping pipeline checks and plan-AC verification." Jump to Step 3.
  3. If the result is non-empty (source files present): note "pipeline-exempt label present but PR contains source changes — ignoring label." Continue with checks 1–3 normally.

  If the labels do NOT include `pipeline-exempt`: continue with checks 1–3 normally.
  ```

  **Patterns to follow:**
  - The existing Step 2 conditional structure: each check has a condition and a verdict if the condition fails. The new gate follows the same pattern but short-circuits all of Step 2 instead of blocking.
  - The existing Step 3e skip note pattern: `"BUILD VERIFICATION: skipped (no Behavioral ACs in plan)"` — reuse the same `skipped (reason)` shape.

  **Test scenarios:**
  - Structural: prompt text contains `pipeline-exempt` gate before Step 2 check 1
  - Structural: gate text includes source-change guard (the `grep -v` pattern confirming docs-only)
  - Structural: prompt text never contains `Pipeline-Exempt` (capital case) — grep for case-insensitive match and verify only lowercase-kebab form appears
  - Structural: verdict template includes `PLAN-AC VERIFICATION: skipped (pipeline-exempt)` example
  - Structural: gate text includes the "ignoring label" path for `pipeline-exempt` on code PRs

  **Verification:**
  - Reading the prompt, a docs-only PR with `pipeline-exempt` label would flow: Step 1 → pipeline-exempt gate (source-change check passes → bypass) → Step 3 diff review → Step 4 compound check → Step 5 post review. Steps 2.1, 2.2, 2.3, and 2.5 are skipped.
  - A code PR with `pipeline-exempt` misapplied: Step 1 → pipeline-exempt gate (source-change check fails → label ignored) → Step 2 checks 1–3 → Step 2.5 → normal flow.
  - A docs-only PR *without* `pipeline-exempt` still hits Step 2 check 2 and gets `block[pipeline]` as before.

- [ ] **Unit 2: Canonicalize `pipeline-exempt` label in `.github/labels.yml`**

  **Goal:** Add `pipeline-exempt` to the canonical label taxonomy so EndBug/label-sync recreates it on wipe.

  **Requirements:** R3

  **Dependencies:** None (can be done in parallel with Unit 1)

  **Files:**
  - Modify: `.github/labels.yml`

  **Approach:**
  - Add `pipeline-exempt` under a new `# ── Pipeline ──` section (or append to `# ── State ──` if a new section feels heavy for one label). Given that `ready` lives under `# ── State ──` and `pipeline-exempt` is a state signal, append under `# ── State ──`.
  - Label definition:
    - name: `pipeline-exempt`
    - color: a neutral/muted color (e.g., `"bfdadc"` matching `infrastructure`, or `"c5def5"` matching `p3-nice-to-have`)
    - description: `"Docs-only or non-code PR exempt from pipeline gates"`

  **Patterns to follow:**
  - Existing entries in `.github/labels.yml` — same YAML format, same comment style.

  **Test scenarios:**
  - Structural: `pipeline-exempt` entry exists in `.github/labels.yml` with `name`, `color`, and `description` fields
  - Structural: YAML is valid (no syntax errors)

  **Verification:**
  - `grep -q 'pipeline-exempt' .github/labels.yml` returns 0.
  - The label definition matches the name used in the orphaned GitHub label (preserving continuity).

## System-Wide Impact

- **Interaction graph:** The `pipeline-exempt` label gate affects only the qa-review skill prompt's Step 2 flow. No engine code changes. No other skills reference `pipeline-exempt`.
- **Error propagation:** If `qa_pr_view` fails to return labels (tool error), the existing flow applies — no label means no bypass, so the default `block[pipeline]` fires. This is safe.
- **API surface parity:** The `pipeline-exempt` label is already honored by CI's `verify-pipeline.sh` (via the `Pipeline-Exempt:` trailer path from mika#861). This fix makes qa-review consistent with CI.
- **Unchanged invariants:** Step 3 diff review (security checks) runs unconditionally regardless of `pipeline-exempt`. Step 5 verdict posting protocol is unchanged. The `pr review` call requirement is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLM may ignore the gate and still block pipeline-exempt PRs | The gate is positioned first in Step 2, before any blocking check runs. The skip instruction is directive ("skip to Step 3") not advisory. |
| Over-broad use: operators apply `pipeline-exempt` to code PRs to skip plan verification | Source-change guard in the gate: the gate confirms the diff is docs-only before honoring the label. If source files are present, the label is ignored and normal Step 2 runs. |

## Sources & References

- Related issues: mika#1064 (this ticket), mika#1062, mika#1063, mika-platform#99 (stuck PRs)
- Related code: `skills/bundled/qa-review/system_prompt.md`, `.github/labels.yml`
- Related learnings: `docs/solutions/best-practices/verify-which-script-ci-actually-invokes-2026-04-28.md`, `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
- Prior art: mika#861 (label-driven CI exemption for docs-only PRs)

## Acceptance Criteria

- [ ] Docs-only PR with `pipeline-exempt` label receives `VERDICT: pass` from mika-qa (not `block[pipeline]`)
- [ ] Docs-only PR without `pipeline-exempt` label still receives `block[pipeline]` (current behavior preserved)
- [ ] `pipeline-exempt` label is in `.github/labels.yml` with name, color, and description
- [ ] Verdict text uses consistent `pipeline-exempt` casing (never `Pipeline-Exempt`)
- [ ] Post-merge verification: docs-only PR + `ready` + `pipeline-exempt` → autonomous merge path completes without operator intervention (integration test — depends on verdict handler and merge automation, not directly delivered by this PR)
