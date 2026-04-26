---
title: "CLI metadata trailer via --verbose flag"
module: mika-cli
date: 2026-04-26
problem_type: best_practice
component: tooling
severity: low
tags:
  - cli
  - verbose
  - metadata
  - session-id
  - cross-command
  - trailer
applies_when:
  - Adding machine-readable metadata to CLI text output for downstream command consumption
  - Downstream slash commands or scripts need session context from mika ask
  - JSON format is not usable because the consumer does text-oriented processing
---

# CLI metadata trailer via --verbose flag

## Context

Downstream slash commands (`/mika-groom-ticket`, `/mika-ask-arch`) need to capture the `session_id` from `mika ask` output to reference prior conversation context in multi-pass pipelines. Using `--format json` would require JSON parsing that breaks text-oriented pipeline processing. A lightweight opt-in metadata trailer on stdout was needed.

## Guidance

Add metadata as key-value trailer lines on stdout, gated behind `--verbose`, with a blank-line separator between response body and trailer:

```rust
// In the OutputFormat::Text branch, after printing response and pending callbacks notice:
if verbose {
    // Blank line separates response body from metadata trailer,
    // making `grep ^session_id:` reliable even when LLM prose
    // contains "session_id:" text.
    println!();
    println!("session_id: {session_id}");
}
```

Key design decisions:

- **stdout, not stderr** -- downstream consumers capture stdout via command substitution (`$(mika ask --verbose ...)`). The flag is opt-in so existing pipe workflows are unaffected.
- **Blank-line separator** -- prevents false positives when LLM prose coincidentally contains `session_id:` text. Parsers can reliably `grep ^session_id:` on lines after the last blank line.
- **Key-name matching** -- downstream parsers match by `session_id:` prefix, not by line position or section markers. This is robust against future trailer additions.
- **`conflicts_with = "team"`** -- consistent with other solo-mode flags (`--model`, `--task-id`). Team mode has its own metadata path via `team_run` in JSON.
- **Text mode only** -- JSON mode already has a structured envelope (`AskJsonResponse`). Adding `session_id` to JSON is a separate concern.

## Why This Matters

Cross-command integration (e.g., `/mika-groom-ticket` two-pass pipeline) requires session context without mandating JSON parsing. Without the blank-line separator, LLM responses containing `session_id:` text create false positives for grep-based parsers. Without `--verbose` gating, every `mika ask` consumer would need to handle trailer lines even when they don't need metadata.

## When to Apply

- When adding new metadata fields to the `mika ask` verbose trailer (e.g., `trace_id`, `model`)
- When building downstream commands that consume `mika ask` output and need session context
- When deciding between stdout trailer vs stderr vs JSON for machine-readable CLI metadata

## Examples

Downstream consumer pattern (shell):
```bash
output=$(mika ask --verbose "analyze this code")
session_id=$(echo "$output" | grep '^session_id: ' | head -1 | cut -d' ' -f2)
# Use session_id for follow-up call
mika ask --session-id "$session_id" "now review the analysis"
```

Adding a new trailer field (follow the established pattern):
```rust
if verbose {
    println!();
    println!("session_id: {session_id}");
    // Future additions go here, one per line, same key: value format
    // println!("trace_id: {trace_id}");
}
```
