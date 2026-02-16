---
status: complete
priority: p3
issue_id: "223"
tags: [code-review, performance, skills-system]
dependencies: []
---

# Disk I/O Every Turn for Prompt Snippets

## Problem Statement
`load_prompt_snippet()` reads `system_prompt.md` from disk on every matched turn. Since all skills are `always_on`, this means 3 file reads per message. Files are small (<2KB) but this adds unnecessary latency and could cause issues on slow storage.

## Findings
- Location: `crates/mika-agent/src/skills/loader.rs:5-10`
- Called in agent.rs for each matched skill every turn
- Files never change during a session (they're config files)
- Current design comment says "no caching — edits take effect on next message" but this is over-optimized for a rare editing scenario

## Proposed Solutions

### Option 1: Cache snippets at startup in SkillEntry
- **Pros**: Zero per-turn I/O, simpler code
- **Cons**: Must restart to pick up edits
- **Effort**: Small
- **Risk**: Low

### Option 2: Keep per-turn loading (accept the cost)
- **Pros**: Live-reload of snippets
- **Cons**: Unnecessary I/O
- **Effort**: None
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/index.rs`, `crates/mika-agent/src/skills/loader.rs`

## Acceptance Criteria
- [ ] Prompt snippets loaded once at startup or cached after first load

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — performance-oracle agent
