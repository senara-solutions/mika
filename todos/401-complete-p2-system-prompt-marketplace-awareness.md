---
status: complete
priority: p2
issue_id: "401"
tags: [code-review, agent-native, marketplace, pr-56]
dependencies: []
---

# System prompt has no marketplace awareness

## Problem Statement

The system prompt in `prompt.rs` mentions skill tools but has no awareness of marketplace skills as a concept. When the agent sees `[marketplace]` tags in `list_skills` output, it has no context for what that means, cannot guide users toward CLI commands, and doesn't know that marketplace skills were installed from Git repos.

## Findings

- **Source**: agent-native-reviewer (both instances)
- **File**: `crates/mika-agent/src/prompt.rs:252-259`

## Proposed Solutions

### Option A: Add one line about marketplace skills (Recommended)

Add to the Tool Usage section:
```
"- Skills may be [built-in], [marketplace] (installed from Git repos via CLI), or [custom] (created locally). You can delete marketplace and custom skills.\n"
```

- Effort: Small (1 line)
- Risk: Low

## Acceptance Criteria

- [ ] System prompt mentions marketplace skill origin
- [ ] Agent can explain [marketplace] tag when users ask

## Resources

- `crates/mika-agent/src/prompt.rs:252-259`
