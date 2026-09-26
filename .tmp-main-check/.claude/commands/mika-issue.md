---
name: mika-issue
description: Create a GitHub issue on any repo in the mika-platform workspace
argument-hint: "[repo] <description>"
---

Create a GitHub issue for the Mika platform based on the user's description below.

## Input

$ARGUMENTS

## Determine target repo

If `$ARGUMENTS` starts with a repo name (`mika`, `mika-cloud`, `mika-skills`, `claude-pilot`), use that repo and strip the prefix from the description.

Otherwise, infer from keywords:
- "helm", "chart", "k8s", "kubernetes", "provision", "terraform" → `mika-cloud`
- "skill", "marketplace", "manifest", "skill.toml", "self-dev" → `mika-skills`
- "claude-pilot", "relay", "headless", "worktree handler" → `claude-pilot`
- Everything else → `mika`

If ambiguous, ask the user.

## Instructions

1. **Classify** the issue as one of: `bug`, `enhancement`, `documentation`, `question`
2. **Write a clear title** — concise, imperative mood (e.g. "Fix TUI crash on terminal resize below 40 cols")
3. **Write the body** using the appropriate template:

### Bug
```
## Description
<What happened vs what was expected>

## Steps to Reproduce
1. ...

## Expected Behavior
<What should happen>

## Actual Behavior
<What actually happens>
```

### Enhancement
```
## Description
<What and why>

## Proposed Solution
<How to implement it>

## Alternatives Considered
<Other approaches, or "None">
```

### Documentation / Question
```
## Description
<Details>
```

4. **Select labels** from the target repo's `.github/labels.yml` if it exists. Common labels:
   - **Type:** `bug`, `enhancement`, `documentation`
   - **Priority:** `p0-critical`, `p1-important`, `p2-normal`, `p3-nice-to-have`
   - **Component:** repo-specific (check `.github/labels.yml`)
5. **Show the full `gh issue create` command** for review before executing. Always include `--repo senara-solutions/<target-repo>`.

## Command Format

```bash
gh issue create --repo senara-solutions/<repo> --title "..." --label "type,priority" --body "$(cat <<'EOF'
## Description
...
EOF
)"
```

Present the complete command and wait for approval before running it.

## Ticket vocabulary

Use exactly these terms when filing tickets. No fourth "umbrella" concept.

- **milestone** — single-repo multi-ticket grouping (GitHub milestone). Use when multiple tickets need to ship together in one repo.
- **project** — cross-repo or sprint-scoped grouping (GitHub project). Same GraphQL API as single-repo projects, just wider scope. Use when the work spans repos or represents a sprint.
- **sub-issue** — parent-child ticket link (GraphQL `addSubIssue` mutation). A link primitive, not a grouping pattern — use it to express that one ticket parents another, not to group tickets.

"Umbrella tickets" are deprecated (2026-04-22). If you're tempted to file one, the correct move is almost always a milestone (single repo) or a project (cross-repo / sprint). See `feedback_no_umbrella_tickets.md`.

## Sub-issue relationships

If the issue body contains a task list referencing other issue numbers (e.g., `- [ ] #410 — ...`) and those children should be formally linked to this parent, add sub-issue relationships after creation:

1. Extract all `#N` references from the task list
2. Get the new parent's node ID: `gh api graphql -f query='{ repository(owner: "senara-solutions", name: "<repo>") { issue(number: <N>) { id } } }' --jq '.data.repository.issue.id'`
3. For each child issue, get its node ID and add the relationship:
   ```bash
   gh api graphql -f query='mutation { addSubIssue(input: {issueId: "<parent_id>", subIssueId: "<child_id>"}) { issue { number } subIssue { number } } }'
   ```
4. Report which relationships were added
