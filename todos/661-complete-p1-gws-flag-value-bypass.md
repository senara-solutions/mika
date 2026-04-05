---
status: complete
priority: p1
issue_id: 661
tags: [code-review, security, skill]
---

# Blocked flag bypass via `--flag=value` syntax in run_gws (and run_gh)

## Problem Statement

The blocked flag check in `validate_gws_input` uses exact string equality (`s == *flag`), which only catches `--token` as a standalone argument. It does NOT catch `--token=evil_value` (single argument with `=`). If the `gws` CLI accepts `--token=<value>`, an LLM could bypass the flag check via prompt injection.

The same weakness exists in `run_gh`'s `--repo`/`-R` check, though the risk is lower there.

## Findings

- **File**: `crates/mika-agent/src/skills/builtin_handlers.rs`, line 474-476
- **Agents**: Security sentinel, Architecture strategist, Pattern recognition specialist
- **Evidence**: `args.iter().any(|s| s == *flag)` only matches exact strings

## Proposed Solutions

### Option A: starts_with check (Recommended)
Update both `validate_gws_input` and `validate_gh_input` to also match `--flag=value` forms:

```rust
// In validate_gws_input:
if args.iter().any(|s| s == *flag || s.starts_with(&format!("{flag}="))) {

// In validate_gh_input (for --repo):
if args.iter().any(|s| s == "--repo" || s == "-R" || s.starts_with("--repo=")) {
```

Add test cases for `["gmail", "messages", "list", "--token=evil"]` and `["pr", "list", "--repo=evil/repo"]`.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] `validate_gws_input` catches `--token=value`, `--credentials-file=value`, `--config=value`, `--config-dir=value`
- [ ] `validate_gh_input` catches `--repo=value`
- [ ] Tests added for `=` form bypass attempts
