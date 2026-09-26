---
ticket: mika#985
title: "obs(logging): per-agent runtime log file is sparse — sink-architecture clarification"
type: obs
status: groomed-pending
labels: [bug, p1-important]
branch: obs/985/logging-mika-dev-runtime-log-file-is
related_tickets: [984]
---

# Plan — mika#985: per-agent runtime log file is sparse

## Verified state (root cause identified during plan drafting)

The audit ticket framed this as "per-agent log file is sparse despite DB activity," with the implicit assumption that `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` is the sink for the running mika-spirit's per-agent runtime events. That assumption is false. **This is not a regression — it's an architectural mismatch between two sink targets and a single audit recipe.**

### The two sinks

| Sink | Initializer | Server-side call site | Used by | Path | Rotation |
|---|---|---|---|---|---|
| Server log | `mika_common::logging::init()` (`crates/mika-common/src/logging.rs:208`) | `crates/mika-agent/src/bin/mika-spirit.rs` `main()` startup | `mika-spirit` (long-running daemon) | `MIKA_SPIRIT_LOG_FILE` (e.g. `/var/log/mika/server.log`) | None — single file via `tracing_appender::rolling::never` |
| Per-agent CLI log | `mika_common::logging::init_pretty()` (`crates/mika-common/src/logging.rs:314`) | `crates/mika-cli/src/main.rs:215` (per-agent), `:339` (team), `:367` (chat) | `mika-cli` (`mika ask`, `mika chat`, `mika team run`, etc.) | `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` (per-agent, daily) | Daily via `tracing_appender::rolling::daily` |

### The evidence

- Today's `~/.mika/agents/mika-dev/logs/mika.log.2026-05-05` has only 2 entries — both `GitHub App configured` from `mika_common::github_app`. These correspond to two `mika ask --agent mika-dev ...` CLI invocations (each one fires GitHub App init at startup and exits).
- `/var/log/mika/server.log` is fully populated (2.5 GB cumulative). The `mika#984` task audit found the relevant runtime events there: `long-running exec exited but task already in terminal state`, `resuming agent for callback task`, `task_engine_reaper: transitioned orphaned parent to failed` — all at `target: mika_agent::skills::executor` / `mika_agent::task_engine::*`.
- `mika-cli/src/main.rs:164` confirms: `let log_dir = agent_home.as_ref().map(|h| h.join("logs"));` — only the CLI sets a per-agent log dir.
- `mika-spirit`'s init path takes `log_file: Option<&Path>` from `MIKA_SPIRIT_LOG_FILE`, not from a per-agent home dir.

### What this means

The audit's `shared:log-grep` recipe in `/mika-audit` (canonical pattern: `~/.mika/agents/<agent_id>/logs/mika.log.YYYY-MM-DD`) is structurally wrong for server-mode events. It looks at the CLI sink while the runtime events go to the server sink. The DB telemetry is correct because both sinks emit the same events to `tracing` — only the file write target differs.

## Acceptance criteria

1. **Documentation:** `crates/mika-agent/CLAUDE.md` § Observability documents which sink contains which class of event. The decision tree is one paragraph: "If you want to read the running mika-spirit's runtime events, read `MIKA_SPIRIT_LOG_FILE`. If you want to read a specific `mika ask` or `mika chat` invocation's events, read `~/.mika/agents/<name>/logs/mika.log.<date>`. Both contain the same `agent_id` field on every entry, so cross-filtering by agent works in either sink."
2. **Audit recipe correction:** `/mika-audit`'s `shared:log-grep` helper (in `mika-platform/.claude/commands/mika-audit.md`) reads from `MIKA_SPIRIT_LOG_FILE` filtered by `agent_id`, with a fallback to the per-agent CLI log path. The recipe explicitly notes both sinks and which one to prefer.
3. **Smoke test (optional, low effort):** add a `mika logs --agent <name>` subcommand or extend `mika status` to print the resolved log paths so operators don't need to remember which sink is which.

## Plan

### Step 1 — Documentation in `crates/mika-agent/CLAUDE.md`

Add a new "Log Sinks" subsection under § Observability with the table from "Verified state" above and the cross-reference rule. Cite `mika_common::logging::init()` vs `init_pretty()` and the server-side call site (`crates/mika-agent/src/bin/mika-spirit.rs main()`) so future readers can find the source of truth.

**Include the single-sink rationale.** State explicitly that mika-spirit uses one file with `agent_id`-filtered queries instead of per-agent appenders, because per-agent appenders would (a) double the disk-write rate per event, (b) create a sync gap risk if the per-agent worker can't keep up, and (c) duplicate data already correctly addressable via the JSON `agent_id` field. The rationale belongs next to the table so future edits to the architecture see it without going to the plan archive.

### Step 2 — Update `/mika-audit`'s `shared:log-grep`

Edit `mika-platform/.claude/commands/mika-audit.md` § shared:log-grep:
- Default sink becomes the server log file. Resolve via `MIKA_SPIRIT_LOG_FILE` env var (or fall back to `/var/log/mika/server.log` as the documented production default).
- Filter is now `jq 'select(.agent_id == "<agent_id>" and .timestamp >= "<start>" and .timestamp <= "<end>")'` instead of relying on path-based agent scoping.
- The per-agent CLI sink stays documented as a fallback for "what did this specific `mika ask` invocation do" — useful for grooming-tool debugging, not for tracing autonomous-loop wedges.
- Update the rendering header to print which sink was used: `**Log file:** <path> (<size>) | **Sink:** server|cli` so the audit output discloses its source of truth.

### Step 3 — Verify the recipe against today's evidence

After Step 2 lands, re-run `/mika-audit task 0b38a0ec-4d4b-4039-b43b-c05d4e121bb3` (the failed mika#666 dispatch). The audit should now find the `long-running exec exited but task already in terminal state` and `resuming agent for callback task` lines that were invisible to the old recipe. If it doesn't, Step 2's filter is wrong and needs revision.

### Step 4 — Optional `mika logs --agent <name>` subcommand

If Step 3 reveals operators need easier sink discovery, add a thin CLI subcommand that prints both resolved paths and tail the appropriate one. Out-of-scope if Steps 1-3 are sufficient.

## Out of scope (explicit)

- **Per-agent file appender on mika-spirit.** Adding `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` as a second sink for the running mika-spirit would make the audit recipe "just work" without doc/recipe changes — but it adds disk writes per event for every per-agent action, doubles log volume, and creates a sync gap (events lost if the per-agent appender's worker can't keep up). Server log + agent_id filter is the cheaper, lossless answer.
- **Schema change to enrich `agent_id` field on every log line.** All entries already carry `agent_id` in the JSON envelope (verified by spot-check in `/var/log/mika/server.log`). No code change needed.
- **CLI-mode log routing changes.** The per-agent CLI sink is correct as-is for its purpose (discrete invocations).

## Risk and reversibility

- All three steps are documentation + audit-recipe edits. No production-runtime change. Trivially reversible.
- Step 4 is optional and additive; if shipped, no removal needed.
- The "audit was reading the wrong file" framing has zero functional impact on running services — only on operator debugging speed.

## Files touched (estimated)

- `mika/crates/mika-agent/CLAUDE.md` — add § Log Sinks with single-sink rationale (Step 1)
- `mika-platform/.claude/commands/mika-audit.md` — update shared:log-grep (Step 2)
- `mika/crates/mika-cli/src/commands/logs.rs` (new, optional) — `mika logs` subcommand (Step 4 if needed)
- `mika/crates/mika-cli/src/main.rs` — wire subcommand if Step 4 ships

## Delivery shape (cross-repo)

Two PRs per `mika-platform/CLAUDE.md` "primary + direct" cross-repo guidance:

- **Primary PR (mika repo):** Step 1 + Step 3 verification + Step 4 if needed. Largest change, owns the architectural docs.
- **Secondary PR (mika-platform repo):** Step 2 audit-recipe edit only. Single-file change to `.claude/commands/mika-audit.md`.

Both PRs cross-reference via `Companion PR: senara-solutions/<other-repo>#<n>` in the body. Same branch name (`obs/985/logging-mika-dev-runtime-log-file-is`) across both repos for traceability. Land in any order — they're independent (mika-platform recipe edit doesn't break if mika docs ship later, and vice versa).

## Why P1, not P0

Audit operators currently fall back to the DB telemetry, which has the same fidelity as logs (the events are the same; only the file write target differs). The cost is debugging speed, not correctness. P1 — fix soon, not blocking.

## Cross-cutting note (mika#984)

This ticket was filed today during the mika#984 audit specifically because the `validate_required_fields` warn was expected to surface in the per-agent log file but did not. Once the audit recipe is corrected (Step 2), the same trace becomes immediately visible in the server log. Useful checkpoint for the mika#984 F5 step (Phase 0.5.1 of mika#984's plan): once the trace lands, find it via the corrected recipe, not via the empty per-agent file.

## What this plan deliberately does NOT do

- Does NOT add a third log sink. Two sinks is correct given two execution modes (server daemon vs discrete CLI invocation).
- Does NOT promote `MIKA_SPIRIT_LOG_FILE` from "optional" to "required." The default fallback path (`/var/log/mika/server.log` per the deployed gentoo init script) is fine; the env var is the override.
- Does NOT change log levels or filters. The log content is correct; only the audit recipe pointer is wrong.
