---
status: complete
priority: p3
issue_id: 299
tags: [code-review, testing, prompt]
dependencies: []
---

# Add drift-detection test for CLI Reference command list

## Problem Statement

The CLI Reference section in `prompt.rs` hardcodes a list of CLI commands. If new commands are added to the CLI (in `cli.rs`) without updating the prompt, the list silently drifts. There is no compile-time or test-time mechanism to detect this.

## Findings

- **Architecture Strategist:** The CLI command list in the prompt is a hardcoded string that duplicates information from the actual `Commands` enum in `crates/mika-cli/src/cli.rs`. A drift-detection test would create a tripwire that fires when new commands are added without updating the prompt.
- Currently the prompt is in `mika-agent` crate and the CLI definition is in `mika-cli` crate, making cross-crate testing non-trivial.

## Proposed Solutions

### Option 1: Integration test in mika-cli that checks prompt content
- Since `mika-cli` depends on `mika-agent`, add a test in the CLI crate that builds the prompt and checks it mentions each top-level command name.
- **Pros:** Catches drift automatically, runs in CI
- **Cons:** Cross-crate coupling in tests
- **Effort:** Medium
- **Risk:** Low

### Option 2: Comment in prompt.rs listing the source of truth
- Add a comment: `// Keep in sync with crates/mika-cli/src/cli.rs Commands enum`
- **Pros:** Zero effort, documents intent
- **Cons:** Relies on human attention, no automated enforcement
- **Effort:** Small
- **Risk:** Low (but no enforcement)

## Technical Details

- **Prompt file:** `crates/mika-agent/src/prompt.rs` (write_cli_section)
- **CLI definition:** `crates/mika-cli/src/cli.rs` (Commands enum)

## Acceptance Criteria

- [ ] Mechanism exists to detect when CLI commands are added without updating the prompt
- [ ] Does not create brittle tests that break on every minor CLI change

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Hardcoded prompt content drifts from source of truth |
