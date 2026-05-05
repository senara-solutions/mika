---
module: observability
date: "2026-05-06"
problem_type: documentation_gap
component: documentation
severity: medium
tags: [logging, tracing, mika-server, mika-cli, audit, observability]
applies_when:
  - Audit tooling reads per-agent log files for server-mode events
  - Operators expect mika-server runtime events in ~/.mika/agents/<name>/logs/
  - Debugging autonomous-loop issues using per-agent CLI log paths
---

# Log Sink Architecture Mismatch

## Context

An audit (`/mika-audit task ...`) reported that the per-agent runtime log file at `~/.mika/agents/mika-dev/logs/mika.log.2026-05-05` was "sparse" — only 2 entries despite the DB recording 12+ LLM calls and 3+ tool calls in the same window. The initial assumption was a logging regression.

Investigation revealed this is not a regression — it's an architectural mismatch between two log sinks and the audit recipe pointing at the wrong one.

## Guidance

mika has **two distinct log sinks**, each written by a different process:

| Sink | Process | Path | Contains |
|------|---------|------|----------|
| Server log | `mika-server` (long-running daemon) | `MIKA_SERVER_LOG_FILE` (default: `/var/log/mika/server.log`) | All runtime events: skill execution, task engine, callbacks, autonomous-loop lifecycle |
| Per-agent CLI log | `mika` CLI (`mika ask`, `mika chat`) | `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` | Events from discrete CLI invocations only |

**The key insight:** Both sinks write the same structured JSON with an `agent_id` field. The CLI log is per-agent by path; the server log is per-agent by filter. They are not duplicates — they capture events from different processes.

**Query patterns:**
- Server-mode events: `jq 'select(.agent_id == "mika-dev")' /var/log/mika/server.log`
- CLI invocation events: read `~/.mika/agents/mika-dev/logs/mika.log.2026-05-06` directly

**Use `mika logs --agent <name>` to print both resolved paths with sizes and filter commands.**

## Why This Matters

Audit tooling that reads only the per-agent CLI log path will find it nearly empty for server-mode events. This creates false "sparse logging" reports and blocks debugging of autonomous-loop issues. The data exists — it's just in the server log, not where the audit recipe looked.

The single-sink rationale for mika-server (no per-agent file appenders) is intentional:
1. Per-agent appenders would double disk-write rate per event
2. Sync gaps risk if a per-agent appender worker can't keep up
3. Duplicates data already addressable via `agent_id` JSON field

## When to Apply

- Building or modifying audit/grep tooling that reads agent log files
- Debugging "missing log entries" for server-mode agent activity
- Writing new observability features that need to find runtime events
- Any time an operator reports "logs are empty" for an agent

## Examples

**Wrong (reads CLI sink for server events):**
```bash
# Will be nearly empty — only CLI invocations write here
grep "task_engine" ~/.mika/agents/mika-dev/logs/mika.log.2026-05-05
```

**Right (reads server sink filtered by agent_id):**
```bash
# Full runtime events filtered by agent
jq 'select(.agent_id == "mika-dev" and .timestamp >= "2026-05-05T21:18:00")' \
  /var/log/mika/server.log
```

**Discovery command:**
```bash
# Shows both paths, sizes, and ready-to-use filter commands
mika logs --agent mika-dev
```
