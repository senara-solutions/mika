# Plan: Reconcile hot-patched skill prompts to bundled source (#638)

## Context

During the mika milestone#7 audit (2026-04-18), several prompt fixes were applied directly to deployed skill files (`~/.mika/agents/*/skills/`). These are live and tested but not in the bundled source — they'll be lost on the next `make deploy`.

## Changes

### self-dev/system_prompt.md (3 fixes)

1. **Milestone M1-M3 hardening** — CRITICAL gate warning, mika milestone#7 incident reference, mandatory M2/M3 gates to prevent skipping issue enumeration or creating incomplete children. Root cause: agent pattern-matched into single-issue dispatch mode, skipping M2 entirely.

2. **Memory recording in milestone workflow** — `store_fact(category="event")` at 4 points: M3 init, child complete, child blocked/failed, M5 completion. Root cause: agent had no persistent memory of milestone execution, making post-mortem diagnosis impossible.

3. **Rule 8 expansion** — Status notifications after `run_claude_pilot` dispatch must not include PR numbers until callback confirms. Incident: agent fabricated PR #640 for mika#608 while claude-pilot was still running.

### qa-review/system_prompt.md (1 fix)

4. **Memory recording after PR reviews** — "Record to memory" section: `store_fact(category="event")` after every review, `store_fact(category="preference")` for recurring patterns across 2+ PRs. Root cause: mika-qa had zero archival memory entries from dozens of completed reviews.

## Approach

Copy hot-patched files to bundled source. No code changes — prompt-only.
