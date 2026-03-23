---
name: mika-issue
description: Create a single GitHub issue aligned with repo issue templates
argument-hint: "<description of the issue>"
---

Create a GitHub issue for the Mika project based on the user's description below.

## Input

$ARGUMENTS

## Instructions

1. **Classify** the issue as one of: `bug`, `enhancement`, `documentation`, `question`
2. **Write a clear title** — concise, imperative mood (e.g. "Fix TUI crash on terminal resize below 40 cols")
3. **Write the body** using the appropriate template format below
4. **Select labels** from `.github/labels.yml` (canonical source):
   - **Type label** (one): `bug`, `enhancement`, `documentation`, `question`
   - **Priority label** (one): `p0-critical`, `p1-important`, `p2-normal`, `p3-nice-to-have`
   - **Component label** (one or more): `agent-core`, `tui`, `team-engine`, `skill`, `gateway`, `infrastructure`, `dashboard`
5. **Milestone:** assign one if a matching open milestone exists, otherwise omit
6. **Show me the full `gh issue create` command** for review before executing

## Body Templates

### Bug

```
## Description
<What happened vs what was expected>

## Steps to Reproduce
1. ...
2. ...

## Expected Behavior
<What should happen>

## Actual Behavior
<What actually happens>

## Environment
- OS: <if relevant>
- Rust version: <if relevant>
- Mika version: <if relevant>
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

## Branch References

If a branch is associated with the issue, always link it as a clickable GitHub URL:
- Use `[branch-name](https://github.com/senara-solutions/mika/tree/branch-name)` format
- Place it in a `## Branch` section at the bottom of the body

## Command Format

Use a HEREDOC for the body:

```bash
gh issue create --title "..." --label "type,priority,component" --body "$(cat <<'EOF'
## Description
...

## Branch
[`branch-name`](https://github.com/senara-solutions/mika/tree/branch-name)
EOF
)"
```

Present the complete command and wait for my approval before running it.
