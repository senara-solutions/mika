---
status: complete
priority: p2
issue_id: "321"
tags: [code-review, agent-native, maintenance]
dependencies: []
---

# set_config Tool Schema Enum Out of Sync with Config Key Allowlist

## Problem Statement

The `set_config` tool's JSON schema hardcodes `"enum": ["chat_id", "timezone"]` at
`crates/mika-agent/src/tools/set_config.rs:30`, but the shared allowlist in
`config_keys.rs` now includes `["chat_id", "timezone", "thinking_level"]`.

This means the Claude agent cannot discover or use `thinking_level` through the
`set_config` tool, even though the Rust-side validation would accept it. The JSON
schema `enum` is what the model uses to constrain its parameter generation.

This is a maintenance hazard: every new key added to `SETTABLE_CONFIG_KEYS` requires
a manual update to the hardcoded enum in the tool schema.

## Findings

- **Architecture reviewer:** "The JSON schema enum only lists `["chat_id", "timezone"]`. The agent tool schema and the allowlist are now out of sync."
- **Security reviewer:** "The tool's JSON schema enum does not include `thinking_level`. The schema and allowlist should be aligned."
- **Agent-native reviewer:** "The agent cannot set `thinking_level` via `set_config` despite the key being in the allowlist. This is the primary action-parity failure."
- **Simplicity reviewer:** Confirmed same finding.

## Proposed Solutions

### Option A: Generate enum from SETTABLE_CONFIG_KEYS (Recommended)

Replace the hardcoded enum with a dynamic reference:

```rust
"enum": serde_json::Value::Array(
    SETTABLE_CONFIG_KEYS.iter()
        .map(|k| serde_json::Value::String(k.to_string()))
        .collect()
)
```

- **Pros:** Single source of truth, never goes out of sync again
- **Cons:** Slightly more code in `definition()`
- **Effort:** Small
- **Risk:** Low

### Option B: Manually add "thinking_level" to enum

```rust
"enum": ["chat_id", "timezone", "thinking_level"]
```

- **Pros:** Minimal change
- **Cons:** Same maintenance hazard persists for future keys
- **Effort:** Trivial
- **Risk:** Low

## Recommended Action

Option A — generate from allowlist.

## Technical Details

- **File:** `crates/mika-agent/src/tools/set_config.rs` line 30
- **Also update:** `value` description (line 34) to include thinking_level examples
- **Related:** `crates/mika-agent/src/config_keys.rs` (SETTABLE_CONFIG_KEYS)

## Acceptance Criteria

- [ ] `set_config` tool schema enum matches `SETTABLE_CONFIG_KEYS`
- [ ] Adding a new key to `SETTABLE_CONFIG_KEYS` automatically appears in tool schema
- [ ] `value` description includes thinking_level example values
- [ ] Existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review of think persistence feature | Hardcoded enums drift from shared allowlists |

## Resources

- PR: uncommitted changes on main (think level persistence)
- File: `crates/mika-agent/src/tools/set_config.rs`
