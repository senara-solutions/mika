---
status: complete
priority: p3
issue_id: 606
tags: [code-review, quality, deduplication]
dependencies: []
---

# Extract `prompt_optional_secret` helper to reduce repetition

## Problem Statement

The setup wizard has three nearly identical blocks for prompting optional secrets (Anthropic API key, Brave Search key, OTLP auth header). Each block is ~7 lines with the same `Password::new() → trim → set_env_var` pattern. This is straightforward deduplication, not premature abstraction.

## Findings

- **Source:** code-simplicity-reviewer agent
- **Location:** `crates/mika-cli/src/commands/setup.rs` — lines 31-40, 44-53, 75-85
- **Evidence:** Three copy-pasted blocks with identical structure, differing only in prompt text and env var name

## Proposed Solutions

### Option A: Extract helper function (Recommended)
```rust
fn prompt_optional_secret(home_dir: &Path, env_key: &str, prompt: &str) -> Result<bool> {
    let value = Password::new()
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    mika_common::dotenv::set_env_var(home_dir, env_key, value)?;
    Ok(true)
}
```
- Effort: Small
- Risk: Low
- Pro: ~15 LOC saved, eliminates repetition
- Con: None

## Acceptance Criteria

- [x] Three password prompt blocks replaced with helper function calls
- [x] Wizard behavior unchanged (tests pass, manual verification)
