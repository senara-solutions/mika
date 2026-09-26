---
name: mika-issues
description: Create multiple GitHub issues across any repos in the mika-platform workspace
argument-hint: "<list of issues to create>"
---

Create multiple GitHub issues for the Mika platform based on the user's description below.

## Input

$ARGUMENTS

## Instructions

1. **Parse** the input into individual issues
2. **For each issue**, determine the target repo using the same rules as `/mika-issue`:
   - Explicit prefix: `mika-cloud: add health endpoint` → mika-cloud
   - Keyword inference: "helm", "deploy" → mika-cloud; "skill", "manifest" → mika-skills; else → mika
   - Each issue in the batch can target a different repo
3. **Classify and draft** each issue following `/mika-issue` rules (type, title, body template, labels)
4. **Present a numbered summary table** for review:

   | # | Repo | Title | Type | Priority |
   |---|------|-------|------|----------|
   | 1 | mika | ... | enhancement | p2-normal |
   | 2 | mika-cloud | ... | bug | p1-important |

5. **After approval**, create issues sequentially using `gh issue create --repo senara-solutions/<repo>` with HEREDOC bodies
6. **Print a summary** with issue numbers and URLs grouped by repo

Wait for approval before creating any issues.

## Ticket vocabulary and sub-issue relationships

Use the standardized **milestone / project / sub-issue** vocabulary — canonical definitions are in `mika-issue.md § Ticket vocabulary`. The same rules apply to batch filing. If any created issue has children that should be linked, add sub-issue relationships after creation via the GraphQL `addSubIssue` mutation (see `mika-issue.md § Sub-issue relationships` for the exact API call).
