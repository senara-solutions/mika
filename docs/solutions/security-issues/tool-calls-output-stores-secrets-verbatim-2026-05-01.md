---
title: tool_calls.output stores secret-shaped values verbatim from file-reading tools
date: 2026-05-01
category: security-issues
module: mika-agent
problem_type: security_issue
component: tooling
symptoms:
  - "tool_calls.output contained real MIKA_GITHUB_TOKEN=github_pat_... from read_agent_file reading .env"
  - "Dashboard API GET /api/v1/traces/:trace_id/tool-calls served unredacted secrets to authenticated users"
  - "17-day durable-storage window before discovery during #903 audit"
root_cause: missing_validation
resolution_type: code_fix
severity: high
tags:
  - secret-scrubbing
  - tool-calls
  - persistence-boundary
  - redaction
  - read-agent-file
  - defense-in-depth
---

# tool_calls.output stores secret-shaped values verbatim from file-reading tools

## Problem

`tool_calls.output` durably stored verbatim file content from any tool that returned file data (`read_agent_file`, exec handlers, MCP tools). When an agent read a file containing secrets (e.g., `.env` with `MIKA_GITHUB_TOKEN=github_pat_...`), the secret persisted in `mika.db` indefinitely and was served via the dashboard API. The broader secret discipline (`SecretString`, `scrub_mika_env_vars`, MCP `env_clear`) operated at process boundaries but did not cover the internal `tool_calls` persistence path.

## Symptoms

- `tool_calls` row `461c76a1` (2026-04-13) contained a real GitHub PAT from `read_agent_file({path: ".env"})` for `mika-qa` agent
- 17-day exposure window before discovery during the #903 bash `set -x` audit
- Dashboard API at `/api/v1/traces/:trace_id/tool-calls` served the secret to any holder of `MIKA_DASHBOARD_TOKEN`
- Three columns at risk: `tool_calls.output`, `tool_calls.input`, and `tool_calls.error_message`

## What Didn't Work

- **Process-boundary scrubbing alone**: `SecretString` (compile-time), `scrub_mika_env_vars()` (exec handler children), and MCP `env_clear() + allowlist` are thorough at process boundaries but don't cover internal write paths like `save_tool_call()`
- **Per-tool scrubbing**: Scrubbing in individual tools (e.g., `read_agent_file`) would miss exec handlers, MCP tools, and custom skills that can also return file content

## Solution

Engine-side `scrub_secrets()` function applied at the `Database::save_tool_call()` persistence boundary, covering ALL tools universally:

**New module `crates/mika-agent/src/secret_scrubber.rs`:**
- `RegexSet` for O(1) fast rejection (common case: no secrets = zero allocation via `Cow::Borrowed`)
- 14 patterns covering: `github_pat_*`, `ghp_*`, `gho_*`, `ghs_*`, `ghu_*`, `sk-ant-(api|oat)*`, `sk-proj-*`, `sk-or-*`, `gsk_*`, `xoxb-*`, `xoxp-*`, PEM private keys, `MIKA_*{TOKEN,KEY,SECRET}=`, `GH(_APP)?_TOKEN=`
- Pattern minimum length requirements (10+ chars after prefix) reduce false positives

**Three application points:**
1. `Database::save_tool_call()` — scrubs `input`, `output`, AND `error_message` before the INSERT, before truncation
2. `ToolCallSummary` metadata — scrubs `input_summary` and `output_summary` before serialization to `messages.metadata`
3. Schema v28->v29 backfill migration — one-shot sweep of existing `tool_calls` rows

**Critical design choice:** The LLM's in-memory `ToolOutput` is NOT scrubbed. The agent needs real values to pass them onward (e.g., env-var injection into shell commands). Only the durable copy is sanitized.

## Why This Works

The root cause was a missing output boundary. The `tool_calls` table is a persistence boundary with the same security properties as network responses — it's durable, queryable, and served over HTTP via the dashboard API. By scrubbing at the single funnel point (`Database::save_tool_call()`), all current and future tools are covered without relying on each tool or caller to remember scrubbing.

The `RegexSet` + individual `Regex` pattern ensures the common case (no secrets in tool output) has near-zero overhead: one DFA pass, zero allocation.

## Prevention

- **Treat all persistence as an output boundary.** When adding new columns or tables that store tool/LLM output, apply `scrub_secrets()` at the write site. The existing `MIKA_STORE_TOOL_CALLS` toggle gates whether writes happen; when they do, they must be scrubbed.
- **Extend patterns when adding new secret types.** The `SECRET_PATTERNS` constant in `secret_scrubber.rs` is the single source of truth. Each new pattern needs positive and negative test cases.
- **Review finding: don't forget error paths.** The initial implementation missed `error_message` — when a tool fails, the error text carries the same content as `output` and needs the same treatment. Code review caught this before merge.
- **lefthook `no-secrets` hook exclusion.** The test file `secret_scrubber.rs` contains realistic token patterns for testing. Added `grep -v 'secret_scrubber'` to the no-secrets hook to avoid false positives on test data.

## Related Issues

- #908 — this fix
- #903 — sibling leak class (bash `set -x` trace forwarding)
- `docs/solutions/security-issues/bash-set-x-leaks-secrets-in-trace-and-callback-2026-04-30.md` — the audit that discovered this incident
- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — related env-var scrubbing pattern
- `docs/solutions/best-practices/secretstring-expose-at-boundary-pattern.md` — `[REDACTED]` convention
- `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md` — "treat logs as output boundary" principle
