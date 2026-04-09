---
title: "fix: is_legacy_format() rejects valid skill.toml with both [skill] and [handler]"
type: fix
status: completed
date: 2026-04-09
issue: "#507"
---

# fix: is_legacy_format() rejects valid skill.toml with both [skill] and [handler]

## Overview

`is_legacy_format()` in `crates/mika-agent/src/skills/index.rs:1171` incorrectly rejects valid new-format `skill.toml` files that contain both `[skill]` and `[handler]` sections. The function checks for `[handler]` with a `type` field but never checks whether `[skill]` is present. Any marketplace or custom skill with exec/http handlers is silently skipped at startup.

## Root Cause

The legacy skill format is: bare `name`/`description` at root level (no `[skill]` wrapper) + `[handler]` with a `type` field. The new format wraps metadata under `[skill]` and CAN legitimately include a `[handler]` section for exec/http tool definitions.

The current function only checks for `[handler]` presence, missing the key discriminator: whether `[skill]` exists.

```rust
// crates/mika-agent/src/skills/index.rs:1171-1184
fn is_legacy_format(content: &str) -> bool {
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if let Some(handler) = table.get("handler").and_then(|v| v.as_table())
        && handler.get("type").and_then(|v| v.as_str()).is_some()
    {
        return true;  // BUG: doesn't check for [skill] absence
    }
    false
}
```

## Fix

Add a `[skill]` presence check before the `[handler]` check. If `[skill]` exists, the file is new-format regardless of other sections.

### `crates/mika-agent/src/skills/index.rs` — `is_legacy_format()`

```rust
fn is_legacy_format(content: &str) -> bool {
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    // New format has [skill] section — never legacy
    if table.contains_key("skill") {
        return false;
    }
    // Legacy: [handler] with type but no [skill]
    if let Some(handler) = table.get("handler").and_then(|v| v.as_table())
        && handler.get("type").and_then(|v| v.as_str()).is_some()
    {
        return true;
    }
    false
}
```

### `crates/mika-agent/src/skills/index.rs` — stale comment at `scan_skills_dir()` call site (~line 389)

Update the comment from `"builtin"` reference to reflect detection of any handler type.

## Test Cases

### Unit tests — `test_is_legacy_format()`

Add to existing test function in `crates/mika-agent/src/skills/index.rs`:

| # | Scenario | `[skill]` | `[handler]` | `handler.type` | Expected |
|---|----------|-----------|-------------|-----------------|----------|
| 1 | Legacy builtin | No | Yes | `"builtin"` | `true` |
| 2 | Legacy exec | No | Yes | `"exec"` | `true` |
| 3 | Legacy http | No | Yes | `"http"` | `true` |
| 4 | New format, no handler | Yes | No | N/A | `false` |
| 5 | Invalid TOML | N/A | N/A | N/A | `false` |
| **6** | **Both [skill] + [handler] with type** | **Yes** | **Yes** | **`"exec"`** | **`false`** |
| **7** | **Empty [skill] + [handler] with type** | **Yes (empty)** | **Yes** | **`"exec"`** | **`false`** |

Cases 1-5 already exist. Cases 6-7 are new (the bug scenario and its variant).

## Acceptance Criteria

- [x] `is_legacy_format()` returns `false` when `[skill]` section is present, regardless of `[handler]`
- [x] Existing legacy detection (no `[skill]`, has `[handler]` with type) still returns `true`
- [x] Stale comment at `scan_skills_dir()` call site updated
- [x] Unit tests added for both `[skill]` + `[handler]` cases (rows 6 and 7)
- [x] `cargo test -p mika-agent` passes
- [x] `cargo clippy` clean

## Impact

- **Two callers affected:** `scan_skills_dir()` (startup) and `validate_skill()` (CLI). Both benefit from the fix automatically.
- **No behavioral change** for currently-passing test cases — the fix is additive (early-return `false` before existing `true` path).
- **Unblocks** marketplace skills with exec/http handlers (e.g., qa-review with qa_pr_view tool).

## Sources

- GitHub issue: #507
- Prior fix history: `docs/solutions/integration-issues/skills-doc-code-drift-and-validation-infrastructure.md`
- Original discovery: `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md`
