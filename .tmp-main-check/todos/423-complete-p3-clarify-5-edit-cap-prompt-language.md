---
status: complete
priority: p3
issue_id: "423"
tags: [code-review, agent-native, reflection]
dependencies: []
---

# Clarify "5 Memory Edits" Prompt Language

## Problem Statement

The reflection prompt says "you have a maximum of 5 memory edits this session" but the 5-edit cap only applies to `core_memory_edit_count` which only `update_core_memory` increments. `store_fact` and `update_fact` have no edit counting. The agent could make 5 core memory edits AND unlimited fact mutations.

## Proposed Solutions

### Option A: Clarify prompt wording (Recommended)
Change to: "you have a maximum of 5 core memory edits this session" to accurately reflect enforcement.

### Option B: Add total edit counter
Add a second counter that all three tools increment. More complex, unclear benefit.

## Technical Details

- **Affected file**: `crates/mika-agent/src/agent.rs` (reflection trigger context string)

## Acceptance Criteria

- [ ] Prompt accurately describes what is capped
