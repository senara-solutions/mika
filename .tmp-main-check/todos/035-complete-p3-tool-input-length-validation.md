---
status: complete
priority: p3
issue_id: "035"
tags: [code-review, security, rust-v2]
dependencies: []
---

# No Input Length Validation on Tool Inputs

## Problem Statement

All tool implementations validate for empty strings but impose no maximum length limits. A multi-megabyte `value` in `update_core_memory` or `description` in `add_commitment` would be encrypted and stored before any budget check. While the LLM's own token limits mitigate this, a future direct API exposure could be weaponized.

**Why it matters:** Resource exhaustion (memory, storage, encryption CPU) through oversized inputs.

## Findings

- **Source:** Security Sentinel (M2)
- **Location:** `crates/mika-agent/src/tools/` — all 4 tool implementations

## Proposed Solutions

### Option A: Add length limits to all tool string inputs (Recommended)
- Check `value.len() > 10_000` and return ToolOutput::error
- Apply consistently across all tools
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] All tool string inputs have maximum length validation
- [ ] Error messages are helpful for the LLM to self-correct
