---
title: "Required Tools Terminal Failure Bypass"
date: 2026-04-20
category: architecture-patterns
module: agent-core
problem_type: logic_error
component: tooling
symptoms:
  - "required_tools gate retries when a required tool fails with an unrecoverable error (e.g., GitHub self-approval)"
  - "Agent wastes LLM calls re-running the entire workflow and hitting the same terminal error"
  - "9 LLM calls instead of expected 4 in mika-qa PR review trace"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - required-tools
  - agent-loop
  - post-condition-guard
  - terminal-error
  - retry
  - github-api
---

# Required Tools Terminal Failure Bypass

## Problem

The `required_tools` gate (guard #2 in the 5-guard EndTurn chain) retries when required tools are missing, but doesn't distinguish between "agent skipped the tool" and "agent called a required tool, it failed terminally, and the remaining tools are pointless." This wastes LLM calls on unrecoverable workflows.

Observed in production: mika-qa trace `b12e6cbd5ce64b008be8369b21dced0b` (2026-04-10). `qa-review` requires `["qa_pr_view", "run_gh"]`. After `run_gh pr review --approve` failed with "Can not approve your own pull request" at step 2, the gate rejected EndTurn because `qa_pr_view` was never called. The agent re-ran the full review flow, hitting the same error. Total: 9 LLM calls instead of 4.

## Symptoms

- Required tools gate fires even when a required tool failed with a terminal error
- Agent re-executes an entire skill workflow that has already hit an unrecoverable wall
- Extra LLM calls (5 in the observed case) with no chance of success

## What Didn't Work

The prior fix (#516 partial) added `filter_available_required_tools()` to exclude tools not in the registry. This prevents retries for impossible-to-call tools but doesn't help when the tool IS available but its invocation fails terminally.

## Solution

Added terminal failure detection to the required_tools gate via two new functions:

**`is_terminal_tool_error(output: &str) -> bool`** classifies tool output text as terminal or retryable using two pattern lists checked against the lowercased output:

1. `RETRYABLE_ERROR_PATTERNS` — checked first, takes priority: HTTP 429/500/502/503/504, rate limits, timeouts, connection errors
2. `TERMINAL_ERROR_PATTERNS` — GitHub self-approval, HTTP 4xx, permission errors (specific phrases only, not bare words like "not found" which match too broadly)

Unknown errors (matching neither list) return `false` — conservative default preserves existing retry behavior.

**`has_terminal_required_tool_failure(required: &HashSet<String>, summaries: &[ToolCallSummary]) -> bool`** scans `all_tool_summaries` for any required tool with `success == false` and a terminal output pattern.

**Gate integration:** In the required_tools gate block, after computing `missing` tools:
- If `has_terminal_required_tool_failure()` returns true → log warning, set `required_tools_retry_done = true`, fall through to the next guard (no retry)
- Otherwise → existing retry behavior (push correction message, `continue`)

Key design decisions:
- **Retryable patterns take priority** — a 429 with "permission denied" text is still retryable
- **Patterns are intentionally specific** — bare words like "not found", "forbidden", "unauthorized", and "permission denied" were excluded to avoid false positives on search results, diagnostic output, or Unix filesystem errors
- **Any terminal failure on a required tool waives the entire gate** — required tools within a skill are part of the same workflow chain; if one fails terminally, remaining tools are likely pointless

## Why This Works

The root cause is that the gate had a binary view: tools either called or not called. But a tool that was called and failed terminally is fundamentally different from a tool that was never attempted. The fix adds a third classification: "called and failed unrecoverably," which allows EndTurn without retry.

The two-list pattern architecture (retryable checked first, terminal checked second, unknown defaults to retryable) ensures conservative behavior — new error patterns are retried until explicitly classified.

## Prevention

- New terminal error patterns can be added to `TERMINAL_ERROR_PATTERNS` as they're discovered in production traces
- Always check retryable patterns first in any error classification logic — retryable errors should never be misclassified as terminal
- When adding broad substring patterns to error classifiers, verify they don't match common non-error text (e.g., "not found" in search results)
- The `output_summary` field is truncated to 300 chars — terminal error patterns should appear early in tool output to be reliably detected

## Related Issues

- mika#516 — this fix
- mika#517 — availability filter (companion fix)
- mika#270 — original required_tools gate
- mika#265 — match-reason conditioning (Keyword-only enforcement)
- `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — original gate design
- `docs/solutions/prompt-engineering/required-tools-availability-filter.md` — availability filter
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — engine guard philosophy
