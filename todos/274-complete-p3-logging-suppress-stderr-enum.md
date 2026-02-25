---
status: complete
priority: p3
issue_id: 274
tags: [code-review, architecture, api-design]
dependencies: []
---

# Replace suppress_stderr bool with LogOutput enum in init_pretty

## Problem Statement

`init_pretty` uses a `suppress_stderr: bool` parameter, which is less self-documenting than the codebase's convention of using enums for behavioral modes (see `SilentTrigger`, `StopReason`, `ChatRole`). The call site passes `is_tui` which doesn't match the parameter name `suppress_stderr`.

## Findings

- **File**: `crates/mika-common/src/logging.rs:27`
- **Impact**: Low — single call site, functional behavior is correct
- **Found by**: pattern-recognition-specialist

## Proposed Solution

```rust
pub enum LogOutput {
    /// Pretty stderr + file (non-TUI CLI commands)
    PrettyAndFile,
    /// File only, no stderr (TUI mode)
    FileOnly,
}
```

## Acceptance Criteria

- [x] `suppress_stderr: bool` replaced with `LogOutput` enum
- [x] Call site in main.rs updated
- [x] All tests pass
