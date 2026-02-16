---
status: complete
priority: p2
issue_id: 226
tags: [code-review, security, slash-commands]
dependencies: []
---

# Config Info Disclosure via /config Command

## Problem Statement

The `/config` command reads and displays the entire TOML config file contents, which may include sensitive values like API keys or internal tokens. Even though `Settings` has a redacted `Debug` impl, the raw file content is displayed as-is.

**Why it matters:** Users might share screenshots or logs containing `/config` output, inadvertently exposing secrets.

## Findings

**Source:** Security Sentinel review agent

**Location:** `crates/mika-cli/src/tui/commands/handlers.rs:227-253` (`handle_config`)

```rust
if let Ok(content) = tokio::fs::read_to_string(&config_path).await {
    let _ = writeln!(out, "\nLocal config ({}):\n{}", config_path.display(), content);
}
```

The raw file content is dumped without filtering sensitive keys.

## Proposed Solutions

### Solution A: Parse TOML and redact sensitive keys (Recommended)
- Parse config as TOML Value, redact keys matching patterns like `*key*`, `*token*`, `*secret*`
- Show redacted values as `***`
- **Pros:** Safe by default, still useful for debugging
- **Cons:** Must maintain list of sensitive key patterns
- **Effort:** Small
- **Risk:** Low

### Solution B: Show only non-sensitive config fields
- Display specific known-safe fields (model, home dir) rather than dumping the file
- **Pros:** Simplest, no risk of leaking anything
- **Cons:** Less useful for debugging config issues
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Solution B — the handler already shows model and home dir. Just remove the raw file dump.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/commands/handlers.rs`

## Acceptance Criteria

- [ ] `/config` does not display raw file contents that may contain secrets
- [ ] Key config values (model, home dir, schema version) still shown

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Security sentinel flagged info disclosure |

## Resources

- PR branch: `feat/slash-commands`
