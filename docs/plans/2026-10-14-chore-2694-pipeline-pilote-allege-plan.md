---
date: 2026-10-14
type: chore
issue: mika#2694
---

# Plan: pilot flights end lean (mika#2694)

## Problem

On 2026-10-14, three implementation flights died at their turn ceiling after their last work commit. The flights were mika#2683 and mika#2654 (201/200) and mika#2687 (151/150). Two of them died inside `ce-simplify-code` and its review sub-agents, which `/ce-work`'s shipping workflow runs before `ce-code-review`. None opened its PR. mika-qa then reviews every PR at code level.

## Fix

Two lines in `CLAUDE.md`, the file every pilot loads, scoped to loop sessions started with `/ce-work <plan>`:

- the project simplification threshold, a hook the plugin provides: none;
- code review delegated to mika-qa, with a fixed line in the PR body.

The full test suite, clippy, the repo lints, `verify-pipeline.sh` and opening the PR all stay. `.claude/commands/mika.md`, the interactive path, is unchanged.

## Accepted loss

mika-qa's diff review uses a closed checklist (Step 3b of `qa-review/system_prompt.md`): secrets, `unsafe`, dynamic execution, SQL, obvious logic errors, dead code, missing I/O error handling, and missing negative assertions. It never compiles. Reuse, efficiency and non-obvious correctness are no longer checked by anyone. The trial measures this loss.

## Acceptance criteria

- AC1: `CLAUDE.md` carries the two directives, scoped to `/ce-work` loop sessions, and `claude_md_size` stays green.
- AC2 (trial, first lean flight after merge, main-session transcript):
  - the flight opens its PR within its turn budget;
  - neither `ce-simplify-code` nor `ce-code-review` is invoked;
  - full verification runs before the PR.
- AC3: if either skill is invoked despite the directive, the trial records the hook as inert and names the next lever.
- AC4: after the trial, a note compares what mika-qa flagged on the lean PR with what simplify and review flagged on mika#2683 and mika#2654.
