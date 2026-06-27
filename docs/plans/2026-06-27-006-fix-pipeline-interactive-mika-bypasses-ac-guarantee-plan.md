# Plan: fix(pipeline): interactive `/mika` bypasses mika#1585 `## Acceptance criteria` guarantee

**Issue:** mika#1600
**Type:** bug fix
**Scope:** mika repo only (cross-repo propagation is out of scope per ticket)

## Summary

mika#1585 made `qa-review` hard-`block[pipeline]` any plan lacking a `## Acceptance criteria` section and placed the guarantee at the mika-arch grooming gate (mika#1559). That guarantee covers only the autonomous dispatch path. The interactive `/mika` pipeline (Mode 2) bypasses grooming entirely, so `/ce:plan` produces plans without the section, and qa-review blocks every interactively-dispatched PR.

## Problem Frame

- `/ce:plan` (third-party Compound Engineering plugin) emits `## Definition of Done` but never `## Acceptance criteria`. The plugin is marketplace-updated and not vendored — editing its template is out of scope (established by mika#1585).
- The autonomous path injects the section via mika-arch grooming (mika#1559 AC gate). The interactive path has no equivalent injection point.
- The recurring workaround — renaming `## Definition of Done` to `## Acceptance criteria` — papers over the gap and mislabels the section.

## Requirements

1. The interactive `/mika` pipeline must produce plans with a non-empty `## Acceptance criteria` section.
2. A structural gate must catch missing sections before PR creation (not at qa-review time).
3. `## Definition of Done` must remain intact — both sections coexist.
4. The fix must not edit the `/ce:plan` plugin template.

## Key Technical Decisions

- **Prose injection step in `mika.md` (U1):** Add a pipeline step after `/ce:plan` that instructs the model to ensure the `## Acceptance criteria` section exists. This mirrors the mika-arch grooming gate's role on the autonomous path. The step specifies two sourcing branches: (a) transcribe from the issue body's AC section when present, (b) derive from Requirements/Verification Contract when no issue body ACs exist.
- **Structural assertion in `verify-pipeline.sh` (U2):** Add a check that the plan doc contains `## Acceptance criteria` followed by at least one non-blank content line. This is defense-in-depth — the prose step is the primary injector, the script is the backstop that catches failures locally instead of at qa-review.
- **Both sections coexist:** The step explicitly prohibits renaming `## Definition of Done`. The plan ends up with both `## Definition of Done` (from `/ce:plan`) and `## Acceptance criteria` (injected by the new step).

## Implementation Units

### U1 — Prose injection step in `mika/.claude/commands/mika.md`

**File:** `.claude/commands/mika.md`

Add a new step between current step 1 (`/ce:plan`) and step 2 (`/ce:work`) in the Pipeline section. The new step (becomes step 2, shifting subsequent steps):

```
2. **Ensure `## Acceptance criteria` section exists in the plan.** `/ce:plan` does not emit this section — it must be added after plan generation. Rules:
   - If the referenced issue body has an `## Acceptance criteria` section, transcribe its criteria verbatim into a new `## Acceptance criteria` section in the plan (placed after `## Definition of Done`).
   - If the issue body has no `## Acceptance criteria` section (or no issue was referenced), derive concrete, testable acceptance criteria from the plan's Requirements and Verification Contract sections.
   - Do NOT rename `## Definition of Done` to `## Acceptance criteria`. Both sections must coexist.
   - The section must contain at least one markdown checkbox item (`- [ ] <criterion>`).
```

Renumber subsequent steps (current 2 becomes 3, etc. through to step 7 becoming step 8; cleanup step numbers adjust accordingly).

### U2 — Structural assertion in `scripts/verify-pipeline.sh`

**File:** `scripts/verify-pipeline.sh`

Add an AC-section check after the `PLAN` variable is captured (around line 103) and before the bucket-comparison logic. The check runs only when a plan file is detected in the diff:

```bash
# --- mika#1600: Acceptance criteria section check ---
if [[ -n "$PLAN" ]]; then
  # Take the first plan file if multiple exist
  PLAN_FILE=$(echo "$PLAN" | head -1)
  if [[ -f "$PLAN_FILE" ]]; then
    # Check for ## Acceptance criteria heading
    if ! grep -q '^## Acceptance criteria' "$PLAN_FILE"; then
      echo "FAIL: Plan '$PLAN_FILE' missing '## Acceptance criteria' section. See mika#1600." >&2
      ERRORS=$((ERRORS + 1))
    else
      # Check for at least one non-blank line after the heading
      AC_CONTENT=$(sed -n '/^## Acceptance criteria/,/^## /{ /^## /d; /^[[:space:]]*$/d; p; }' "$PLAN_FILE")
      if [[ -z "$AC_CONTENT" ]]; then
        echo "FAIL: Plan '$PLAN_FILE' has empty '## Acceptance criteria' section. See mika#1600." >&2
        ERRORS=$((ERRORS + 1))
      fi
    fi
  fi
fi
```

The check is gated on `$PLAN` being non-empty (a plan file exists in the diff) AND the file existing on disk. This avoids false positives on code-only PRs (which have no plan) and on historical plans not present in the worktree.

### U3 — Test coverage in `scripts/verify-pipeline-test.sh`

**File:** `scripts/verify-pipeline-test.sh`

Add three test cases after the existing trailer tests:

1. **Plan with AC section present** — mixed diff (plan + source), plan contains `## Acceptance criteria` with content -> PASS.
2. **Plan with missing AC section** — mixed diff, plan has no `## Acceptance criteria` heading -> FAIL with "missing '## Acceptance criteria' section" message.
3. **Plan with empty AC section** — mixed diff, plan has `## Acceptance criteria` heading but no content lines before next heading -> FAIL with "empty '## Acceptance criteria' section" message.

## Scope Boundaries

### In scope
- `mika/.claude/commands/mika.md` — add AC injection step (U1)
- `scripts/verify-pipeline.sh` — add AC section assertion (U2)
- `scripts/verify-pipeline-test.sh` — add test cases (U3)

### Out of scope
- Editing `/ce:plan` plugin template (marketplace-updated, not vendored)
- Changing qa-review's block behavior (intended backstop)
- Cross-repo propagation to mika-cloud, mika-skills, mika-platform
- Modifying mika-arch grooming gate or autonomous path

## Deferred to Implementation

- Exact line placement of the assertion in `verify-pipeline.sh` relative to existing checks
- Whether the `sed` command for content extraction needs adjustment for plans where `## Acceptance criteria` is the last section (no trailing `## ` delimiter) — the `sed` range `/^## Acceptance criteria/,/^## /` handles this via implicit EOF termination

## Verification Contract

- `bash scripts/verify-pipeline-test.sh` passes with the three new AC test cases
- Manual inspection: a plan without `## Acceptance criteria` triggers `verify-pipeline.sh` failure
- Manual inspection: a plan with a non-empty `## Acceptance criteria` section passes `verify-pipeline.sh`
- The `## Definition of Done` section is untouched in plans produced after the change

## Definition of Done

- [ ] U1 implemented: `mika.md` has AC injection step after `/ce:plan`
- [ ] U2 implemented: `verify-pipeline.sh` asserts AC section presence and non-emptiness
- [ ] U3 implemented: `verify-pipeline-test.sh` has three new test cases covering AC check
- [ ] All existing `verify-pipeline-test.sh` tests still pass
- [ ] Step numbering in `mika.md` is consistent after insertion

## Acceptance criteria

- [ ] AC1. `mika/.claude/commands/mika.md` (interactive pipeline) contains a step after `/ce:plan` that instructs the model to ensure a non-empty `## Acceptance criteria` section exists in the plan. The step specifies: transcribe from the issue body's AC section when present; derive from Requirements/Verification Contract when no issue body ACs exist. The step explicitly prohibits renaming `## Definition of Done` to `## Acceptance criteria`.
- [ ] AC2. `scripts/verify-pipeline.sh` (or the repo's canonical pre-PR gate script) contains a structural assertion that the plan doc has a `## Acceptance criteria` heading followed by at least one non-blank line of content. Failure prints a message naming the missing section and referencing this ticket.
- [ ] AC3. End-to-end: an interactive `/mika #<issue>` run against an issue WITH an `## Acceptance criteria` section in its body produces a plan with that section transcribed (not renamed from DoD), and `verify-pipeline.sh` passes.
- [ ] AC4. End-to-end: an interactive `/mika #<issue>` run against an issue WITHOUT an `## Acceptance criteria` section in its body produces a plan with ACs derived from Requirements/Verification Contract, and `verify-pipeline.sh` passes.
- [ ] AC5. Regression: a plan missing the `## Acceptance criteria` section causes `verify-pipeline.sh` to fail with a clear error message (not a silent pass).
- [ ] AC6. The `## Definition of Done` section produced by `/ce:plan` is left intact — not renamed or removed. Both sections coexist in the plan.
