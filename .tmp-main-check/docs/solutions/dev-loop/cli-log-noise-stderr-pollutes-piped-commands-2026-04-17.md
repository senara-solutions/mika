---
title: "CLI INFO log on stderr pollutes terminal during piped commands"
date: 2026-04-17
category: dev-loop
module: logging
problem_type: developer_experience
component: tooling
severity: low
applies_when:
  - "Running CLI subcommands with pipe/grep (e.g., mika skills list | grep qa)"
  - "log_level = info in config.toml (needed for server/agent observability)"
tags: [cli, logging, stderr, tracing, developer-experience]
---

# CLI INFO log on stderr pollutes terminal during piped commands

## Context

Running `mika --agent mika-dev skills list | grep qa | grep -v disabled` displays a noisy `INFO skills loaded count=27 names=[...]` line on the terminal alongside the filtered output. The log goes to stderr (correctly — `logging.rs` uses `.with_writer(std::io::stderr)` for the `PrettyAndFile` output mode), so it bypasses the pipe. But it still appears interleaved with stdout in the terminal, making it look like grep let it through.

This only happens when `log_level = "info"` is set in config.toml. The CLI default is `"warn"` (`main.rs:307`), but users set `"info"` for server/agent observability and it applies globally to all CLI subcommands too.

## Guidance

The `tracing::info!("skills loaded", ...)` at `crates/mika-agent/src/skills/mod.rs:43` should be demoted to `tracing::debug!`. This log is useful when actively debugging skill loading but adds no value to every CLI invocation. The `WARN` log for skipped/invalid skills (line 32) stays at WARN — failures are still visible.

```rust
// Before (mod.rs:43)
tracing::info!(count = loaded_names.len(), skipped = result.skipped.len(), names = ?loaded_names, "skills loaded");

// After
tracing::debug!(count = loaded_names.len(), skipped = result.skipped.len(), names = ?loaded_names, "skills loaded");
```

## Why This Matters

Short-lived CLI subcommands (`skills list`, `status`, `config list`) are used for scripting and quick lookups. Startup INFO banners on stderr break the clean-output expectation. Users shouldn't need `2>/dev/null` or `MIKA_LOG_LEVEL=warn` to get clean output from a read-only command.

## When to Apply

- Any `tracing::info!` that fires during CLI startup (before the actual command logic) should be evaluated: is this useful for every invocation, or only for debugging? If the latter, use `debug!`.
- Server and agent startup logs at INFO are fine — those are long-running processes where startup visibility matters.

## Examples

**Workaround until fixed:**
```bash
# Suppress stderr
mika --agent mika-dev skills list 2>/dev/null | grep qa | grep -v disabled

# Or override log level for this invocation
MIKA_LOG_LEVEL=warn mika --agent mika-dev skills list | grep qa
```

## Related

- mika#619 — fix: demote 'skills loaded' log from INFO to DEBUG
