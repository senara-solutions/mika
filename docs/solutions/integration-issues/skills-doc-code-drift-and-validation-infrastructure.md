---
title: Skills Documentation-Code Drift and Validation Infrastructure
date: 2026-03-11
module: skills system
tags:
  - documentation-drift
  - skill-validation
  - manifest-format
  - cli-tooling
  - silent-failure-prevention
  - legacy-detection
severity: medium
symptoms:
  - docs/skills.md describes [handler] and [options] sections that don't exist in SkillManifest
  - Agent following its own documentation creates broken skills
  - Broken skills silently skipped at startup with no user-facing warning
  - is_legacy_format() only detected type="builtin", missing exec and http
  - mika skills create scaffolded handler using grep instead of jq
  - Self-knowledge skill missing runtime-structure topic
related:
  - docs/solutions/integration-issues/custom-skill-silent-loading-failure.md
  - docs/solutions/integration-issues/shell-exec-jq-json-parsing.md
  - docs/adr/002-filesystem-skill-registry.md
  - docs/adr/006-git-based-skills-marketplace.md
---

# Skills Documentation-Code Drift and Validation Infrastructure

## Problem

The skills documentation (`docs/skills.md`) described a manifest format that did not match the actual `SkillManifest` struct in `crates/mika-agent/src/skills/manifest.rs`. The documented format used `[handler]` and `[options]` sections at the top level, but the code required a `[skill]` section wrapper with handler config moved to per-tool objects in `tools.json`.

When Mika's self-knowledge skill directed the agent to read this documentation via `get_documentation("skills")`, the agent created skills using the wrong format. These skills were silently skipped by `scan_skills_dir()` with only a `warn!` tracing log — no user-facing error.

### Root Cause

The documentation was written for an older format that no longer matched the code. Over time, the code evolved (adding `[skill]` section, moving handler config to `tools.json`), but the docs were never updated. The system had no mechanism to detect this drift.

### Impact

- Any custom skill authored by following the docs would fail silently
- `is_legacy_format()` only detected `type = "builtin"` in `[handler]`, not `exec` or `http`
- `mika skills create` scaffolded a handler script using `grep` for JSON parsing instead of `jq`
- No startup warning when skills were skipped
- Self-knowledge skill was missing the `runtime-structure` documentation topic

## Solution

Six coordinated fixes implemented in branch `fix/build-mika-skill-format`:

### 1. Documentation Rewrite

Rewrote `docs/skills.md` to match the actual code format:

**Wrong format (old docs):**
```toml
name = "my-skill"
description = "Does things"
[handler]
type = "exec"
command = "./handler.sh"
[options]
always_on = true
```

**Correct format (actual code):**

`skill.toml`:
```toml
[skill]
name = "my-skill"
description = "Does things"
always_on = true

[triggers]
keywords = ["do things"]
```

`tools.json`:
```json
[{
  "name": "do_thing",
  "description": "Performs the action",
  "input_schema": {"type": "object", "properties": {"input": {"type": "string"}}, "required": ["input"]},
  "handler": {"type": "exec", "command": "handlers/run.sh"}
}]
```

Source of truth: `SkillManifest`, `SkillToolDef`, and `ToolHandler` in `crates/mika-agent/src/skills/manifest.rs`.

### 2. `mika skills validate` CLI Command

Added a validation command that checks skill directories for format errors:

```
$ mika skills validate
  web-search/
    [OK] skill.toml valid — name=web-search, description=Search the web
    [OK] tools.json valid — 1 tool(s)
    [OK] tool 'web_search': handler command OK

  broken-skill/
    [FAIL] legacy format detected: has [handler] section.

  1/2 valid, 1 with errors, 0 with warnings.
```

Validation checks:
- `skill.toml` exists, readable, under 64KB
- Not legacy format (no `[handler]` section)
- Parses as `SkillManifest` (has `[skill]` section with name + description)
- `tools.json` valid JSON with `handler` field on each tool
- Exec handler commands exist and are executable
- Symlink containment (handler commands resolve within skill directory)
- Warnings for no-op skills (no tools, no system_prompt) and never-activating skills (no triggers, not always_on)

Exit code is non-zero when errors are found, matching `mika doctor` behavior.

### 3. Startup Skipped-Skills Warning

`SkillRegistry::from_dir()` now emits a warning when skills are skipped:

```rust
if skipped_count > 0 {
    tracing::warn!(
        count = skipped_count,
        "skipped invalid skill(s) at startup — run `mika skills validate` for details"
    );
}
```

This covers all call sites (CLI chat, CLI ask, server, team engine, delegate task) from a single location.

### 4. Broadened Legacy Format Detection

`is_legacy_format()` changed from detecting only `type = "builtin"` to any `type` field under `[handler]`:

```rust
// Before: only caught builtin
handler.get("type").and_then(|v| v.as_str()).is_some_and(|t| t == "builtin")

// After: catches builtin, exec, http — any legacy handler
handler.get("type").and_then(|v| v.as_str()).is_some()
```

### 5. Handler Template Fix

`mika skills create` now scaffolds handler scripts using `jq` instead of `grep`:

```sh
#!/bin/sh
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required" >&2; exit 1; }
INPUT=$(cat)
QUERY=$(printf '%s\n' "$INPUT" | jq -r '.query // empty')
echo "TODO: implement handler for query: $QUERY"
```

### 6. Self-Knowledge Skill Fixes

- Added `runtime-structure` to the `topic` enum in `tools.json`
- Updated `system_prompt.md` schema reference from v7 to v8

## Code Review Findings Addressed

| ID | Priority | Fix |
|----|----------|-----|
| 629 | P1 | UTF-8 panic: byte-based string slicing → `chars().take(60)` |
| 630 | P2 | Path traversal: added `validate_skill_name()` before path join |
| 632 | P2 | Moved skipped_count warning into `SkillRegistry::from_dir()` |
| 635 | P3 | Renamed `DiagnosticLevel::Error` → `Fail` to match `mika doctor` |
| 636 | P3 | Non-zero exit code on validation errors |
| 637 | P3 | Updated stale solution doc |
| 638 | P3 | Symlink containment check on exec command paths |
| 639 | P3 | Removed dead `#[cfg(not(unix))]` code block |

## Prevention

### Why This Happened

The documentation was treated as a static artifact separate from the code. When `SkillManifest` evolved (adding `[skill]` section, moving handler config to `tools.json`), the docs were not updated. There was no automated check connecting documentation examples to code.

### How to Prevent Recurrence

1. **Validate after authoring:** Always run `mika skills validate` after creating or modifying a skill
2. **Follow bundled examples:** Reference `crates/mika-agent/templates/skills/` for canonical format
3. **Check manifest.rs:** When changing skill format, update `docs/skills.md` and run `scripts/sync-agent-docs.sh`
4. **Startup warnings:** If the agent logs "skipped invalid skill(s)", run validate immediately
5. **Use `mika skills create`:** The scaffold generates correct format with jq-based handler

### Remaining Work

| Todo | Priority | Description |
|------|----------|-------------|
| 631 | P2 | Add unit tests for `validate_skill()` (~8 code paths untested) |
| 633 | P2 | Add agent-facing `validate_skill` tool (agent can create but not validate) |
| 634 | P3 | Extract shared parsing helper between `scan_skills_dir()` and `validate_skill()` |

## Key Files

| File | Role |
|------|------|
| `crates/mika-agent/src/skills/manifest.rs` | Source of truth — `SkillManifest`, `SkillToolDef`, `ToolHandler` |
| `crates/mika-agent/src/skills/index.rs` | `scan_skills_dir()`, `validate_skill()`, `is_legacy_format()` |
| `crates/mika-agent/src/skills/mod.rs` | `SkillRegistry::from_dir()` with skipped_count warning |
| `crates/mika-cli/src/commands/skills.rs` | `validate_skills()` CLI handler |
| `docs/skills.md` | Single source of truth for skills documentation |
| `crates/mika-agent/templates/skills/` | Bundled skill templates (canonical format examples) |
