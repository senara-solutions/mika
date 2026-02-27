---
status: complete
priority: p3
issue_id: "315"
tags: [code-review, robustness]
dependencies: []
---

# Use filter_map in format_tool_summary_block for partial results

## Problem Statement

`format_tool_summary_block` uses `?` inside the for-loop which means if any single tool call entry is missing a "name" field, the entire function returns `None` and no summary block is shown. It should skip malformed entries and produce a partial summary.

Identified by: architecture-strategist

## Proposed Solutions

Replace the for loop with `filter_map`:
```rust
let parts: Vec<String> = calls.iter().filter_map(|call| {
    let name = call.get("name")?.as_str()?;
    let output = call.get("output_summary").and_then(|v| v.as_str()).unwrap_or("");
    let success = call.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    let status = if success { "" } else { " [FAILED]" };
    let short_output = truncate_summary(output, 80);
    Some(format!("{name}{status} → {short_output}"))
}).collect();
if parts.is_empty() { return None; }
```

## Technical Details

- **Affected file:** `crates/mika-agent/src/agent.rs:163-177`
- Effort: Small

## Acceptance Criteria

- [ ] Malformed entries are skipped, not fatal
- [ ] Partial summaries are produced when possible

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
