---
name: mika-issues
description: Create multiple GitHub issues in batch, aligned with repo issue templates
argument-hint: "<list of issues to create>"
---

Create multiple GitHub issues for the Mika project based on the user's description below.

## Input

$ARGUMENTS

## Instructions

1. **Parse** the input into individual issues
2. **For each issue**, classify and draft it following the same rules as `/mika-issue`:
   - Classify as `bug`, `enhancement`, `documentation`, or `question`
   - Write a clear, concise title
   - Write the body using the appropriate template (Bug, Enhancement, or Documentation/Question — see `/mika-issue` for formats)
   - Select labels: type + priority (`p0-critical` / `p1-important` / `p2-normal` / `p3-nice-to-have`) + component (`agent-core` / `tui` / `team-engine` / `skill` / `gateway` / `infrastructure`)
   - Assign a milestone if a matching open one exists
3. **Present a numbered summary table** for review:

   | # | Title | Type | Priority | Component |
   |---|-------|------|----------|-----------|
   | 1 | ... | enhancement | p2-normal | agent-core |
   | 2 | ... | bug | p1-important | tui |

4. **After approval**, create the issues sequentially using `gh issue create` with HEREDOC bodies (same format as `/mika-issue`)
5. **After all issues are created**, print a summary with issue numbers and URLs

If the issues share a theme, suggest grouping them under a milestone.

Wait for my approval before creating any issues.
