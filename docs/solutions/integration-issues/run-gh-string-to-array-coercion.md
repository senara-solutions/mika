---
title: "run_gh string-to-array coercion for LLM serialization mistakes"
category: integration-issues
date: 2026-03-27
tags: [skills, builtin-handlers, parse-command-array, llm-behavior, github-skill]
issue: 299
---

# run_gh string-to-array coercion for LLM serialization mistakes

## Problem

LLMs (observed with Sonnet 4.6) sometimes serialize the `command` array parameter as a JSON string instead of a native JSON array. For example, instead of `["pr", "view", "42"]`, the LLM sends `"[\"pr\", \"view\", \"42\"]"`. The shared `parse_command_array()` function in `builtin_handlers.rs` rejected all string inputs outright, causing tool call failures. In one traced session (mika-qa, trace `d96a2da8ea9e4d18ad68c53fc98beccc`), this caused 5 of 12 tool calls to fail (42% failure rate).

A secondary issue: the agent attempted `gh pr diff 35 -- self-dev/system_prompt.md`, which fails because `gh pr diff` does not support `--` file path filtering — but neither the tool description nor system prompt documented this limitation.

## Root Cause

1. `parse_command_array()` had a hard rejection for any string value in the `command` field — no attempt to parse it as a potential JSON array.
2. Complex `--jq` expressions in tool calls seem to trigger the LLM to serialize the entire command array as a single string.
3. Missing tool description guidance for `gh pr diff` limitations.

## Solution

### String-to-array coercion in `parse_command_array()`

Replace the string rejection branch with a coercion attempt using `serde_json::from_str::<Vec<String>>()`:

```rust
// In parse_command_array()
Some(cmd) if cmd.is_string() => {
    // LLMs sometimes serialize arrays as JSON strings — attempt coercion
    match serde_json::from_str::<Vec<String>>(cmd.as_str().unwrap()) {
        Ok(parsed) => {
            tracing::debug!("coerced string command parameter to array");
            parsed
        }
        Err(_) => {
            return Err(ToolOutput::error(
                "The 'command' parameter must be a JSON array of strings, not a single string."
                    .to_string(),
            ));
        }
    }
}
```

**Security invariant:** The coerced `Vec<String>` feeds into the same `args` variable as native arrays, so all downstream validation (empty check, length limit, allowlist, blocked flags) applies identically.

**Design decision:** The `tools.json` schema stays as `"type": "array"` — coercion is silent resilience, not a declared capability. Changing the schema to accept strings would legitimize string input and increase its frequency.

### Tool description update

Added `gh pr diff` limitation notes to both `tools.json` (command property description) and `system_prompt.md` (inline after the diff example).

## Key Files

- `crates/mika-agent/src/skills/builtin_handlers.rs` — `parse_command_array()` (shared by `validate_gh_input`, `validate_gws_input`, and `validate_git_ops_input`)
- `crates/mika-agent/templates/skills/github/tools.json` — tool description
- `crates/mika-agent/templates/skills/github/system_prompt.md` — usage examples

## Prevention

- **Defense-in-depth for LLM input:** When a tool parameter has a strict type (`array`), always consider that LLMs may serialize it as a string. Attempt coercion before rejection. This pattern applies to any builtin handler that expects structured input.
- **Document CLI tool limitations in both `tools.json` and `system_prompt.md`:** LLMs need explicit negative guidance ("does NOT support X") to avoid wasting tool calls on unsupported operations. Positive examples alone are insufficient.
- **Shared validation benefits all handlers:** Because `parse_command_array()` is shared, the coercion fix automatically benefits `run_gh`, `run_gws`, and `run_git` — no per-handler changes needed.

## Related

- [cli-blocked-flag-equals-bypass](../security-issues/cli-blocked-flag-equals-bypass.md) — `parse_command_array()` extraction history
- [github-skill-missing-label-documentation](./github-skill-missing-label-documentation.md) — similar pattern of improving tool description to prevent LLM mistakes
