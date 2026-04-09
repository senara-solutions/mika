---
title: "is_legacy_format() false positive on valid skill.toml with [skill] + [handler]"
category: integration-issues
date: 2026-04-09
tags: [skills, scanning, legacy-detection, false-positive]
issue: "#507"
module: mika-agent/skills
severity: medium
---

# is_legacy_format() false positive on valid skill.toml with [skill] + [handler]

## Problem

Valid new-format `skill.toml` files containing both `[skill]` and `[handler]` sections were silently skipped at startup with:

```
⚠ 1 skill(s) skipped at startup:
  • qa-review: legacy format (has [handler] section)
```

This blocked any marketplace or custom skill that uses exec/http handlers alongside the new `[skill]` metadata section.

## Root Cause

`is_legacy_format()` in `crates/mika-agent/src/skills/index.rs` checked only for `[handler]` with a `type` field, without checking whether `[skill]` was also present. The function was broadened in an earlier fix (from detecting only `type="builtin"` to any `type` value) but the broadening went too far — it caught valid new-format skills that legitimately include `[handler]` sections.

The key discriminator between legacy and new format is the presence of `[skill]` — legacy format has bare `name`/`description` at root level without a `[skill]` wrapper.

## Solution

Added a `[skill]` presence check before the `[handler]` check:

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

Both callers (`scan_skills_dir()` for startup and `validate_skill()` for CLI) benefit automatically.

## Prevention

- **Test the discriminator, not just the positive case.** The original tests covered "legacy = true" and "new format = false" but never tested the intersection case (both `[skill]` and `[handler]`). Adding edge-case tests for overlapping conditions prevents false-positive regressions.
- **When broadening a detection heuristic, add a negative guard.** The earlier fix broadened `[handler]` type detection from `"builtin"` to any value — the right move — but should have simultaneously added the `[skill]` exclusion. When widening a filter, always consider what new inputs now pass through that shouldn't.

## Related

- [Custom skill silent loading failure](custom-skill-silent-loading-failure.md) — original discovery of the `is_legacy_format` check
- [Skills doc-code drift and validation infrastructure](skills-doc-code-drift-and-validation-infrastructure.md) — previous broadening fix that introduced this regression
- GitHub issue: #507
