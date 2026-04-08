---
title: "CI gate tool: structural backstop for PR merges"
category: architecture-patterns
date: 2026-04-08
tags: [tool, github, ci, merge, subprocess, safety-gate]
related_issues: ["#490", "#485"]
components: [tools, builtin-handlers]
severity: high
---

# CI Gate Tool: Structural Backstop for PR Merges

## Problem

On 2026-04-08, PR mika#485 merged with a required CI check (`Pipeline Artifacts`) in FAILURE state. The agent called `run_gh pr merge` without first inspecting the `statusCheckRollup`. The merge flow had no check — it proceeded whether CI was green, red, or mid-flight.

Prompt-only guardrails (e.g., "check CI before merging") were insufficient because weaker models rationalize them away under pressure (see `feedback_prompt_enforcement_fragile` memory).

## Root Cause

The `run_gh` builtin handler is a generic `gh` CLI wrapper — it executes any valid `gh` command without domain-specific validation. There was no structural mechanism to enforce CI checks before merge.

## Solution

Added a new `pr_merge_with_gate` builtin tool (`crates/mika-agent/src/tools/pr_merge_with_gate.rs`) that makes the CI gate impossible to skip:

1. **Fetch required checks:** `gh pr checks <number> --repo <repo> --required --json name,state,bucket,link`
2. **Classify via decision matrix:**
   - Any `fail`/`cancel` bucket → `action: "blocked"` (never merges)
   - Any `pending` (no failures) → `gh pr merge --auto`, returns `action: "auto_merge_enabled"`
   - All `pass`/`skipping` → `gh pr merge`, returns `action: "merged"`
   - Empty → treat as all-pass
3. **Return structured JSON** so the agent can act programmatically

Key design decisions:
- **Tool trait implementor** (not builtin handler): Cannot be disabled per-agent via `skill_overrides`, making the gate truly structural
- **Registered in `default_tools()`**: All agents including delegates get the gate — intentional, since delegates spawned via claude-pilot need it most
- **Atomic operation**: Check + merge in one call, per the "merge two-step LLM tool contracts" learning
- **`run_gh` remains available** as an emergency escape hatch — the self-dev prompt (separate PR) directs agents to prefer `pr_merge_with_gate`

## Prevention

- **Structural over prompt:** When a safety property must hold regardless of LLM model quality, encode it in a tool that refuses invalid states rather than relying on prompt instructions.
- **Decision matrix as pure function:** `classify_checks()` is a testable pure function with 7 unit tests covering all matrix branches. New check states can be added to the matrix without changing the tool's control flow.
- **Subprocess pattern:** Follows `run_gh` pattern exactly: `scrub_mika_env_vars()` → `GH_TOKEN` re-injection → bounded output reads → `kill_on_drop`. Any new `gh` subprocess tool should copy this pattern.

## Key Files

- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — tool implementation + 24 tests
- `crates/mika-agent/src/tools/mod.rs` — registration in `default_tools()`
- `crates/mika-agent/src/skills/executor.rs` — `scrub_mika_env_vars()` shared helper
