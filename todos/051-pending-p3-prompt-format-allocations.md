---
status: pending
priority: p3
issue_id: "051"
tags: [code-review, performance, rust-v2]
dependencies: []
---

# Unnecessary String Allocations in Prompt Assembly

## Problem Statement
`build_system_prompt` uses `format!()` calls that create intermediate String allocations, despite already pre-allocating with `String::with_capacity(4096)`. Using `write!` directly on the String avoids 5-8 small heap allocations per prompt build.

**Location:** `crates/mika-agent/src/prompt.rs:68,77`

**Reported by:** performance-oracle

## Proposed Solutions
Replace `prompt.push_str(&format!(...))` with `write!(prompt, ...)`.
- **Effort:** Small (5 minutes)

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
