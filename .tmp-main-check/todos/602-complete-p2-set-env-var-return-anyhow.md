---
status: pending
priority: p2
issue_id: 602
tags: [code-review, quality, conventions]
dependencies: []
---

# set_env_var returns std::io::Result instead of anyhow::Result

## Problem Statement

`set_env_var` returns `std::io::Result<()>` while every other public fallible function in mika-common returns `anyhow::Result<()>`. The CLAUDE.md convention states: "Error handling: anyhow::Result for application code."

## Findings

- Pattern recognition agent flagged this as the most significant convention deviation
- The caller in `setup.rs` wraps with `.with_context()` — would be cleaner if set_env_var used anyhow directly

## Proposed Solutions

Change signature to `anyhow::Result<()>` and add `.with_context()` on I/O operations, matching `write_default_if_missing` pattern in `home.rs`.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] `set_env_var` returns `anyhow::Result<()>`
- [ ] I/O errors include context messages
- [ ] Caller in `setup.rs` simplified
