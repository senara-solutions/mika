---
type: feat
issue: 1705
title: Persist claude-pilot subprocess transcripts to mika.db — instrument-first Part 2 for owned-model bet
status: draft
---

# Plan — mika#1705 pilot-transcript capture (instrument-first Part 2)

## Ticket

mika#1705 — capture claude-pilot subprocess LLM call transcripts so they're queryable. **Critical path for Vincent's owned-model bet 2026-07-01** ("my money is on it"). Part 1 (MIKA_LOG_LLM_BODIES=true) enabled 2026-07-01 ~08:50Z — captures in-process ~10% of trajectory. Part 2 (this ticket) captures the other 90% (implementation reasoning inside claude-pilot subprocess).

## Problem

`run_claude_pilot` spawns claude-pilot-py subprocess. Inside that subprocess, claude-pilot-py calls Claude Code CLI which makes LLM calls to Anthropic/OpenRouter. **None of those subprocess LLM calls are persisted anywhere queryable.** They land in claude-pilot's stdout log at best, discarded at worst.

Per wizzard-CC 2026-07-01 finding: the corpus we HAVE (in `llm_calls` table, orchestration verbs like `update_task_status`, `send_message`) is not what a 7B tool-use model needs. The implementation reasoning — grep-then-edit patterns, debug-then-fix cycles, plan-then-implement traces — lives in claude-pilot's subprocess. Missing.

Vincent's ships-yesterday motto: this is the load-bearing missing piece. Ship v1 today if possible.

## Scope

**In scope (v1 ships fast — motto-aligned):**

Option 3 from ticket body — **filesystem-only capture with mika-side ingestion tick**. Ships fastest:

1. **claude-pilot-py side** — add `ANTHROPIC_LOG_FILE` (or similar) env var support. When set, claude-pilot-py appends JSONL entries per LLM call. dispatch-lib.sh sets this env var to `~/.mika/data/pilot-transcripts/<task-id>.jsonl` at spawn time.
2. **mika-spirit side** — new tick (piggyback on existing 60s watchdog tick) scans `~/.mika/data/pilot-transcripts/*.jsonl`, imports finished files into a new `pilot_transcripts` table linked to `tasks.id`, deletes the JSONL file after successful import.
3. **New table** — `pilot_transcripts(id, task_id, timestamp, provider, model, request_body TEXT, response_body TEXT, tokens_in INTEGER, tokens_out INTEGER, latency_ms INTEGER)`. Schema v40. Async DB write.
4. **Env-var gated** — `MIKA_LOG_PILOT_TRANSCRIPTS=true` (default: on once shipped, but gateable).
5. **Retention** — 3 months default via nightly cron/tick. `MIKA_PILOT_TRANSCRIPT_RETENTION_DAYS=90`.

**Out of scope (v1):**
- HTTP push from claude-pilot to mika-spirit (Option 2 from ticket body — proper, but multi-repo scope creep).
- Structured labeling/filtering of trajectories (feeds owned-model training; separate ticket).
- Cross-agent aggregation (multi-user containers).

## Committed positions

1. **Filesystem + tick shape** (Option 3) — fastest to ship, matches motto. If wizzard-side training ever needs realtime streaming (Option 2), that's a v2 follow-up.
2. **JSONL not JSON** — append-only, no lock contention if multiple pilot subprocesses run in parallel. Line-per-call.
3. **Delete after import** — no double-storage. Tick is idempotent (import + delete atomic per file).
4. **DB schema v40** — new bump. Migration is additive (new table only, no ALTER on existing).
5. **claude-pilot-py side change is minimal** — 30-line env-var-driven append handler in the SDK wrapper. No refactor.

## Acceptance criteria

- **AC1** — `MIKA_LOG_PILOT_TRANSCRIPTS=true` env var recognized by mika-spirit. When set, dispatch-lib.sh exports `ANTHROPIC_LOG_FILE=~/.mika/data/pilot-transcripts/<task-id>.jsonl` for each claude-pilot subprocess spawn.
- **AC2** — claude-pilot-py (fork or PR upstream) writes JSONL entry per LLM call when `ANTHROPIC_LOG_FILE` is set. Entry: `{timestamp, provider, model, request_body, response_body, tokens_in, tokens_out, latency_ms}`.
- **AC3** — New `pilot_transcripts` table (schema v40) with the columns above + `task_id` FK. Additive migration.
- **AC4** — Ingestion tick: 60s cadence, scans `~/.mika/data/pilot-transcripts/*.jsonl`, imports rows + deletes file. Idempotent (same file processed twice = no duplicates).
- **AC5** — Query: `SELECT COUNT(*) FROM pilot_transcripts WHERE task_id IN (SELECT id FROM tasks WHERE created_at > date('now', '-1 day'))` returns non-zero within 24h of ship.
- **AC6** — Retention: `MIKA_PILOT_TRANSCRIPT_RETENTION_DAYS=90` (default 90). Daily tick deletes rows older than N days.
- **AC7** — `docs/observability/pilot-transcript-growth.md` — weekly growth measurement doc. Reports row count + disk usage. Feeds wizzard's accumulation-rate estimate.
- **AC8** — `cargo test -p mika-agent` clean. Regression: existing dispatches work unchanged when `MIKA_LOG_PILOT_TRANSCRIPTS=false`.

## Implementation steps (dispatch order)

**Phase 1 — schema + ingestion tick (mika side, ~2h):**
- Add v40 migration for `pilot_transcripts` table in `crates/mika-agent/src/db.rs`.
- Add ingestion loop in engine tick (alongside callback watchdog, per mika#959 pattern).
- Env-var wiring for `MIKA_LOG_PILOT_TRANSCRIPTS` + `MIKA_PILOT_TRANSCRIPT_RETENTION_DAYS`.

**Phase 2 — dispatch-lib env-var passthrough (~30min):**
- In `skills/bundled/_shared/dispatch-lib.sh` claude-pilot invocation: if `MIKA_LOG_PILOT_TRANSCRIPTS=true`, set `ANTHROPIC_LOG_FILE=~/.mika/data/pilot-transcripts/${TASK_ID}.jsonl` in the child env.
- Ensure `~/.mika/data/pilot-transcripts/` directory exists (dispatch-lib creates on demand).

**Phase 3 — claude-pilot-py side (~1-2h):**
- In `claude-pilot-py/src/claude_pilot/`, add per-LLM-call hook that appends JSONL to `ANTHROPIC_LOG_FILE` if env var set.
- Handle atomic append (`open('a')` + fsync per line).
- Unit test with a mock LLM provider.

**Phase 4 — retention tick + docs (~30min):**
- Daily retention tick.
- Growth doc + weekly measurement script.

**Phase 5 — deploy + verify:**
- `make deploy` from meta-repo. Verify one dispatch produces a JSONL file that gets imported.
- Query the new table. Confirm non-zero after next real dispatch.

## Verification

- Manual test: set `MIKA_LOG_PILOT_TRANSCRIPTS=true`, dispatch a fast task. After 60s, `sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM pilot_transcripts WHERE task_id = '<id>'"` returns non-zero.
- File cleanup: JSONL file exists during subprocess, deleted after import.
- Regression: set `MIKA_LOG_PILOT_TRANSCRIPTS=false`, dispatch a task; no JSONL file created; existing behavior unchanged.
- `cargo test -p mika-agent` clean.
- No perceptible latency increase on dispatched tasks (append is async).

## Risks

1. **claude-pilot-py fork vs upstream PR.** If we fork, we own maintenance. If we upstream PR, waits on Anthropic's review cadence. Recommendation: **fork first for v1, upstream PR after we validate the shape.** Ships-yesterday motto.
2. **Disk growth.** JSONL files during dispatch + persistent DB rows. Cap: retention default 90 days. Weekly measurement doc catches runaway growth.
3. **Schema bump v40.** Additive; no data loss risk. But every mika instance needs migration to run cleanly. Standard pattern.
4. **claude-pilot subprocess doesn't honor ANTHROPIC_LOG_FILE.** Some env-var patterns need explicit SDK support. If Anthropic SDK doesn't have this hook, we need a wrapper layer. Verify Phase 3 step 1.
5. **Race on JSONL import.** If pilot is still writing when ingestion tick fires, tick reads partial. Mitigation: only import files whose corresponding task is in terminal state (completed/failed/blocked), not in_progress.
6. **Secret leakage in request bodies.** LLM request bodies may contain code with secrets. Reuse existing `secret_scrubber::scrub_secrets()` before insert.

## Out of scope (repeated)

- HTTP push from claude-pilot to mika-spirit — v2.
- Trajectory labeling/filtering pipeline — separate wizzard-side ticket.
- Cross-agent aggregation.

## References

- mika#1705 — this ticket
- wizzard#16 finding — `wizzard/docs/brainstorms/2026-07-01-trajectory-corpus-feasibility.md` (commit 1d7f880)
- [[project-mika-owned-model-dev-qa-quality-first]] — the decision framework
- MIKA_LOG_LLM_BODIES=true enabled 2026-07-01 ~08:50Z (Part 1 corpus, in-process)
- Vincent direction 2026-07-01: "my money is on it (glm fine tuning)"
- `crates/mika-agent/src/tools/claude_pilot.rs` — subprocess spawn point
- `skills/bundled/_shared/dispatch-lib.sh` — env-var pass-through point
- `crates/mika-agent/src/db.rs` — schema location (currently v39; this bumps to v40)
- `crates/mika-common/src/secret_scrubber.rs` — for AC scrub step
- mika#959 callback watchdog tick — precedent for the 60s ingestion tick pattern
