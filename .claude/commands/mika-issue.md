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
- "helm", "chart", "deploy", "k8s", "kubernetes", "provision", "terraform" → `mika-cloud`
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
