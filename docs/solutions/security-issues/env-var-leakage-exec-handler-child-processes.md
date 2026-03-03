---
title: Marketplace Skills Could Access Parent Process MIKA_* Environment Variables
date: 2026-03-03
category: security-issues
tags: [env-var-leakage, defense-in-depth, untrusted-code, child-process]
component: crates/mika-agent/src/skills/executor.rs
symptom: Marketplace skill exec handlers inherited sensitive API keys and tokens from parent process environment
severity: high
applies_to: [rust, child-process, environment-variables, marketplace-skills]
---

# MIKA_* Environment Variable Leakage Through Exec Handler Child Processes

## Problem Statement

When the skills marketplace feature (PR #56) was added, marketplace-installed skills with exec handlers could access **all** `MIKA_*` environment variables inherited from the parent process. This included:

- `MIKA_ANTHROPIC_API_KEY` (API key or OAuth token)
- `MIKA_INTERNAL_TOKEN` (shared secret for gateway auth)
- `MIKA_OPENAI_API_KEY` (embedding API key)
- `MIKA_BRAVE_API_KEY` (web search API key)

The existing defense-in-depth only covered bundled handler scripts (shell-exec, github) which manually `unset` specific vars in their shell scripts. Marketplace skills from untrusted third parties had **no such protection**.

**Severity:** High -- a malicious marketplace skill could trivially exfiltrate API keys by reading `$MIKA_ANTHROPIC_API_KEY` from its inherited environment.

## Root Cause

Mika's executor (`crates/mika-agent/src/skills/executor.rs`) spawns child processes for skill tool calls using `tokio::process::Command`. By default, `Command::new()` inherits the parent's full environment. Bundled scripts had manual `unset` calls for specific vars, but this was a script-level defense-in-depth layer on top of nothing -- there was no base protection in the executor itself.

The gap became critical when the marketplace feature enabled installation of untrusted third-party exec handlers that would run with the same environment as Mika.

## Solution

Added `MIKA_*` env var scrubbing directly in the executor's `execute_exec` function, before spawning any child process. This applies uniformly to **all** exec-handler skills (built-in, marketplace, and custom).

### Code Changes

**`crates/mika-agent/src/skills/executor.rs` -- `execute_exec` function:**

```rust
let mut cmd = Command::new(&handler.command);
// ... set args, MIKA_TOOL_INPUT env var, etc.

// Scrub MIKA_* env vars from child process (defense-in-depth)
for (key, _) in std::env::vars() {
    if key.starts_with("MIKA_") {
        cmd.env_remove(&key);
    }
}
```

**`crates/mika-agent/src/skills/git.rs` -- `git_command` function:**

```rust
fn git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    // Scrub MIKA_* env vars from the child process (defense-in-depth)
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
    cmd
}
```

### Design Decisions

1. **Executor-level scrubbing:** Applied at the process spawning layer, not at individual script level, ensuring all exec handlers are protected uniformly.
2. **Prefix-based pattern matching:** Uses `key.starts_with("MIKA_")` rather than listing specific variables, making the solution resistant to future additions of new sensitive environment variables.
3. **Defense-in-depth retention:** Existing script-level `unset` calls in bundled skill handlers are preserved as an additional safety layer.
4. **Consistent coverage:** Applied to both general executor commands and git-specific subprocess handling.

## Environment Security Tiers in Mika

The codebase now has three distinct environment isolation patterns, ordered by strictness:

| Pattern | Trust Level | Strength | Where Used |
|---------|-------------|----------|------------|
| `env_clear()` + allowlist | Untrusted | Highest | MCP child processes |
| `env_remove(MIKA_*)` | Semi-trusted | High | Exec handler executor, git subprocesses |
| Handler-side `unset` | Any | Medium | Bundled shell scripts (defense-in-depth) |

### MCP pattern (strictest) -- `crates/mika-agent/src/mcp/mod.rs`:

```rust
cmd.env_clear();
for key in &["PATH", "HOME", "USER", "LANG", "TERM", "TMPDIR", "XDG_RUNTIME_DIR"] {
    if let Ok(val) = std::env::var(key) {
        cmd.env(key, val);
    }
}
// Block MIKA_* overrides in config.env
```

### Exec handler pattern (medium) -- `crates/mika-agent/src/skills/executor.rs`:

```rust
for (key, _) in std::env::vars() {
    if key.starts_with("MIKA_") {
        cmd.env_remove(&key);
    }
}
```

### Shell script pattern (defense-in-depth) -- bundled handler scripts:

```bash
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY
```

## Prevention Checklist

When adding new child process spawning to Mika:

- [ ] Identify the trust level of the spawned process (untrusted/semi-trusted/trusted)
- [ ] Apply the appropriate env isolation pattern from the tiers above
- [ ] For untrusted sources, default to `env_clear()` + allowlist (MCP pattern)
- [ ] For semi-trusted sources, use `env_remove(MIKA_*)` prefix scrubbing
- [ ] Document the security model in comments at the spawn point
- [ ] When adding new `MIKA_*` environment variables, audit all spawn points -- prefix-based scrubbing handles this automatically, but `unset` lists in scripts need updating
- [ ] Add a test that verifies `MIKA_*` vars are not present in the child environment

### Test Pattern

```rust
#[test]
fn env_vars_not_leaked_to_child_process() {
    unsafe { std::env::set_var("MIKA_TEST_SECRET", "sensitive_value"); }

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg("env | grep MIKA || true");

    // Apply scrubbing (as production code does)
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }

    let output = cmd.output().expect("failed to spawn");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("MIKA_"), "Leaked MIKA_* vars: {stdout}");

    unsafe { std::env::remove_var("MIKA_TEST_SECRET"); }
}
```

## Cross-References

### Related Documentation
- `docs/adr/006-git-based-skills-marketplace.md` -- Marketplace ADR covering env scrubbing in git subprocesses
- `docs/adr/002-filesystem-skill-registry.md` -- Exec handler security considerations
- `docs/solutions/integration-issues/mcp-client-integration-rmcp.md` -- MCP env sandboxing with `env_clear()` + allowlist
- `docs/skills.md` (Security Considerations section) -- Updated to document executor-level scrubbing

### Related Code
- `crates/mika-agent/src/skills/executor.rs:execute_exec` -- Primary fix location
- `crates/mika-agent/src/skills/git.rs:git_command` -- Git subprocess scrubbing
- `crates/mika-agent/src/mcp/mod.rs` -- Reference `env_clear()` + allowlist pattern
- `crates/mika-agent/templates/skills/github/handlers/run.sh` -- Script-level `unset`

### Related Issues
- PR #56: feat: add git-based skills marketplace (amplified the risk)
- `todos/396-complete-p1-exec-handler-mika-env-leak.md` -- Original finding
- `todos/211-complete-p1-exec-handler-env-leakage.md` -- Earlier finding from prior review
