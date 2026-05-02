---
title: "fix: PR-writer must cite plan-doc path in PR body when plan is in diff"
type: fix
status: active
date: 2026-05-02
---

# fix: PR-writer must cite plan-doc path in PR body when plan is in diff

## Overview

The `/mika` command's PR creation step tells the agent to compose a PR body but does not instruct it to cite `docs/plans/*.md` paths. The `plan-doc-check` CI hook on mika-platform requires literal plan-doc paths in the PR body or commit body. Result: every `/mika` PR that adds a plan doc fails the hook (N=1 evidence: mika-platform#72, merged via bypass actor).

The fix adds a plan-doc citation instruction to the PR creation step in each repo's `/mika` command file. This is a prompt-level fix to a prompt-level omission — the structural enforcement (the CI hook) already exists and works correctly; the writer just doesn't emit the text the hook checks for.

## Problem Frame

The `plan-doc-check.sh` hook (mika-platform#64) uses `grep -oE 'docs/plans/[^[:space:]]+\.md'` to find plan-doc citations in the PR body and commit bodies. The `/mika` pipeline creates plan docs at step 1 (`/ce:plan`) and creates the PR at step 8, but the step-8 instructions say only "include `Closes #<number>` in the PR body" — nothing about citing the plan-doc path. The agent summarizes the plan in prose but never emits the literal path, so the grep finds nothing.

## Requirements Trace

- R1. When a `/mika` PR diff includes one or more files under `docs/plans/`, the PR body MUST contain a line citing each such path
- R2. The citation must be machine-parseable: `grep -E 'docs/plans/[^[:space:]]+\.md' PR_BODY` must match
- R3. The mechanism must work automatically — no manual intervention required
- R4. A follow-up `/mika` PR on mika-platform must pass the `Plan-Doc Citation` check without label/trailer fallback

## Scope Boundaries

- The `plan-doc-check.sh` hook itself is not modified
- Hook propagation to `mika` and `mika-skills` repos is a separate ticket

### Deferred to Separate Tasks

- Adding `plan-doc-check` workflow to `mika`, `mika-skills`, `mika-cloud`: separate ticket (the writer fix ships first so propagation is safe)

## Context & Research

### Relevant Code and Patterns

- `mika/.claude/commands/mika.md` lines 81-85 — PR creation step (step 8)
- `mika-platform/.claude/commands/mika.md` lines 199-203 — PR creation step (step 7)
- `mika-skills/.claude/commands/mika.md` lines 61-65 — PR creation step (step 7)
- `mika-cloud/.claude/commands/mika.md` lines 61-65 — PR creation step (step 7)
- `mika-platform/scripts/plan-doc-check.sh` — the CI hook that checks for plan-doc citations
- Existing pattern in mika's `/mika` command: "If a GitHub issue was referenced, include `Closes #<number>` in the PR body" — the plan-doc citation instruction follows the same shape

### Institutional Learnings

- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — this fix is the correct layer: the structural enforcement (CI hook) already exists; we're fixing the writer to comply with it, not adding a new prompt rule as enforcement
- `feedback_cross_repo_awareness.md` — when fixing shared patterns, fix all repos in one pass

## Key Technical Decisions

- **Fix all four repos, not just two:** The ticket says "out of scope" for propagating the hook, but the writer fix itself should be universal. Per `feedback_cross_repo_awareness.md`, fix the pattern everywhere in one pass. When the hook propagates to other repos, the writer will already be compliant.
- **Instruction placement:** Add the citation instruction as a sub-bullet of the existing PR creation step, following the established pattern of the `Closes #<number>` instruction.
- **Citation format:** Use `Plan: docs/plans/<file>.md` — a simple format that satisfies the hook's grep and is readable in PR bodies. The hook checks for `docs/plans/[^[:space:]]+\.md` anywhere in the text, so the exact prefix doesn't matter, but `Plan:` is clear and conventional.
- **Detection method:** Instruct the agent to check `git diff --name-only main...HEAD | grep '^docs/plans/.*\.md$'` before composing the body. This matches exactly what the CI hook checks (files in the diff range).

## Implementation Units

- [ ] **Unit 1: Add plan-doc citation instruction to mika's `/mika` command**

  **Goal:** Update the PR creation step in `mika/.claude/commands/mika.md` to instruct the agent to detect and cite plan-doc paths in the PR body.

  **Requirements:** R1, R2, R3

  **Dependencies:** None

  **Files:**
  - Modify: `.claude/commands/mika.md`

  **Approach:**
  Add a paragraph after the existing `Closes #<number>` instruction at step 8 (lines 81-85). The instruction tells the agent to: (1) check `git diff --name-only main...HEAD` for files matching `docs/plans/*.md`, and (2) include a `Plan: <path>` line in the PR body for each match. The instruction must be clear enough that the agent emits the literal path, not a prose description.

  **Patterns to follow:**
  - The existing `Closes #<number>` instruction at line 85 — same imperative style, same location

  **Test expectation:** none — this is a prompt instruction change to a markdown command file, not executable code. Verification is via the CI hook on the next PR.

  **Verification:**
  - Read the modified file and confirm the instruction is present and unambiguous
  - The next `/mika` PR on any repo that produces a plan doc should have the plan-doc path cited in the PR body

- [ ] **Unit 2: Add plan-doc citation instruction to mika-platform's `/mika` command**

  **Goal:** Same fix applied to the mika-platform repo's `/mika` command.

  **Requirements:** R1, R2, R3, R4

  **Dependencies:** None (can be done in parallel with Unit 1)

  **Files:**
  - Modify: `mika-platform/.claude/commands/mika.md` (relative to workspace root; this file lives in the mika-platform repo)

  **Approach:**
  Same instruction as Unit 1, added after the `Closes #<number>` line at step 7 (line 203). This is the repo where the `plan-doc-check` hook is already active, so this is the critical-path fix.

  **Patterns to follow:**
  - Same as Unit 1

  **Test expectation:** none — prompt instruction change

  **Verification:**
  - The next `/mika` PR on mika-platform that adds a plan doc passes the `Plan-Doc Citation` check without label/trailer fallback

- [ ] **Unit 3: Add plan-doc citation instruction to mika-skills and mika-cloud `/mika` commands**

  **Goal:** Forward-compatibility fix for when `plan-doc-check` propagates to these repos.

  **Requirements:** R1, R2, R3

  **Dependencies:** None

  **Files:**
  - Modify: `mika-skills/.claude/commands/mika.md`
  - Modify: `mika-cloud/.claude/commands/mika.md`

  **Approach:**
  Same instruction as Units 1-2, added after the `Closes #<number>` line at step 7 (line 65 in both files).

  **Patterns to follow:**
  - Same as Units 1-2

  **Test expectation:** none — prompt instruction change; no hook to test against yet

  **Verification:**
  - Read modified files and confirm instruction is present

## System-Wide Impact

- **Interaction graph:** The instruction affects how Claude Code (running `/mika`) composes PR bodies. No engine, tool, or skill code changes.
- **Error propagation:** If the agent fails to follow the instruction, the CI hook catches it (existing structural backstop). Failure mode is the same as today — hook rejects, bypass actor merges. The instruction reduces the frequency of that failure, not the severity.
- **API surface parity:** All four repos get the same instruction for consistency.
- **Unchanged invariants:** The `plan-doc-check.sh` hook is not modified. The plan-doc existence check semantics (F1 in the hook header) are unchanged. The three pass-paths (citation, label, trailer) are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Agent still summarizes in prose instead of citing the literal path | The instruction is explicit about "literal path"; the CI hook is the structural backstop |
| Agent runs `git diff` against wrong base ref | Instruction uses `main...HEAD` which matches the hook's check |

## Sources & References

- Related PRs/issues: senara-solutions/mika#931, senara-solutions/mika-platform#72, senara-solutions/mika-platform#62, senara-solutions/mika-platform#64
- Hook source: `mika-platform/scripts/plan-doc-check.sh`
- Institutional learning: `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
