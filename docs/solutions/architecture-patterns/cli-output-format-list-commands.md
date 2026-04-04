---
title: Add --format text|json to CLI list commands using OutputFormat enum
module: mika-cli (cli.rs, commands/agents.rs)
severity: low
tags:
  - cli
  - clap
  - json
  - output-format
  - scripting
date: 2026-04-04
related_issues:
  - "#370"
---

# Add --format text|json to CLI list commands using OutputFormat enum

## Problem

CLI list commands (e.g., `mika agents list`) output human-readable text with headers, indentation, and markers. This is awkward to parse in scripts — the root Makefile used `tail -n +2 | awk '{print $1}'` to extract agent names, which is fragile and breaks on formatting changes.

## Root Cause

No machine-parseable output format was available. The `OutputFormat` enum (`Text`/`Json`) already existed for `mika ask` and `mika teams log`, but hadn't been extended to other list commands.

## Solution

### Pattern: Add `--format` to a list subcommand

1. **Convert unit variant to struct variant** in the `Commands` enum:
   ```rust
   // Before: unit variant
   List,
   // After: struct variant with format field
   List {
       #[arg(long, value_enum, default_value = "text")]
       format: OutputFormat,
   },
   ```

2. **Update dispatch** to destructure and pass format:
   ```rust
   AgentsCommand::List { format } => list(&global_home, &format),
   ```

3. **Branch handler on format** — JSON emits structured array, text preserves existing output:
   ```rust
   match format {
       OutputFormat::Json => {
           let entries: Vec<serde_json::Value> = items.iter()
               .map(|item| serde_json::json!({"name": item, ...}))
               .collect();
           println!("{}", serde_json::to_string_pretty(&entries)?);
       }
       OutputFormat::Text => { /* existing human-readable output */ }
   }
   ```

4. **Empty list**: JSON emits `[]` (no hint text), text preserves the helpful message.

### Key conventions

- Reuse the existing `OutputFormat` enum — never create a new one
- `#[arg(long, value_enum, default_value = "text")]` annotation (matches `ask` and `teams log`)
- JSON output uses `serde_json::to_string_pretty` for readability
- Scope `--format` to the specific subcommand only (see `cli-flag-subcommand-scoping.md`)

## Prevention

When adding new list-style CLI commands, include `--format` from the start. The pattern is lightweight (one field, one match branch) and prevents future parsing hacks in shell scripts.
