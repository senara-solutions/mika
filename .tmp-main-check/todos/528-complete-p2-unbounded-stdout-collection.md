---
status: complete
priority: p2
issue_id: 528
tags: [code-review, performance, skills]
dependencies: []
---

# Unbounded stdout/stderr Collection in spawn_long_running_exec

## Problem Statement

The monitor task calls `child.wait_with_output().await` which buffers entire stdout and stderr into memory. For long-running subprocesses (potentially hours/days), a process streaming logs could accumulate gigabytes.

**Severity:** P2 — Potential memory exhaustion for chatty subprocesses.

## Findings

- `crates/mika-agent/src/skills/executor.rs:567` — `wait_with_output()` buffers both pipes

## Proposed Solutions

1. **Use wait() instead of wait_with_output(), only read stderr on failure**
   - Drop stdout handle, use `child.wait()`, cap stderr read with `take(MAX_OUTPUT_LEN)`
   - Pros: No unbounded memory, still captures error info
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Long-running subprocess stdout is not buffered into memory
- [ ] Stderr is capped to MAX_OUTPUT_LEN on failure path
