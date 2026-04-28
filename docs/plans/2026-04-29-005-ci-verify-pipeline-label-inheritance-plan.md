---
title: "ci(verify-pipeline): inherit documentation label from linked issue to exempt docs-only PRs"
type: ci
status: active
date: 2026-04-29
ticket: senara-solutions/mika#861
branch: ci/861/verify-pipeline-inherit-documentation
origin: senara-solutions/mika#860 (bucket-comparison + Pipeline-Exempt trailer port to mika)
related: senara-solutions/mika-platform#17 (sprint post-mortem establishing docs/source-balance protection), senara-solutions/mika-platform#18 (canonical bucket-comparison logic), senara-solutions/mika-platform#49 (extend Pipeline Artifacts to mika-platform's branch protection — companion)
---

# ci(verify-pipeline): inherit documentation label from linked issue to exempt docs-only PRs

## Overview

mika#861 layers label-driven exemption on top of the bucket-comparison + `Pipeline-Exempt:` trailer mechanism that PR #860 ported from mika-platform. The `documentation` label on a linked issue exempts the docs-only rejection; the trailer remains as a residual escape hatch with a `warn:` nudge toward the with-reason form.

Issue body is fully designed: 5-case stress test (A-E), sub-task breakdown (3), explicit one-directional asymmetry (`documentation` exempts source-required, NOT docs-required-when-source-changes), explicit rejection of path-pattern fallback with rationale. This plan pins script-implementation details and file paths against the current state of `scripts/verify-pipeline.sh`.

## Problem Frame

### Observed friction (from issue body + #860)

The current `Pipeline-Exempt:` trailer at `scripts/verify-pipeline.sh:82-106` (verified shape this turn) requires per-commit trailer text. PR #860 hit this friction shipping two `/ce:compound` outputs to `docs/solutions/best-practices/` — docs-only ship, no linked source changes, blocked by the bucket check until the trailer was added. The trailer mechanism is functional but client-side, easy to forget, and shifts the burden to commit-time.

### Why label-inheritance is the cleaner mechanism

Per the issue body's "Design (committed)" section: labels are observable in PR/issue UI, deterministic from issue classification, and honor `as above, so below` (issue-level classification → PR-level exemption). Server-side reads-at-check let operators label issues retroactively after PR open without rewriting commit history.

### Asymmetry is load-bearing

Cases A-E from the issue body, restated:
- **(A) `documentation` issue, docs-only diff** → PASS. Label exempts source-required check.
- **(B) `documentation` issue, mixed diff** → PASS. Both checks satisfied independently.
- **(C) `documentation` issue, source-only diff** → **FAIL**. The label is NOT a get-out-of-jail card for the docs-when-source check. Preserves mika-platform#17's protection.
- **(D) Unlabeled issue, docs-only** → FAIL. Label is the explicit classification; absence means no exemption.
- **(E) No linked issue, docs-only** → FAIL (or use trailer with reason). Compound docs deserve tracked rationale.

The asymmetry — `documentation` exempts source-required ONLY, not docs-required-when-source-changes — is the structural invariant the existing bucket comparison enforces.

## Requirements Trace

- **R1.** Sub-task 1: server-side label inheritance in `scripts/verify-pipeline.sh`. Parse `Closes #N` from PR body (`GITHUB_PR_BODY` env var when running in CI; falls back to `gh pr view --json body` if unset; local invocations skip if no PR context). **F2 resolution — branch-name fallback DROPPED.** The branch-name `/[0-9]+/` pattern silently misfires on branches like `feature/v2/new-thing` (extracts `2` → wrong-issue lookup → no `documentation` label found → reject arm fires with "label not found" rather than the truth "no linked issue"). Silent misfire is worse than no fallback per the explicit-over-silent principle established by mika-platform#17's path-pattern rejection. v1 reads only from `GITHUB_PR_BODY` (CI-injected, reliable) → `gh pr view --json body` (git context, reliable). If neither yields a `Closes #N` reference, fall through to trailer/reject — no implicit derivation. Call `gh api repos/{owner}/{repo}/issues/{N} --jq '.labels[].name'` to fetch issue labels. If `documentation` is present, exempt the docs-only rejection at line 91. New log line: `[pipeline-exempt: label] docs-only PR allowed by linked-issue documentation label (#N)` (per F5 structured-prefix sharpening). Asymmetry preserved: the code-only check at lines 102-106 remains untouched.
- **R2.** Sub-task 2: trailer reason-required warning. Current regex at line 82 is `^Pipeline-Exempt: docs-only(\s.*)?$` (verified — accepts both bare and with-reason). Extend the matching arm to:
  - With-reason form (capture group `\s+(.+)$`) → `[pipeline-exempt: trailer] docs-only PR allowed by Pipeline-Exempt trailer with reason: <reason>` (no warn).
  - Bare form → `warn: [pipeline-exempt: trailer] bare Pipeline-Exempt: docs-only trailer detected; prefer 'Pipeline-Exempt: docs-only — <reason>' for audit trail`.
  Same dual-form treatment for `code-only` at lines 85, 102. Backwards compat preserved (bare still passes). Structured prefix `[pipeline-exempt: trailer]` per F5 sharpening — paired with `[pipeline-exempt: label]` (R1) and `[pipeline-exempt: none]` (reject arm) for log triage.
- **R3.** Sub-task 3: documented rejection of path-pattern fallback + design rationale block. Add a header-comment block to `scripts/verify-pipeline.sh` (after the existing decision matrix at lines 14-21) explicitly covering:
  - **Path-pattern auto-exemption rejected** with the issue body's three-point rationale (visibility erosion, artifact-vs-classification inversion, DRY against `.github/labels.yml`).
  - **F3 — bare-trailer warn rationale:** "bare `Pipeline-Exempt: docs-only` is accepted for backward compatibility (PR #860 shipped both forms) but the with-reason form is required for auditability — bare emits a warn directing operators to the with-reason form."
  - **F4 — cross-repo `Closes` non-handling:** "v1 treats cross-repo `Closes senara-solutions/<other>#N` as no-link-in-this-repo (the `gh api repos/$repo/issues/$N` call would 404). The cross-repo close-pattern is what mika-platform#17 protects against; non-handling is intentional. Use trailer with reason for legitimate cross-repo docs-only ships."
  - **F1 — exemption priority order rationale:** "Label is checked FIRST as the classification-driven mechanism (issue body explicitly designates trailer as 'residual escape hatch'). Trailer is the explicit per-instance override. Three-path order: label → trailer → reject."
  Same comment block in PR description per the issue body's acceptance criteria #3.
- **R4.** Tests: bash test fixture covering all 5 cases A-E. Mock the `gh api repos/.../issues/N` call via a function override or `GH_API_MOCK_RESPONSES` env var (decided at implementation against the existing test idioms in `scripts/`). Each case asserts the exit code and the relevant log line in stderr. Trailer test: with-reason form produces `info:` line, bare form produces `warn:` line, both pass; malformed (e.g., `Pipeline-Exempt: docs-only-`) rejected. CI integration: open a draft PR per shape and confirm the check's outcome — implementer-discretion for which subset of A-E warrants a real CI dry-run vs. unit-test-only coverage.
- **R5.** No `gh` token scope expansion. The `gh api repos/.../issues/N --jq '.labels[].name'` call uses the existing `GITHUB_TOKEN` available in the workflow at `ci.yml:117-130`. The Pipeline Artifacts job already runs `bash scripts/verify-pipeline.sh origin/main` at line 130; `GITHUB_TOKEN` is implicitly available via `gh auth status` from the actions checkout step. Verify in implementation that the read-only `issues:read` scope is sufficient (likely yes — issue labels are public on public repos and accessible with default `GITHUB_TOKEN` permissions).
- **R6.** No new files. Single-file script edit + ci.yml unchanged + comment-block update. Test fixture is either inline in the script or a sibling `scripts/verify-pipeline-test.sh` (decision deferred to implementation).

## Proposed Fix

### Sub-task 1 — label inheritance in `verify-pipeline.sh`

**Where:** insert label-fetching logic after the existing `COMMIT_BODIES` extraction (line 81-ish, before the regex matching at line 82). Pseudocode:

```bash
# mika#861: fetch linked issue's labels for documentation-exemption check.
# Read in priority order: GITHUB_PR_BODY env, gh pr view (fallback when env empty), branch name.
linked_issue=""
if [ -n "${GITHUB_PR_BODY:-}" ]; then
    linked_issue=$(echo "$GITHUB_PR_BODY" | grep -oE 'Closes #[0-9]+' | head -1 | grep -oE '[0-9]+')
fi
if [ -z "$linked_issue" ] && command -v gh >/dev/null 2>&1; then
    pr_body=$(gh pr view --json body --jq .body 2>/dev/null || echo "")
    linked_issue=$(echo "$pr_body" | grep -oE 'Closes #[0-9]+' | head -1 | grep -oE '[0-9]+')
fi
if [ -z "$linked_issue" ]; then
    # Branch-name fallback: feat/<n>/..., fix/<n>/..., etc.
    branch_name=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
    linked_issue=$(echo "$branch_name" | grep -oE '/[0-9]+/' | head -1 | tr -d '/')
fi

issue_has_documentation_label=false
if [ -n "$linked_issue" ]; then
    repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || echo "")
    if [ -n "$repo" ]; then
        labels=$(gh api "repos/$repo/issues/$linked_issue" --jq '.labels[].name' 2>/dev/null || echo "")
        if echo "$labels" | grep -qx "documentation"; then
            issue_has_documentation_label=true
        fi
    fi
fi
```

Then in the docs-only rejection arm (line 91 currently), add a label-check branch BEFORE the trailer check:

```bash
if [ "$has_docs" = true ] && [ "$has_source" = false ]; then
    if [ "$issue_has_documentation_label" = true ]; then
        echo "info: docs-only PR allowed by linked-issue documentation label (#$linked_issue)" >&2
        exit 0
    fi
    if [ "$exempt_docs_only" = true ]; then
        # ... existing trailer-based exemption (with R2 reason-required warn) ...
    fi
    # ... existing reject ...
fi
```

The label check is consulted FIRST, then trailer, then reject. Three exits from the docs-only branch: label → exit 0 with info, trailer → exit 0 with info-or-warn (per R2), reject → exit 1.

**Code-only branch unchanged.** The asymmetry is structural: label exempts source-required, NOT docs-required-when-source-changes.

### Sub-task 2 — trailer reason-required warning

**Where:** lines 82-106 (`Pipeline-Exempt:` regex matches and warn lines). Replace the bare-only matches with dual-form matching:

```bash
if echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: docs-only\s+.+$'; then
    exempt_docs_only=true
    exempt_docs_reason=$(echo "$COMMIT_BODIES" | grep -oE '^Pipeline-Exempt: docs-only\s+.+$' | head -1 | sed 's/^Pipeline-Exempt: docs-only\s*//')
elif echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: docs-only$'; then
    exempt_docs_only=true
    exempt_docs_reason=""
fi
# ... same dual-match for code-only ...
```

Then in the exemption arm:

```bash
if [ "$exempt_docs_only" = true ]; then
    if [ -n "$exempt_docs_reason" ]; then
        echo "info: docs-only PR allowed by Pipeline-Exempt trailer with reason: $exempt_docs_reason" >&2
    else
        echo "warn: bare Pipeline-Exempt: docs-only trailer detected; prefer 'Pipeline-Exempt: docs-only — <reason>' for audit trail" >&2
    fi
    exit 0
fi
```

### Sub-task 3 — header comment + PR description note

Add to `scripts/verify-pipeline.sh` after the existing decision matrix (lines 14-21):

```bash
# Path-pattern auto-exemption was considered and rejected. Rationale:
# (1) Silent green CI with three exemption paths (label + trailer + path-pattern)
#     erodes structural visibility — operators can't tell at a glance which path
#     allowed a green check.
# (2) Path-touching is an artifact of classification, not the classification itself
#     (a docs-only PR is one whose intent is documentation, not one whose paths
#     happen to start with `docs/`). Gating on the artifact rather than the
#     decision is an inversion that erodes protections over time.
# (3) Creates a parallel taxonomy alongside `.github/labels.yml` (DRY violation).
# Two mechanisms (label = classification-driven, trailer = explicit override) is
# fine. Three is not. See mika#861 for the design discussion.
```

PR description carries the same note (per issue body acceptance criteria #3).

## Files to Modify

| File | Change |
|------|--------|
| `scripts/verify-pipeline.sh` | Sub-task 1: label-fetch logic + label-check branch in docs-only arm (~line 91). Sub-task 2: dual-form trailer matching (~lines 82-106) with reason-required warn on bare. Sub-task 3: header comment block (~lines 14-21+). |
| `scripts/verify-pipeline-test.sh` | New file (or appended inline test mode in the main script) — bash fixture covering cases A-E + trailer dual-form. Implementation chooses between sibling test script vs. inline self-test mode. |
| (no changes to `.github/workflows/ci.yml`) | The Pipeline Artifacts job at line 130 already runs `bash scripts/verify-pipeline.sh origin/main`; `GITHUB_TOKEN` available implicitly via the actions checkout step. |
| `CHANGELOG.md` | Add entry under "Changed" — "ci: verify-pipeline.sh now inherits the `documentation` label from linked issue to exempt docs-only PRs. Trailer mechanism remains as residual escape hatch with reason-required warn on bare form. Closes #861." |

No schema changes. No new dependencies. No new env vars (the new env-var read of `GITHUB_PR_BODY` is the existing actions convention; ci.yml passes it via `pull_request` event context).

## Verification

### Phase 0 — pre-implementation issue-body verification (F1 residual sharpening)

Before writing any implementation code, run:

```bash
gh issue view 861 --repo senara-solutions/mika --json body | grep -q "residual escape hatch"
```

**Expected:** match found (confirming the issue body's "residual escape hatch" trailer characterization that grounds R1's label-first priority order).

**Fallback if no match:** the priority-order claim is ungrounded against the issue body's current wording. Re-evaluate against whatever the body actually says — if body specifies trailer-first or is silent, swap the priority order to trailer → label → reject and update R1/R3 accordingly.

This Phase 0 verification was performed during grooming (the brief's "residual escape hatch" citation came from the issue-body content available in this session's context). Re-run at implementation time as a pre-commit gate so the priority-order claim stays grounded if the issue body is edited between groom and implementation. Same pre-commit-discovery discipline applied across mika#863 F8, mika#821 F6, mika-platform#52 F2.

### Unit (bash test fixture)

```bash
cd /data/workspace/mika-platform/.claude/worktrees/ci-861-verify-pipeline-inherit-documentation/mika
bash scripts/verify-pipeline-test.sh
```

The test fixture mocks `gh api repos/.../issues/N` via function override and asserts the exit code + relevant stderr line for each of cases A-E. Trailer dual-form has its own three sub-cases (with-reason → info, bare → warn, malformed → reject).

### CI integration (manual, post-merge)

1. Create a draft PR with a `documentation`-labeled linked issue and docs-only diff. Confirm Pipeline Artifacts check passes with `info: docs-only PR allowed by linked-issue documentation label (#N)` in logs.
2. Same setup but the diff includes source changes. Confirm bucket-comparison still passes (mixed diff is the green path regardless of label).
3. Same setup but the diff is source-only. Confirm Pipeline Artifacts FAILS — the docs-when-source-changes check fires regardless of label (case C, the asymmetry under test).
4. Unlabeled linked issue + docs-only diff. Confirm Pipeline Artifacts FAILS unless trailer is present (case D).

### Backwards compatibility

PR #860's bare-trailer form still passes. The new warn line is informational only; no behavior change for existing exemption-by-trailer workflows.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| `gh api` call fails (network, rate-limit, missing scope) — script silently fails-open and rejects an exemptable PR. | The `2>/dev/null || echo ""` pattern fails closed (treats as no label). The trailer mechanism remains as fallback. Operator's audit trail shows the gh failure (via `gh auth status` line) if they investigate. Real risk: a transient network glitch could cause spurious rejection; the trailer is the documented escape. Acceptable. |
| `Closes #N` parsing misses non-canonical close-keywords (`Fixes`, `Resolves`). | v1 parses only `Closes` — the existing convention in mika PR bodies. If `Fixes`/`Resolves` shapes appear, extend the regex; documented as a sentinel in Out of Scope. |
| Branch-name fallback wrongly picks up `release-plz-...` or other non-issue branches. | The fallback grep targets `/[0-9]+/` between slashes — release-plz branches don't match the pattern. False-positive rate negligible. |
| Operator mislabels an issue as `documentation` when it should be `bug` + `documentation`. | The label is checked for presence, not exclusivity. A `documentation+bug` issue exempts the docs-only check (intent is documentation-driven, even if there's also a bug aspect). Aligns with the issue body's design intent. |
| Rate-limit hit on `gh api` for high-PR-volume repos. | One call per PR check-run is well under any reasonable rate limit. If a workflow flakes due to rate limits, that's a broader CI concern, not specific to this script. |

## Out of Scope

- **Auto-applying labels from issue → PR at creation time.** Server-side check reads from issue at check-run, so PR doesn't need its own label. Client-side label inheritance (mika-dev dispatch path) is a separate possible enhancement that this ticket does not depend on.
- **Removing the trailer mechanism entirely.** Two mechanisms with crisp non-overlapping purposes (label = classification-driven, trailer = explicit override) is fine.
- **Other close-keywords (`Fixes`, `Resolves`).** v1 parses `Closes` only per existing mika convention. Sentinel: extend if those shapes appear in real PR bodies.
- **Cross-repo label sync.** `.github/labels.yml` + `EndBug/label-sync` already keeps `documentation` consistent across repos. This script reads labels at check-run; consistency is upstream.
- **mika-platform#49 — extend Pipeline Artifacts to mika-platform's branch protection.** Companion ticket, separate ship; consumes this script's behavior.

## Open Questions for mika-arch

1. **`Closes #N` cross-repo references.** Current parsing assumes the linked issue is in the SAME repo as the PR. If the PR body says `Closes senara-solutions/mika-skills#102`, the `gh api repos/.../issues/N` call against the PR's repo would 404. My read: cross-repo close-references are rare and historically mishandled (the cross-repo split pattern from mika-platform#17 is what we're protecting against, not enabling). v1 treats cross-repo `Closes` as "no linked issue in this repo" → no label exemption → trailer fallback. Defer-to-architect if a different read is preferred.
2. **Test-fixture isolation.** Mocking `gh api` in bash via function override is the canonical pattern, but if the team prefers a Python/Rust test runner, the fixture moves out-of-script. My read: bash-native test is appropriate scope for a bash script. Defer-to-architect.
3. **Label name canonicalization.** `documentation` is the literal label name (per `.github/labels.yml`). If the label gets renamed, the script breaks silently (no exemption, all docs-only rejected unless trailered). Defer-to-architect on whether to add a label-existence pre-check or accept the manual-coordination cost.

---

## Architect first-pass concerns (resolved in this revision)

This revision applies the six findings from mika-arch's first-pass review (session `2f02cec3-aee4-4d2e-bebd-a0c65a0af88b`).

### F1 — Exemption priority order pinned to label-first per issue body (BLOCKING, resolved)

The issue body's "Design (committed)" section explicitly designates the trailer as a "residual escape hatch, with required reason (target form)." "Residual" + "escape hatch" wording confirms label is the PRIMARY mechanism (classification-driven) and trailer is the FALLBACK / explicit per-instance override. Architect's option (a) applies — issue body specifies label-first explicitly. Plan's three-path order (label → trailer → reject) conforms. R3 now documents the priority-order rationale in the header comment block so future maintainers see the issue-body citation alongside the path-pattern rejection rationale.

### F2 — Branch-name fallback dropped (BLOCKING, resolved)

R1 now states the branch-name fallback is DROPPED for v1. Silent-misfire risk on branches like `feature/v2/...` (extracts `2` → wrong-issue lookup → reject arm with misleading "label not found" message) is worse than no fallback. Per the explicit-over-silent principle from mika-platform#17. v1 reads only from `GITHUB_PR_BODY` env (CI-injected) → `gh pr view --json body` (git context) → trailer/reject. No implicit derivation.

### F3 — Bare-trailer warn rationale in header comment (sharpening, applied)

R3's header-comment block now includes the bare-trailer warn rationale alongside the path-pattern rejection rationale. Future maintainers see why bare wasn't hard-rejected in the same place they see why path-pattern wasn't accepted.

### F4 — Cross-repo `Closes` non-handling in header comment (sharpening, applied)

R3's header-comment block now documents the cross-repo `Closes` non-handling explicitly. Maintainers seeing a cross-repo PR that needs a trailer will find the rationale in-place.

### F5 — Structured log prefix `[pipeline-exempt: <path>]` (sharpening, applied)

All three exemption-decision log lines now use the structured prefix: `[pipeline-exempt: label]`, `[pipeline-exempt: trailer]`, `[pipeline-exempt: none]`. CI log triage becomes a `grep '\[pipeline-exempt:` invocation.

### F6 — Asymmetry test in case fixture (non-blocking, addressed by issue body)

The issue body's case table already covers asymmetry: case C (`documentation` label + source-only diff → FAIL because docs-when-source-changes check fires regardless of label). Plan's R4 test fixture covers all 5 cases A-E from the issue body, which includes case C. No additional fixture needed; the asymmetry assertion is implicit in case C's expected exit-code-1 outcome.

---

## Architect verdict

- **First-pass (mika-arch session `2f02cec3-aee4-4d2e-bebd-a0c65a0af88b`):** ITERATE. Two blockers (F1 priority order, F2 branch-name fallback) + four sharpenings (F3 warn-bare rationale, F4 cross-repo non-handling, F5 structured prefix, F6 asymmetry coverage). All resolved in this revision.
- **Second-pass (same session, continuity preserved):** GROOMED. All six findings resolved (F1 conditionally — pending Phase 0 grep verification gate). Two remaining uncertainties correctly deferred (test-fixture format = implementation-detail; dual-form regex shape = readability-over-cleverness deferred). One residual: F1 grounded on the brief's citation of "residual escape hatch" wording from the issue body — Phase 0 grep gate (`gh issue view 861 ... | grep -q "residual escape hatch"`) added to verify the priority-order claim before the first implementation commit. Output-format compatibility confirmed: exit-code contract unchanged, new structured `[pipeline-exempt: ...]` log lines are additive (no downstream parser consumes them).
