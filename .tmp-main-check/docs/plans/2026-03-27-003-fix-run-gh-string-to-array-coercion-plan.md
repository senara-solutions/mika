---
title: "fix(github): run_gh string-to-array coercion and tool description gaps"
type: fix
status: completed
date: 2026-03-27
issue: 299
---

# fix(github): run_gh string-to-array coercion and tool description gaps

## Overview

Turn audit of mika-qa (trace `d96a2da8ea9e4d18ad68c53fc98beccc`, 2026-03-27) revealed 5 of 12 tool calls failed due to two issues in the GitHub CLI builtin handler: (1) `parse_command_array()` rejects string-encoded JSON arrays instead of coercing them, and (2) the `run_gh` tool description lacks `gh pr diff` constraints, causing the agent to attempt unsupported file filtering.

## Problem Statement

### 1. String command parameter rejected instead of coerced

`parse_command_array()` in `builtin_handlers.rs` (line ~281) detects when the LLM passes `command` as a JSON string (e.g., `"[\"pr\", \"view\", \"42\"]"`) but returns an error instead of attempting to parse it. This is a common LLM serialization mistake observed with Sonnet 4.6 when using complex `--jq` expressions.

**Current:** Returns `"The 'command' parameter must be a JSON array of strings, not a single string."`
**Expected:** Attempt `serde_json::from_str::<Vec<String>>()`. If valid, use it. If not, return current error.

### 2. Tool description missing `gh pr diff` constraints

The agent tried `gh pr diff 35 -- self-dev/system_prompt.md` (steps 3-4), which fails because `gh pr diff` accepts only the PR number and does not support `--` file path filtering. Neither the tool description nor system prompt warn about this.

## Proposed Solution

### Change 1: String-to-array coercion in `parse_command_array()`

In `crates/mika-agent/src/skills/builtin_handlers.rs`, replace the string rejection branch (lines 281-286) with a coercion attempt:

```rust
// Before (current)
if let Some(cmd) = input.get("command") {
    if cmd.is_string() {
        return Err(anyhow!("The 'command' parameter must be a JSON array of strings, not a single string."));
    }
    // ... array branch
}

// After (proposed)
if let Some(cmd) = input.get("command") {
    if let Some(s) = cmd.as_str() {
        // LLMs sometimes serialize arrays as JSON strings — attempt coercion
        match serde_json::from_str::<Vec<String>>(s) {
            Ok(parsed) => {
                tracing::debug!("coerced string command parameter to array");
                // Fall through to same validation as native array
                args = parsed;
            }
            Err(_) => {
                return Err(anyhow!(
                    "The 'command' parameter must be a JSON array of strings, not a single string."
                ));
            }
        }
    }
    // ... existing array branch (becomes else-if)
}
```

**Security invariant:** The coerced `Vec<String>` feeds into the same `args` variable, so all downstream validation (empty check, length check, allowlist, blocked flags) applies identically.

**Design decision:** Keep `tools.json` schema as `"type": "array"` — coercion is silent resilience, not a declared capability. Legitimizing string input would increase its frequency.

### Change 2: `gh pr diff` constraint in tool description and system prompt

**`tools.json`** — Add to the `command` property description:
> Note: `gh pr diff` accepts only the PR number — it does not support `--` file path filtering.

**`system_prompt.md`** — Add limitation note after the PR diff example:
```
- View PR diff: ["pr", "diff", "42"]
  - Note: `gh pr diff` does NOT support `--` file path filtering. To review specific files, fetch the full diff and search within it.
```

## Acceptance Criteria

- [x] `parse_command_array()` coerces valid JSON string arrays (e.g., `"[\"pr\", \"list\"]"`) into `Vec<String>` — `builtin_handlers.rs`
- [x] `parse_command_array()` still rejects plain strings (e.g., `"pr list --state open"`) — `builtin_handlers.rs`
- [x] `parse_command_array()` still rejects invalid JSON strings (e.g., `"[\"pr\", 42]"`) — `builtin_handlers.rs`
- [x] Coerced arrays pass through same validation pipeline (empty, length, allowlist, blocked flags) — `builtin_handlers.rs`
- [x] `tracing::debug!` emitted on successful coercion for observability — `builtin_handlers.rs`
- [x] `tools.json` description includes `gh pr diff` limitation note — `templates/skills/github/tools.json`
- [x] `system_prompt.md` includes `gh pr diff` limitation note — `templates/skills/github/system_prompt.md`
- [x] Existing tests pass (plain string rejection unchanged) — `builtin_handlers.rs` tests
- [x] New tests cover coercion success path — `builtin_handlers.rs` tests
- [x] New tests cover coercion failure paths (invalid JSON, mixed types) — `builtin_handlers.rs` tests
- [x] Both `run_gh` and `run_gws` paths tested (shared `parse_command_array()`) — `builtin_handlers.rs` tests
- [x] `cargo test` passes, `cargo clippy` clean

## Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Coercion logic in `parse_command_array()` (~line 281) |
| `crates/mika-agent/templates/skills/github/tools.json` | Add `gh pr diff` limitation to description |
| `crates/mika-agent/templates/skills/github/system_prompt.md` | Add `gh pr diff` limitation note |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (tests) | New coercion tests, verify existing tests |

## MVP

### `crates/mika-agent/src/skills/builtin_handlers.rs` — `parse_command_array()` coercion

```rust
fn parse_command_array(input: &serde_json::Value) -> Result<Vec<String>> {
    let args = if let Some(cmd) = input.get("command") {
        if let Some(s) = cmd.as_str() {
            // LLMs sometimes serialize arrays as JSON strings — attempt coercion
            match serde_json::from_str::<Vec<String>>(s) {
                Ok(parsed) => {
                    tracing::debug!("coerced string command parameter to array");
                    parsed
                }
                Err(_) => {
                    return Err(anyhow!(
                        "The 'command' parameter must be a JSON array of strings, not a single string."
                    ));
                }
            }
        } else if let Some(arr) = cmd.as_array() {
            // Normal array path — existing logic
            arr.iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| anyhow!("All elements in 'command' must be strings"))
                })
                .collect::<Result<Vec<String>>>()?
        } else {
            return Err(anyhow!("Missing or invalid 'command' parameter"));
        }
    } else {
        return Err(anyhow!("Missing or invalid 'command' parameter"));
    };

    if args.is_empty() {
        return Err(anyhow!("The 'command' array must not be empty"));
    }

    let total_len: usize = args.iter().map(|s| s.len()).sum();
    if total_len > 10_000 {
        return Err(anyhow!("Command too long ({total_len} chars, max 10000)"));
    }

    Ok(args)
}
```

### Test: coercion success

```rust
#[test]
fn test_parse_command_array_coerces_valid_json_string() {
    let input = serde_json::json!({
        "command": "[\"pr\", \"list\", \"--state\", \"open\"]"
    });
    let result = parse_command_array(&input).unwrap();
    assert_eq!(result, vec!["pr", "list", "--state", "open"]);
}
```

### Test: plain string still rejected

```rust
#[test]
fn test_parse_command_array_rejects_plain_string() {
    let input = serde_json::json!({
        "command": "pr list --state open"
    });
    let result = parse_command_array(&input);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("JSON array of strings"));
}
```

### Test: invalid JSON string rejected

```rust
#[test]
fn test_parse_command_array_rejects_mixed_type_json_string() {
    let input = serde_json::json!({
        "command": "[\"pr\", 42]"
    });
    let result = parse_command_array(&input);
    assert!(result.is_err());
}
```

## Sources

- GitHub Issue: #299
- Turn audit trace: `d96a2da8ea9e4d18ad68c53fc98beccc`
- Related learning: `docs/solutions/security-issues/cli-blocked-flag-equals-bypass.md` — `parse_command_array()` extraction history
- Related learning: `docs/solutions/integration-issues/github-skill-missing-label-documentation.md` — tool description improvement pattern
