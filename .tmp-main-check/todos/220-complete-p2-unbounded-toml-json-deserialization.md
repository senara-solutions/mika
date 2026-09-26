---
status: complete
priority: p2
issue_id: "220"
tags: [code-review, security, skills-system]
dependencies: []
---

# Unbounded TOML/JSON Deserialization from Skill Files

## Problem Statement
`skill.toml` and `tools.json` files are deserialized without size limits. A malicious skill could contain a multi-GB file or deeply nested structure causing OOM or stack overflow during parsing.

## Findings
- Location: `crates/mika-agent/src/skills/index.rs:22` — `toml::from_str(&content)`
- Location: `crates/mika-agent/src/skills/loader.rs:18` — `serde_json::from_str(&content)`
- No file size check before reading
- No depth limit on deserialization

## Proposed Solutions

### Option 1: Add file size limits (e.g., 64KB for TOML, 256KB for JSON)
- **Pros**: Simple, effective
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/index.rs`, `crates/mika-agent/src/skills/loader.rs`

## Acceptance Criteria
- [ ] File size checked before reading
- [ ] Reasonable limits enforced (64KB TOML, 256KB JSON)

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — security-sentinel agent
