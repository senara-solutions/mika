---
title: "feat: Institutionalize infra-fix compounding rule (case: release automation drift)"
type: feat
status: active
date: 2026-04-23
---

# Institutionalize Infra-Fix Compounding Rule

## Overview

Land a compound doc that names the "infra-fix compounding" practice using release-automation chronic drift as the case study. The deliverable is the rule, not the artifact — the doc is case evidence; the rule is what future infra work inherits.

## Problem Frame

Release automation for this Rust workspace has accumulated 14+ fix commits across three tools over ~7 weeks with zero durable institutional memory. The rule "look back before shipping an infra fix; compound when you see >=3 prior fixes in the class" was identified and captured in session memory (`feedback_compound_infra_fixes.md`). This ticket lands the case-study doc that demonstrates the rule and cross-references the memory entry.

## Requirements Trace

- R1. `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` lands with appendix content from issue #776
- R2. Frontmatter, cross-references, and Class A/B/C/D taxonomy preserved
- R3. Cross-reference to `feedback_compound_infra_fixes.md` memory entry included
- R4. No changes to `.github/workflows/release-plz.yml` or `release-plz.toml`
- R5. `/ce:compound` output institutionalizes the rule, not the drift doc content

## Scope Boundaries

- No workflow file changes (scoped to mika#775)
- No changes to the memory file (outside repo state)
- No retroactive compound docs for the 14+ prior fixes
- No code changes of any kind — this is purely documentation

## Context & Research

### Relevant Code and Patterns

- `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` — existing release-plz setup doc (cross-referenced from the new doc)
- `docs/solutions/ci-cd/` — 5 existing docs following consistent frontmatter + problem/solution structure
- All solution docs use YAML frontmatter with `title`, `date`, `category`, `tags`, `modules`, `severity`, `resolved`

### Institutional Learnings

- `feedback_compound_infra_fixes.md` (session memory) — the rule being institutionalized: grep git log before fixing, compound when >=3 prior fixes in the class

## Key Technical Decisions

- **Content is pre-drafted:** The issue appendix contains peer-reviewed content. `/ce:work` should refine, not rewrite.
- **verify-pipeline.sh expects source changes:** The pipeline verification script checks for source code changes beyond plan/config. Since this is a docs-only ticket, the compound doc in `docs/solutions/` counts as both the deliverable AND satisfies the compound-doc check. The "source code changes" check may need the solution doc to satisfy it — if not, the verification failure is expected and documented.

## Implementation Units

- [ ] **Unit 1: Land the compound doc**

**Goal:** Create `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` with the full content from issue #776's appendix.

**Requirements:** R1, R2, R3

**Files:**
- Create: `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`

**Approach:**
- Copy the drafted content from the issue appendix verbatim
- Verify frontmatter fields match the repo's solution doc conventions (compare with existing `ci-cd/` docs)
- Ensure cross-reference to `feedback_compound_infra_fixes.md` is in the final section
- Ensure cross-reference to `rust-workspace-release-plz-github-actions.md` uses relative path
- Verify Class A/B/C/D structure is clean and the "failure classes that outlive tool choice" framing is preserved

**Patterns to follow:**
- `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` — frontmatter and structure conventions

**Test expectation:** none — pure documentation artifact with no behavioral change

**Verification:**
- File exists with correct frontmatter fields
- All four failure classes (A, B, C, D) are present with tables
- Cross-references to related docs and memory entry are intact
- `resolved: false` reflects the open Class C issue (mika#775)

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `verify-pipeline.sh` fails on "source code changes" check for a docs-only ticket | The solution doc itself may satisfy the check; if not, the failure is a known limitation of the script for docs-only tickets |

## Sources & References

- Related issue: #776
- Related issue: #775 (Class C fix, out of scope)
- Related code: `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md`
- Memory: `feedback_compound_infra_fixes.md`
