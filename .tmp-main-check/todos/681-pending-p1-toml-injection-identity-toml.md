---
status: pending
priority: p1
issue_id: "681"
tags: [code-review, security, injection]
dependencies: []
---

# TOML injection via display_name and emoji in identity.toml

## Problem Statement

User input (`display_name`, `emoji`) from the agent wizard is interpolated directly into a TOML format string via `format!()` without escaping. A display name containing a double quote, backslash, or newline can produce malformed TOML or inject arbitrary TOML sections. For example, a name like `My Agent"\n[reflection]\nenabled = true\n#` could inject the `[reflection]` section, changing agent behavior.

The same pattern pre-exists in `crates/mika-agent/src/tools/create_agent.rs` line 91, where the LLM-provided display_name is written unsanitized — arguably worse since the LLM could be manipulated via prompt injection.

## Findings

- **Security Sentinel**: Medium severity. Easy to trigger (type a quote in the name prompt). Config corruption or injection of reflection settings.
- **Code Simplicity Reviewer**: Corroborated — identity.toml is written via raw `format!()`.

**Affected files:**
- `crates/mika-cli/src/commands/agents.rs` (lines 66-69 — wizard overwrite)
- `crates/mika-agent/src/tools/create_agent.rs` (line 91 — pre-existing, same pattern)

## Proposed Solutions

### Option A: Use toml crate serializer (Recommended)
Define a small serializable struct and use `toml::to_string_pretty()` to generate the file.
```rust
#[derive(serde::Serialize)]
struct Identity { name: String, emoji: String }
let identity = toml::to_string_pretty(&Identity {
    name: result.display_name.clone(),
    emoji: result.emoji.clone(),
})?;
```
- **Pros:** Correct escaping guaranteed by the toml crate, handles all edge cases
- **Cons:** Adds a small struct (could be inline or reuse existing Identity type)
- **Effort:** Small
- **Risk:** Low

### Option B: Manual TOML string escaping
Escape `\`, `"`, `\n`, `\r`, `\t` in user values before interpolation.
- **Pros:** No new types needed
- **Cons:** Easy to miss edge cases, duplicates toml crate logic
- **Effort:** Small
- **Risk:** Medium (incomplete escaping)

## Acceptance Criteria

- [ ] Display names containing `"`, `\`, `\n` produce valid identity.toml
- [ ] Same fix applied to `create_agent.rs` tool
- [ ] Existing tests pass; add test for round-trip with special characters
