---
status: complete
priority: p3
issue_id: 613
tags: [code-review, agent-native, ux, ci]
dependencies: []
---

# No non-interactive path for mika setup (CI/automation gap)

## Problem Statement

The `mika setup` TTY guard (setup.rs:33-39) blocks non-interactive first-time setup. The bail message suggests pre-setting env vars, but this only works for the current process — it does not persist secrets to `~/.mika/.env` for subsequent runs. There is no `mika setup --api-key <key>` flag for CI/CD pipelines, Docker entrypoint scripts, or agent-driven orchestration.

This also creates an agent-native parity gap: the Mika agent cannot programmatically bootstrap a new instance.

## Findings

- **Source:** agent-native-reviewer (primary), code-simplicity-reviewer
- **Location:** `crates/mika-cli/src/commands/setup.rs:33-39`
- **Impact:** Blocks CI/CD automation; agent-native score is 3/12 for configuration capabilities
- **Note:** The dotenv `set_env_var()` primitive already exists — only the CLI wiring is missing

## Proposed Solutions

### Option A: Add --api-key flag to mika setup (Recommended)
```rust
Setup {
    #[arg(long, value_enum, default_value = "cli")]
    mode: SetupMode,
    /// Anthropic API key (non-interactive)
    #[arg(long)]
    api_key: Option<String>,
}
```
When `--api-key` is provided, skip the interactive prompt and call `set_env_var` directly.
- Effort: Small
- Risk: Low
- Pro: Unblocks CI/CD; uses existing primitives

### Option B: Document manual .env creation as the non-interactive path
Add docs showing: `echo "MIKA_LLM_API_KEY=sk-..." > ~/.mika/.env && chmod 600 ~/.mika/.env`
- Effort: Small
- Risk: Low
- Con: Fragile; users must remember all required vars

### Option C: Full non-interactive mode with all flags
Add flags for all setup values (api-key, brave-key, routing-url, etc.)
- Effort: Medium
- Risk: Low
- Pro: Complete automation story
- Con: Many flags; may be YAGNI until CI/CD is actually needed

## Acceptance Criteria

- [ ] At minimum, `mika setup --api-key <key>` works non-interactively
- [ ] First-time setup succeeds in a non-TTY environment with the flag
- [ ] Documentation updated with CI/CD setup instructions
