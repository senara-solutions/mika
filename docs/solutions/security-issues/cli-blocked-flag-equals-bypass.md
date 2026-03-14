---
title: "Blocked flag validation bypass via --flag=value form in CLI skill handlers"
date: 2026-03-13
module: "skills/builtin_handlers"
problem_type: security-issues
severity: medium
tags:
  - input-validation
  - credential-smuggling
  - prompt-injection
  - run_gh
  - run_gws
  - builtin-handlers
  - flag-parsing
symptoms:
  - "Blocked flags (e.g., --token, --repo) could be bypassed using --flag=value syntax"
  - "validate_gh_input did not catch --repo=evil form"
  - "validate_gws_input did not catch --token=evil form"
  - "LLM prompt injection could smuggle credentials or override repository targeting"
resolved: true
---

# Blocked flag validation bypass via --flag=value form in CLI skill handlers

## Problem

The builtin CLI handlers (`run_gh`, `run_gws`) validate user-supplied command arrays against blocked flag lists to prevent credential smuggling and unauthorized configuration overrides. The validation used exact string equality:

```rust
// run_gh: repo smuggling check
args.iter().any(|s| s == "--repo" || s == "-R")

// run_gws: credential flag check
for flag in GWS_BLOCKED_FLAGS {
    if args.iter().any(|s| s == *flag) {
```

This only catches standalone flags like `["--token", "evil"]` (two separate array elements) but misses `["--token=evil"]` (equals-separated, single element). Most CLI tools built with clap (including `gh` and `gws`) accept both forms.

**Attack vector:** An LLM subjected to prompt injection could pass `["gmail", "messages", "list", "--token=attacker_token"]`, overriding the legitimate credential and potentially redirecting API calls through an attacker-controlled OAuth token.

## Root Cause

Exact string matching (`s == "--token"`) does not account for the `--flag=value` syntax where the flag name and value are a single string joined by `=`. This is a common CLI convention that clap, argparse, and most option parsers accept.

The pre-existing `run_gh` handler had the same issue with `--repo` (since the GitHub skill predated the Google Workspace skill), but the risk was lower since `--repo` controls repository targeting rather than credentials.

## Solution

### Fix 1: Add `starts_with` check for equals form

**validate_gws_input** (blocked flags list):
```rust
for flag in GWS_BLOCKED_FLAGS {
    if args.iter().any(|s| s == *flag || s.starts_with(&format!("{flag}="))) {
```

**validate_gh_input** (inline repo check):
```rust
if args.iter().any(|s| s == "--repo" || s == "-R" || s.starts_with("--repo=")) {
```

### Fix 2: Extract shared validation to prevent duplication

Extracted two shared helpers to eliminate duplicated code between `run_gh` and `run_gws`:

- **`parse_command_array(input)`** -- shared validation: string rejection, array parsing, empty check, 10K length limit. Each handler calls this, then applies its own allowlist and blocked-flag checks.
- **`spawn_and_collect(cmd, tool_name, install_hint)`** -- shared subprocess execution: spawn, bounded stdout/stderr reads, wait, exit-code formatting.

This ensures any future CLI handler (e.g., `run_aws`, `run_kubectl`) gets the same validation and execution behavior without copy-paste.

## Verification

Tests added:
- `test_validate_gws_input_token_equals_smuggling` -- rejects `["gmail", "messages", "list", "--token=evil"]`
- `test_validate_gws_input_credentials_file_equals_smuggling` -- rejects `["gmail", "+send", "--credentials-file=/etc/creds"]`
- `test_run_gh_repo_equals_smuggling` -- rejects `["pr", "list", "--repo=evil/repo"]`

All 44 builtin handler tests pass. Full `cargo test` suite passes.

## Prevention

When adding blocked-flag validation for CLI tool handlers:

1. **Always check both forms:** `s == flag` (standalone) AND `s.starts_with(&format!("{flag}="))` (equals-separated).
2. **Use the shared `parse_command_array` helper** for common validation steps.
3. **Add tests for both flag forms** in the test suite.
4. **Consider prefix matching** for flag families (e.g., `--token` also catches `--token-file` if applicable).

## Related Documentation

- [Env var leakage in exec handler child processes](../security-issues/env-var-leakage-exec-handler-child-processes.md) -- three-tier env isolation model
- [Shell-exec jq JSON parsing](../integration-issues/shell-exec-jq-json-parsing.md) -- input parsing security in handler scripts
- [Builtin skill tool name shadowing](../logic-errors/builtin-skill-tool-name-shadowing.md) -- dispatch chain security
- [Skills doc-code drift and validation infrastructure](../integration-issues/skills-doc-code-drift-and-validation-infrastructure.md) -- skill validation patterns
- GitHub issue: [#75](https://github.com/senara-solutions/mika/issues/75) -- Google Workspace skill feature request

## Affected Files

- `crates/mika-agent/src/skills/builtin_handlers.rs` -- validation functions and shared helpers
- `crates/mika-agent/templates/skills/google-workspace/` -- new skill templates
