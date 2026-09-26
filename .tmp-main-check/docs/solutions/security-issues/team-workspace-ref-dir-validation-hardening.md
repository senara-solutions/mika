---
title: "Team workspace security hardening: UUID validation, dotfile filtering, and fallback narrowing"
category: security-fix
date: 2026-03-15
tags:
  - path-traversal
  - input-validation
  - defense-in-depth
  - team-workspace
  - code-review
  - workspace-tools
affected_components:
  - crates/mika-cli/src/commands/ask.rs
  - crates/mika-cli/src/main.rs
  - crates/mika-agent/src/tools/read_workspace.rs
  - crates/mika-agent/src/tools/list_workspace.rs
  - crates/mika-agent/src/teams/engine.rs
severity: medium
root_cause_category: insufficient-input-validation
---

# Team Workspace Reference Directory Validation Hardening

## Problem

After implementing run-scoped team workspaces with a reference directory fallback (`--run-id`),
code review identified six security and correctness issues:

1. `--run-id` used directly in filesystem paths without UUID format validation
2. Engine-internal `.meta/` files leaked to agents via workspace tools
3. Reference workspace fallback triggered on ANY error, masking security rejections
4. Zero test coverage for the reference workspace fallback logic
5. `write_metadata_file()` bypassed `validate_and_resolve_path()` without guards
6. OTel/logging setup duplicated across CLI branches (maintenance risk)

None were actively exploitable at the time of discovery, but all represented defense-in-depth
gaps that could become vulnerabilities through future code changes.

## Root Cause

The reference workspace feature added a second directory to the workspace tool resolution
chain without fully applying the security patterns established for single-directory workspace
access. Specifically:

- **UUID validation**: The DB lookup acted as an indirect format check (all stored run_ids are
  UUIDs from `Uuid::new_v4()`), but this relied on an assumption about DB content rather than
  explicit validation. A future code path inserting a non-UUID run_id would create a path
  traversal vector.
- **Dotfile filtering**: `collect_files()` was designed for flat workspaces without internal
  subdirectories. The `.meta/` engine metadata directory was a new concept not accounted for.
- **Fallback semantics**: The `!output.is_error` check was a broad proxy for "file found" that
  treated all errors uniformly, including security rejections that should never trigger fallbacks.

## Solution

### Fix 1: UUID format validation at CLI boundary

Added `uuid::Uuid::parse_str()` validation in both CLI entry points before any filesystem or
DB use:

```rust
// crates/mika-cli/src/main.rs (chat --team path)
if let Some(ref_id) = run_id
    && uuid::Uuid::parse_str(ref_id).is_err()
{
    anyhow::bail!(
        "Invalid --run-id format. Expected a UUID (e.g., from a previous team run)."
    );
}

// crates/mika-cli/src/commands/ask.rs (ask --team path)
if let Some(ref_id) = run_id {
    if uuid::Uuid::parse_str(ref_id).is_err() {
        anyhow::bail!(
            "Invalid --run-id format. Expected a UUID (e.g., from a previous team run)."
        );
    }
    // ... proceed with DB validation
}
```

### Fix 2: Dotfile/dotdir filtering in workspace tools

**list_workspace.rs** — skip entries starting with `.` in `collect_files()`:

```rust
if let Some(name) = entry.file_name().to_str()
    && name.starts_with('.')
{
    continue;
}
```

**read_workspace.rs** — reject paths whose first component starts with `.`:

```rust
if path.split('/').next().map_or(false, |first| first.starts_with('.')) {
    return Ok(ToolOutput::error("Cannot access hidden files in workspace."));
}
```

### Fix 3: Narrow fallback to "not found" only

Changed the reference workspace fallback to only trigger on genuine "not found" errors:

```rust
Ok(not_found) => {
    let is_not_found = not_found.content.contains("not found")
        || not_found.content.contains("does not exist");
    if is_not_found {
        if let Some(ref ref_dir) = self.reference_dir {
            match self.read_from_dir(path, ref_dir).await {
                Ok(output) if !output.is_error => Ok(output),
                _ => Ok(not_found),
            }
        } else {
            Ok(not_found)
        }
    } else {
        Ok(not_found) // Security errors propagate without fallback
    }
}
```

### Fix 4: Test coverage for reference workspace

Added 6 tests covering: file in current only, file in reference only, current wins over
reference, path traversal blocked with reference_dir set, symlink error does not trigger
fallback, and list_workspace dual-section output.

### Fix 5: Metadata path safety

Added `debug_assert!` in `write_metadata_file()`:

```rust
fn write_metadata_file(&self, name: &str, content: &str) {
    debug_assert!(
        !name.contains('/') && !name.contains('\\') && !name.contains(".."),
        "metadata file name must be a simple filename: {name}"
    );
    // ...
}
```

### Fix 6: Logging helper extraction

Extracted `init_team_logging()` with `#[cfg(feature = "telemetry")]` variants, eliminating
15-line duplication between Chat and Ask branches in `main.rs`.

## Defense Layers Summary

| Layer | Threat | Mitigation |
|-------|--------|------------|
| CLI boundary | Malformed run-id | UUID format validation |
| Listing | Internal file exposure | Dotfile filtering in `collect_files` |
| Reading | Metadata access | First-component dot check |
| Error handling | Security bypass via fallback | Content-aware fallback gating |
| Internal API | Future path traversal | `debug_assert` on metadata names |

## Prevention

### Code review checklist for workspace tools

1. Does the tool construct filesystem paths from user/agent input? Validate format before
   `PathBuf::join()`.
2. Does the tool list directory contents? Filter dotfiles/dotdirs and engine-internal paths.
3. Does the tool have a fallback path? Match on specific error conditions (`"not found"`),
   never catch-all.
4. Does the tool cross workspace boundaries? Test all branches: primary success, fallback
   trigger, fallback suppression on security errors.
5. Does the tool write files? Route through `validate_and_resolve_path()` or assert filename
   safety.

### Patterns to watch for in future PRs

- `PathBuf::push(user_input)` without adjacent validation
- `match result { Err(_) => try_alternative() }` on filesystem operations
- New dotfiles/directories without listing filter updates
- `std::fs::write` with variable filename parameters

## Related Documentation

- [tilde-home-expansion-file-tools.md](../logic-errors/tilde-home-expansion-file-tools.md) — `validate_and_resolve_path` creation
- [tool-path-reporting-misbehavior.md](../logic-errors/tool-path-reporting-misbehavior.md) — workspace path resolution bugs
- [env-var-leakage-exec-handler-child-processes.md](./env-var-leakage-exec-handler-child-processes.md) — defense-in-depth pattern
- [cli-flag-subcommand-scoping.md](../architecture-patterns/cli-flag-subcommand-scoping.md) — `--team`/`--agent` flag routing
- [team-conversation-continuity.md](../architecture/team-conversation-continuity.md) — previous run context injection
- [team-engine-code-review-findings-batch.md](../logic-errors/team-engine-code-review-findings-batch.md) — prior team engine review
