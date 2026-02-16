---
status: complete
priority: p3
issue_id: 275
tags: [code-review, config, consistency]
dependencies: []
---

# Add MIKA_LOG_LEVEL env var check in early CLI init

## Problem Statement

The CLI's early log initialization in main.rs reads `log_level` from TOML config but does not check the `MIKA_LOG_LEVEL` environment variable. The server mode (via config-rs) respects this env var. This creates an inconsistency where `MIKA_LOG_LEVEL=debug` works for the server but not for the CLI.

## Findings

- **File**: `crates/mika-cli/src/main.rs:37-48`
- **Impact**: Low — users expecting consistent env var behavior may be surprised
- **Found by**: pattern-recognition-specialist

## Proposed Solution

Add env var check before the config file chain:

```rust
let log_level = std::env::var("MIKA_LOG_LEVEL").ok()
    .filter(|s| matches!(s.as_str(), "trace"|"debug"|"info"|"warn"|"error"|"off"))
    .or_else(|| agent_home.as_ref().and_then(|h| ...))
    // ... existing chain
```

## Acceptance Criteria

- [ ] `MIKA_LOG_LEVEL` env var is checked before config files
- [ ] Only allowlisted values accepted
- [ ] Tests cover env var override
