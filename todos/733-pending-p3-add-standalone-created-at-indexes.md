---
status: pending
priority: p3
issue_id: "733"
tags: [code-review, performance, observability]
dependencies: []
---

# Add standalone `created_at` indexes for unfiltered dashboard queries

## Problem Statement

Queries without `agent_id` filter (`ORDER BY created_at DESC LIMIT N`) can't use the composite `(agent_id, created_at)` index, causing full table scans at scale.

## Findings

- **Agent**: performance-oracle
- **Files**: `crates/mika-agent/src/db.rs`

## Proposed Solutions

Add indexes: `idx_llm_calls_created ON llm_calls(created_at DESC)`, `idx_tool_calls_created ON tool_calls(created_at DESC)`. Follow-up PR.
