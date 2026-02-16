---
status: complete
priority: p2
issue_id: "215"
tags: [code-review, security, skills-system]
dependencies: []
---

# Prompt Injection via system_prompt.md Files

## Problem Statement
Skill prompt snippets from `system_prompt.md` are injected directly into the system prompt without sanitization. A malicious skill could override instructions, inject conflicting directives, or manipulate Claude's behavior (e.g., "Ignore all previous instructions").

## Findings
- Location: `crates/mika-agent/src/agent.rs` — snippet injection in `run_agent_inner`
- `write!(system, "\n## {} Skill\n{}\n", entry.manifest.name, snippet)` — raw string injection
- No content validation or sanitization
- Related to past finding #089 (stored-data prompt injection)
- Skills directory is user-writable, so compromised skills could inject arbitrary prompts

## Proposed Solutions

### Option 1: Validate/sanitize snippet content
- **Pros**: Blocks known injection patterns
- **Cons**: Hard to validate natural language comprehensively
- **Effort**: Medium
- **Risk**: Medium (cat-and-mouse with injection techniques)

### Option 2: Sandbox prompt snippets in a delimited section
- **Pros**: Claude can distinguish system vs skill instructions
- **Cons**: Not foolproof against sophisticated injection
- **Effort**: Small
- **Risk**: Low

### Option 3: Only load snippets from verified/builtin skills
- **Pros**: Simple trust boundary
- **Cons**: Limits third-party skill extensibility
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/agent.rs`, `crates/mika-agent/src/skills/loader.rs`

## Acceptance Criteria
- [ ] Prompt snippets are sanitized or bounded
- [ ] Injection attempts are detectable

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — security-sentinel agent
