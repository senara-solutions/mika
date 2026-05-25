---
name: mika-audit
description: Audit a task, session, turn, team run, milestone, PR, or dev run
argument-hint: "<scope> [<id>] [<observation>]"
---

Unified audit command. Dispatches to scope-specific logic while sharing data-gathering helpers, output skeleton, and red-flag rendering.

## Argument parsing

Parse `$ARGUMENTS`:

1. **First token: `<scope>`** — must be one of: `task`, `session`, `turn`, `team`, `milestone`, `pr`, `dev-run`. If the first token is not in this set, exit with:
   ```
   Usage: /mika-audit <scope> [<id>] [<observation>]
   Scopes: task, session, turn, team, milestone, pr, dev-run
   ```
2. **Second token: `<id>`** — scope-dependent format:
   - `task`: agent name (e.g., `mika-dev`) or task ID (UUID, 36 chars with dashes)
   - `session`: agent name or session ID (UUID)
   - `turn`: agent name (required)
   - `team`: team name (e.g., `inner-circle`) or run ID (UUID)
   - `milestone`: milestone name (optional — defaults to next open milestone)
   - `pr`: PR reference — `https://github.com/senara-solutions/<repo>/pull/<N>` or `<repo>#<N>`
   - `dev-run`: task ID (UUID, optional — defaults to latest mika-dev self_dev task)
3. **Remainder: `<observation>`** — free-text user observation. Store for the Red Flags section.

Detect UUID format: contains dashes and is 36 characters.

---

## Shared helpers

All scope sections below reference these helpers by name. Each helper is defined exactly once here. **No scope section may redefine or inline these patterns** — that would reintroduce the DRY violation this consolidation resolves.

### shared:db

All SQL queries run against `~/.mika/data/mika.db` unless stated otherwise.

### shared:log-grep

**Canonical log-grep pattern.** Every scope that reads agent runtime logs MUST use this helper, passing scope-specific parameters.

**Parameters:**
- `<agent_id>` — the agent whose logs to read
- `<start_timestamp>` — ISO timestamp for window start
- `<end_timestamp>` — ISO timestamp for window end (or `now` if still in progress)

**Determine log files:** Extract date(s) from start/end timestamps. If the window spans midnight, check both date files.

**Step A — Extract significant entries:**
```bash
grep -E '"level":"(WARN|ERROR)"|"agent done"|"agent exceeded"|"skills loaded"|"compacting"|"long-running exec failed"|"task cancelled"' \
  ~/.mika/agents/<agent_id>/logs/mika.log.YYYY-MM-DD \
  | jq -r 'select(.timestamp >= "<start_timestamp>" and .timestamp <= "<end_timestamp>") | [.timestamp, .level, .message, .target] | @tsv' \
  | head -50
```

**Step B — Count WARN/ERROR:**
```bash
grep -E '"level":"(WARN|ERROR)"' ~/.mika/agents/<agent_id>/logs/mika.log.YYYY-MM-DD \
  | jq -r 'select(.timestamp >= "<start_timestamp>" and .timestamp <= "<end_timestamp>") | .level' \
  | sort | uniq -c
```

**Step C — Cross-reference with DB telemetry:** If LLM/tool call queries showed zero entries but logs show `llm_call started` entries, the telemetry storage is broken (`MIKA_STORE_LLM_CALLS` may be off, or DB writes are failing). Flag this gap.

**If no log directory, files, or matching lines exist:** Report "No agent runtime log entries found for `<agent_id>` in window `<start>` → `<end>`." and continue.

**Rendering:**
```
### Agent Runtime Logs
**Log file:** <path> (<size>) | **Time window:** <start> → <end>
| Timestamp | Level | Message | Target |
|-----------|-------|---------|--------|
...
**Summary:** X warnings, Y errors | Telemetry cross-check: <match / gap detected>
```

### shared:log-grep-by-trace

Variant of `shared:log-grep` that filters by `trace_id` instead of timestamp window. Used by scopes that have a precise trace correlation key (turn, team).

```bash
grep '<trace_id>' ~/.mika/agents/<agent_id>/logs/mika.log.YYYY-MM-DD \
  | jq -r '[.timestamp, .level, .message, .target] | @tsv'
```

If trace_id yields no results, fall back to `shared:log-grep` with timestamp window.

### shared:kg-precheck

Before running any KG query in any scope, execute:
```sql
SELECT MAX(version) FROM schema_version
```

If the returned value is NULL or < 28, render:
> *Skipping KG sections — container is on schema v<value or "unknown">; KG tables require v28+. v27 added shared-corpus PK (#787); v28 added `agent_kg_corpora` (#798). Audits target the latest schema only; older containers should snapshot pre-deploy or wait for the next deploy cycle.*

...and skip ALL KG queries in this audit. All pre-existing (non-KG) sections run as before.

If agent has zero rows in `agent_kg_corpora`, render: "No KG activity in scope." and skip remaining KG queries.

### shared:kg-llm-bucketing

Bucket LLM calls as extraction / resolution / conversational. Used by task, session, and team scopes.

**Parameters:** `<agent_id>`, `<session_filter>` (either `lc.session_id IN (<ids>)` or `lc.trace_id = '<trace_id>'`)

```sql
SELECT
  CASE
    WHEN lc.trace_id IN (
        SELECT extraction_trace_id FROM kg_extractions
        WHERE docs_root_hash IN (
            SELECT docs_root_hash FROM agent_kg_corpora WHERE agent_id = '<agent_id>'
        )
    ) THEN 'extraction'
    WHEN lc.trace_id IN (SELECT resolution_trace_id FROM kg_resolutions_log WHERE agent_id = '<agent_id>') THEN 'resolution'
    ELSE 'conversational'
  END AS kind,
  lc.provider, lc.model, COUNT(*) AS calls, SUM(lc.input_tokens+lc.output_tokens) AS tokens
FROM llm_calls lc
WHERE <session_filter>
GROUP BY kind, lc.provider, lc.model
```

Report conversational vs KG cost side by side. If any rows were classified via model-name fallback (NULL trace_id), label them "KG (model-name fallback, trace_id unavailable)." Omit the fallback label if no rows used the fallback path.

**Rendering:**
```
**LLM Call Bucketing:**
| Kind | Provider | Model | Calls | Tokens |
|------|----------|-------|-------|--------|
| conversational | ... | ... | N | N |
| extraction | ... | ... | N | N |
| resolution | ... | ... | N | N |
```

### shared:kg-query-tool-usage

Count `query_knowledge_graph` tool calls and fallback rate. Used by task, session, turn, and team scopes.

**Parameters:** `<filter>` (session_id or trace_id filter clause)

```sql
SELECT COUNT(*) as total_calls,
       SUM(CASE WHEN tc.success = 1 THEN 1 ELSE 0 END) as successes,
       SUM(CASE WHEN tc.success = 0 THEN 1 ELSE 0 END) as failures,
       AVG(tc.latency_ms) as avg_latency_ms
FROM tool_calls tc
WHERE <filter>
  AND tc.tool_name = 'query_knowledge_graph'
```

Fallback count:
```sql
SELECT COUNT(*) as fallback_count
FROM tool_calls tc
WHERE <filter>
  AND tc.tool_name = 'query_knowledge_graph'
  AND (tc.output LIKE '%starting_entity_missing%' OR tc.output LIKE '%fallback_reason%')
```

Report: total calls, success rate, fallback rate (fallback_count / total_calls). If zero calls, omit this sub-section.

### shared:kg-extraction-check

Check if a scope triggered compound-hook extraction. Used by task, session, and turn scopes.

**Parameters:** `<trace_filter>` (e.g., `WHERE extraction_trace_id = '<trace_id>'` or `WHERE extraction_trace_id IN (SELECT DISTINCT m.trace_id FROM messages m WHERE m.session_id IN (<session_ids>))`)

```sql
SELECT ke.source_doc_path, ke.entities_extracted, ke.relationships_extracted,
       ke.extraction_model, ke.docs_root_hash, substr(ke.created_at,1,19) as time
FROM kg_extractions ke
<trace_filter>
```

Note: extraction is corpus-scoped; the resulting `kg_extractions` row is shared across all agents using that corpus. Display the corpus hash alongside each row.

### shared:observation-handling

If the user passed an observation as the last argument:
- Lead the Red Flags / Analysis section with observation-specific findings
- Cross-reference the observation against ALL gathered data (DB telemetry, logs, tool calls, KG metrics)
- Do NOT echo the observation back — report what the data shows about it
- Provide a targeted diagnosis grounded in evidence

### shared:red-flag-rendering

Format the Red Flags / Analysis section as:
- Bulleted list of actual issues found, citing evidence
- If no issues: "No issues detected"
- If an observation was provided: observation-specific findings come first (per `shared:observation-handling`), then remaining flags
- Each flag states the fact, cites the source (query, log line, metric), and suggests action if applicable

### shared:output-skeleton

Every scope renders output in this order:
1. **Summary header** — scope-specific subject identification and key metadata
2. **Scope-specific detail sections** — tables, timelines, breakdowns per the scope's spec
3. **KG Usage / KG Health** (v28+) — per `shared:kg-precheck`, skip if pre-v28 or no activity
4. **Agent Runtime Logs** — per `shared:log-grep`
5. **Red Flags / Analysis** — per `shared:red-flag-rendering` and `shared:observation-handling`
6. **Reference** — cross-references to other `/mika-audit` scopes (not external commands)

---

## Scope: task

Audit an agent's task — task tree, claude-pilot logs, runtime logs, PRs, and pipeline quality.

**ID format:** agent name (e.g., `mika-dev`) or task ID (UUID). If agent name, resolve to latest manual task for that agent.

### Step 1: Find the task

**If agent name:**
```sql
SELECT id, label, status, source, created_at
FROM tasks
WHERE agent_id = '<agent_name>' AND trigger_type = 'manual'
ORDER BY created_at DESC LIMIT 1
```

**If task ID:**
```sql
SELECT id, label, status, source, agent_id, created_at
FROM tasks WHERE id = '<task_id>'
```

Store `agent_id` and task `id`.

### Step 2: Get the task tree

```sql
SELECT id, label, status, trigger_type, action_type, substr(created_at,1,19) as created
FROM tasks
WHERE id = '<task_id>' OR parent_task_id = '<task_id>'
ORDER BY created_at
```

Present as table: main task status, session count, each session's status.

Collect all session IDs:
```sql
SELECT DISTINCT s.id as session_id, s.channel_type, substr(s.started_at,1,19) as started, substr(s.ended_at,1,19) as ended
FROM sessions s
JOIN tasks t ON s.agent_id = t.agent_id
WHERE (t.id = '<task_id>' OR t.parent_task_id = '<task_id>')
  AND s.started_at >= (SELECT created_at FROM tasks WHERE id = '<task_id>')
ORDER BY s.started_at
```

### Step 3: LLM call analysis

```sql
SELECT lc.session_id, lc.provider, lc.model,
       COUNT(*) as call_count,
       SUM(lc.input_tokens) as total_input, SUM(lc.output_tokens) as total_output,
       SUM(lc.cache_read_tokens) as total_cache_read, SUM(lc.cache_write_tokens) as total_cache_write,
       SUM(lc.latency_ms) as total_latency_ms,
       SUM(CASE WHEN lc.status = 'error' THEN 1 ELSE 0 END) as error_count,
       GROUP_CONCAT(DISTINCT lc.stop_reason) as stop_reasons
FROM llm_calls lc
WHERE lc.agent_id = '<agent_id>' AND lc.session_id IN (<session_ids>)
GROUP BY lc.session_id, lc.provider, lc.model
ORDER BY lc.session_id
```

Anomalous calls:
```sql
SELECT lc.id, substr(lc.created_at,1,19) as time, lc.session_id, lc.step, lc.provider, lc.model,
       lc.input_tokens, lc.output_tokens, lc.cache_read_tokens, lc.latency_ms,
       lc.stop_reason, lc.status, lc.error_message
FROM llm_calls lc
WHERE lc.agent_id = '<agent_id>' AND lc.session_id IN (<session_ids>)
  AND (lc.status = 'error' OR lc.stop_reason = 'max_tokens' OR lc.latency_ms > 30000)
ORDER BY lc.created_at
```

**KG bucketing (v28+):** Run `shared:kg-precheck`. If schema >= 28, run `shared:kg-llm-bucketing` with `lc.agent_id = '<agent_id>' AND lc.session_id IN (<session_ids>)`.

Compute: total tokens, cache efficiency (cache_read / input), average latency, error rate, model distribution, token burn rate per session.

### Step 4: Tool call analysis

```sql
SELECT tc.session_id, tc.tool_name, tc.tool_source, tc.skill_name,
       COUNT(*) as call_count,
       SUM(CASE WHEN tc.success = 1 THEN 1 ELSE 0 END) as successes,
       SUM(CASE WHEN tc.success = 0 THEN 1 ELSE 0 END) as failures,
       SUM(CASE WHEN tc.non_zero_exit = 1 THEN 1 ELSE 0 END) as non_zero_exits,
       SUM(tc.latency_ms) as total_latency_ms, AVG(tc.latency_ms) as avg_latency_ms
FROM tool_calls tc
WHERE tc.agent_id = '<agent_id>' AND tc.session_id IN (<session_ids>)
GROUP BY tc.session_id, tc.tool_name, tc.tool_source
ORDER BY tc.session_id, call_count DESC
```

Failed tool calls:
```sql
SELECT tc.id, substr(tc.created_at,1,19) as time, tc.session_id, tc.step, tc.tool_name,
       tc.tool_source, tc.skill_name, tc.latency_ms, tc.error_message,
       substr(tc.input,1,200) as input_preview, substr(tc.output,1,200) as output_preview
FROM tool_calls tc
WHERE tc.agent_id = '<agent_id>' AND tc.session_id IN (<session_ids>)
  AND (tc.success = 0 OR tc.non_zero_exit = 1)
ORDER BY tc.created_at
```

Retry/loop detection:
```sql
SELECT tc.tool_name, tc.step, COUNT(*) as consecutive_calls,
       substr(tc.input,1,100) as input_sample
FROM tool_calls tc
WHERE tc.agent_id = '<agent_id>' AND tc.session_id IN (<session_ids>)
GROUP BY tc.session_id, tc.tool_name, tc.step
HAVING COUNT(*) > 2
ORDER BY consecutive_calls DESC
```

**KG tool usage (v28+):** If KG active, run `shared:kg-query-tool-usage` and `shared:kg-extraction-check` scoped to the task's sessions.

### Step 5: Read claude-pilot logs

For each callback child task, check `/var/log/claude-pilot/<task_id>.log`. Also list all `/var/log/claude-pilot/*.log` files modified today.

For each log found:
- Read first 5 lines (config, session, prompt) and last 10 lines (completion status, cost, turns)
- Extract: session ID, turns, cost, duration from the `[done]` line
- Check if the prompt used raw `/ce:*` commands instead of `/mika` (bypasses the quality pipeline)

Cross-reference log-reported cost/turns with LLM call data from Step 3. Flag discrepancies.

### Step 6: Check agent runtime logs

Run `shared:log-grep` with `<agent_id>`, task's `created_at` as start, task's end time (or now) as end.

### Step 7: Check the branch and PR

From log prompts or task labels, identify the branch name. Then:
```bash
gh pr list --repo senara-solutions/<repo> --state all --head <branch> --json number,title,state,url
```

Check: CI status, files changed (includes `docs/plans/`? `docs/solutions/`?), conflicts.

### Step 8: Assess pipeline quality

| Artifact | Check | Status |
|----------|-------|--------|
| Plan doc | `docs/plans/` file in PR diff | ? |
| Source changes | Non-doc files in PR diff | ? |
| Review findings | `todos/` files in branch | ? |
| Compound doc | `docs/solutions/` file in PR diff | ? |

### Step 9: Render output

```
## Task Audit: <label>
**Agent:** <agent_id> | **Task:** <id> | **Status:** <status> | **Source:** <source>
**Created:** <timestamp>
```

Then: Sessions table, LLM Usage table (with totals), Tool Usage table, Failed Tool Calls, Pipeline Artifacts, KG Usage (v28+), Agent Runtime Logs (via `shared:log-grep`), Red Flags (via `shared:red-flag-rendering`).

### Task-scope red flags

**Cost & efficiency:** tokens > 500K, cache efficiency < 50%, error rate > 10%, repeated `max_tokens`.
**Tool health:** failure rate > 20%, same tool failing repeatedly, non_zero_exit, latency > 30s.
**Workflow:** claude-pilot turn count mismatch, no telemetry recorded, skipped pipeline steps, raw `/ce:*` instead of `/mika`.
**Runtime logs:** WARN/ERROR entries, telemetry gap, `agent exceeded max tool steps`, repeated `compacting`, `long-running exec failed`, no logs found.
**KG (v28+):** repeated `starting_entity_missing`, zero-entity extraction, LLM burst dwarfs conversational spend, extraction > conversational spend.

---

## Scope: session

Audit all turns in an agent's session — turn timeline, aggregate LLM/tool metrics, runtime logs, and red flags.

**ID format:** agent name or session ID (UUID). If agent name, resolve to latest session.

### Step 1: Find the session

**If agent name:**
```sql
SELECT s.id, s.agent_id, s.channel_type,
       substr(s.started_at,1,19) as started, substr(s.ended_at,1,19) as ended,
       s.metadata, s.task_id
FROM sessions s WHERE s.agent_id = '<agent_name>'
ORDER BY s.started_at DESC LIMIT 1
```

**If session ID:**
```sql
SELECT s.id, s.agent_id, s.channel_type,
       substr(s.started_at,1,19) as started, substr(s.ended_at,1,19) as ended,
       s.metadata, s.task_id
FROM sessions s WHERE s.id = '<session_id>'
```

If `ended_at` is NULL, note: **"Session in progress — metrics are incomplete."**

### Step 2: List all turns

```sql
SELECT m.trace_id,
       MIN(m.created_at) as turn_started, MAX(m.created_at) as turn_ended,
       SUM(CASE WHEN m.role = 'user' THEN 1 ELSE 0 END) as user_msgs,
       SUM(CASE WHEN m.role = 'assistant' THEN 1 ELSE 0 END) as assistant_msgs
FROM messages m
WHERE m.session_id = '<session_id>' AND m.trace_id IS NOT NULL
GROUP BY m.trace_id
ORDER BY MIN(m.created_at)
```

Orphan count: `SELECT COUNT(*) FROM messages WHERE session_id = '<session_id>' AND trace_id IS NULL`

If > 30 turns, show first 30 and note truncation.

### Step 3: Per-turn summary

Get turn content previews, per-turn LLM counts, per-turn tool counts (three queries scoped by `session_id`, grouped by `trace_id`).

### Step 4: Aggregate LLM calls

```sql
SELECT lc.provider, lc.model,
       COUNT(*) as call_count,
       SUM(lc.input_tokens) as total_input, SUM(lc.output_tokens) as total_output,
       SUM(lc.cache_read_tokens) as total_cache_read, SUM(lc.cache_write_tokens) as total_cache_write,
       SUM(lc.latency_ms) as total_latency_ms,
       SUM(CASE WHEN lc.status = 'error' THEN 1 ELSE 0 END) as error_count,
       GROUP_CONCAT(DISTINCT lc.stop_reason) as stop_reasons
FROM llm_calls lc WHERE lc.session_id = '<session_id>'
GROUP BY lc.provider, lc.model
```

**KG bucketing (v28+):** Run `shared:kg-precheck`, then `shared:kg-llm-bucketing` with `lc.session_id = '<session_id>'`.

Also get anomalous LLM calls (error, max_tokens, latency > 30s).

Compute: total tokens, cache efficiency, average latency, error rate.

### Step 5: Aggregate tool calls

Grouped by `tool_name, tool_source`. Plus failed tool calls and retry/loop detection. All scoped by `session_id`.

**KG tool usage (v28+):** Run `shared:kg-query-tool-usage` scoped by session_id.

### KG Health (v28+)

*Skip if pre-check returned < 28 or "No KG activity in scope."*

**Extraction coverage (per-corpus):**
```sql
WITH agent_corpora AS (
    SELECT docs_root_hash FROM agent_kg_corpora WHERE agent_id = '<agent_id>'
),
chunk_paths AS (
    SELECT DISTINCT docs_root_hash, source_doc_path FROM kg_chunks
    WHERE docs_root_hash IN (SELECT docs_root_hash FROM agent_corpora)
),
extracted_paths AS (
    SELECT DISTINCT docs_root_hash, source_doc_path FROM kg_extractions
    WHERE docs_root_hash IN (SELECT docs_root_hash FROM agent_corpora)
)
SELECT cp.docs_root_hash,
       COUNT(DISTINCT ep.source_doc_path) AS extracted,
       COUNT(DISTINCT cp.source_doc_path) AS total,
       printf('%.0f%%', 100.0 * COUNT(DISTINCT ep.source_doc_path) / NULLIF(COUNT(DISTINCT cp.source_doc_path), 0)) AS coverage_pct
FROM chunk_paths cp
LEFT JOIN extracted_paths ep ON cp.docs_root_hash = ep.docs_root_hash AND cp.source_doc_path = ep.source_doc_path
GROUP BY cp.docs_root_hash;
```

For multi-corpus agents, show per-corpus breakdown then average. Do NOT collapse to a single percentage.

**Extraction staleness:** Count rows with `source_doc_hash IS NULL` — should be 0 if v27 backfill completed. Non-zero = "v26 backfill incomplete."

**Resolution outcome distribution (window-scoped):**
```sql
SELECT outcome, COUNT(*) as count
FROM kg_resolutions_log
WHERE agent_id = '<agent_id>' AND resolved_at >= '<session_started_at>'
GROUP BY outcome
```

### Step 6: Audit events (data mutations)

```sql
SELECT substr(ae.created_at,1,19) as time, ae.trace_id, ae.tool_name, ae.target_key,
       substr(ae.before_value,1,100) as before_val,
       substr(ae.after_value,1,150) as after_val, ae.reasoning
FROM audit_events ae WHERE ae.session_id = '<session_id>'
ORDER BY ae.created_at
```

### Step 7: Check agent runtime logs

Run `shared:log-grep` with `<agent_id>`, session's `started_at`/`ended_at`.

### Step 8: Render output

```
## Session Audit: <agent_name>
**Session:** <session_id> | **Channel:** <channel_type> | **Task:** <task_id or "none">
**Duration:** <started_at> → <ended_at or "in progress"> (<elapsed>)
**Turns:** <count> | **Orphan messages:** <count>
```

Then: Turn Timeline, LLM Usage, Anomalous LLM Calls, Tool Usage, Failed Tool Calls, Retry/Loop Patterns, Data Mutations, KG Health (v28+), Agent Runtime Logs, Analysis.

### Session-scope red flags

**Session-level:** duration > 1h (non-team/non-claude-pilot), > 50 turns, orphan messages > 10%, abandoned (ended_at NULL, last turn > 30m ago).
**Token trajectory:** per-turn input steadily increasing, sudden spike, total > 500K, cache efficiency < 50%.
**LLM issues:** error status, max_tokens, latency > 30s, unexpected model switching, error rate > 10%.
**Tool issues:** success=0, non_zero_exit, retry loops (> 3 same step), failure rate > 20%, high latency (> 10s avg).
**Data integrity:** before_value mismatch, memory writes without reasoning, oscillating mutations.
**Runtime logs:** transient API errors, telemetry write failure, exceeded max tool steps, compacting, no log file.
**KG (v28+):** extraction coverage < 80% on any corpus, corpus-specific lag, query miss rate > 30%, extraction > conversational LLM spend, resolution outcomes skewed to error.

---

## Scope: turn

Audit an agent's last turn — message, tool calls, audit events, and runtime logs.

**ID format:** agent name (required). Always resolves to the latest assistant message.

### Step 1: Find the last assistant message and trace_id

```sql
SELECT m.id, m.session_id, m.trace_id, substr(m.created_at,1,19) as time, m.content
FROM messages m
WHERE m.agent_id = '<agent_name>' AND m.role = 'assistant'
ORDER BY m.created_at DESC LIMIT 1
```

Store `trace_id` — per-turn correlation key.

### Step 2: Get the full turn context by trace_id

Run in parallel:

**Messages:**
```sql
SELECT substr(m.created_at,1,19) as time, m.role, m.content, m.metadata
FROM messages m WHERE m.trace_id = '<trace_id>' ORDER BY m.created_at
```

**LLM calls:**
```sql
SELECT lc.id, substr(lc.created_at,1,19) as time, lc.step, lc.provider, lc.model,
       lc.input_tokens, lc.output_tokens, lc.cache_read_tokens, lc.cache_write_tokens,
       lc.latency_ms, lc.stop_reason, lc.status, lc.error_message
FROM llm_calls lc WHERE lc.trace_id = '<trace_id>'
ORDER BY lc.step, lc.created_at
```

**Tool calls:**
```sql
SELECT tc.id, substr(tc.created_at,1,19) as time, tc.tool_name, tc.tool_source, tc.skill_name,
       tc.step, tc.success, tc.non_zero_exit, tc.latency_ms,
       substr(tc.input,1,200) as input_preview, substr(tc.output,1,200) as output_preview,
       tc.error_message, tc.llm_call_id
FROM tool_calls tc WHERE tc.trace_id = '<trace_id>'
ORDER BY tc.step, tc.created_at
```

For any `query_knowledge_graph` tool call, inspect output for: `starting_entity_missing`, `fallback_reason`, `entities`/`edges` keys. Surface one-line summary per invocation.

**Audit events:**
```sql
SELECT substr(ae.created_at,1,19) as time, ae.tool_name, ae.target_key,
       substr(ae.before_value,1,100) as before_val, substr(ae.after_value,1,150) as after_val, ae.reasoning
FROM audit_events ae WHERE ae.trace_id = '<trace_id>'
ORDER BY ae.created_at
```

**KG side effects (v28+):** Run `shared:kg-precheck`. If schema >= 28:

Extractions:
```sql
SELECT source_doc_path, entities_extracted, relationships_extracted,
       extraction_model, docs_root_hash, substr(created_at,1,19) as time
FROM kg_extractions WHERE extraction_trace_id = '<trace_id>'
```

Resolutions:
```sql
SELECT subject_entity_id, outcome, model, duration_ms, substr(resolved_at,1,19) as time
FROM kg_resolutions_log
WHERE (resolution_trace_id = '<trace_id>' OR source_extraction_trace_id = '<trace_id>')
  AND agent_id = '<agent_name>'
```

If both return 0 rows: "No KG side effects in this turn."

### Step 3: Compute metrics

Per LLM call: total tokens, cache efficiency, latency (flag > 30s).
Per tool call: flag success=0 or non_zero_exit=1, link to LLM call via `llm_call_id`.

### Step 4: Check agent runtime logs

Run `shared:log-grep-by-trace` with `<agent_name>` and `<trace_id>`. Use the assistant message timestamp for the date file.

### Step 5: Identify the user message

From the messages query, find `role = 'user'` — the prompt that triggered the turn.

### Step 6: Check for engine-injected context

Look at `tool_calls.skill_name` for this turn. If a skill is found, check its system prompt for template variables:
```bash
find /data/workspace/senara-solutions/mika-platform/mika-skills/<skill_name>/ -name "system_prompt.md" -exec grep -oP '\{\{[^}]+\}\}' {} \;
```

If template variables exist, note as **Engine-Injected Context**. Token accounting hint: if first LLM call's `input_tokens` >> bare system prompt (~5-10K), the difference is likely injected context.

### Step 7: Render output

```
## Turn Audit: <agent_name>
**Time:** <timestamp> | **Session:** <session_id> | **Trace:** <trace_id>
```

Then: User Prompt, Agent Response, LLM Calls table, Tool Calls table, Data Mutations, KG Side Effects (v28+), Agent Runtime Logs, Engine-Injected Context, Analysis.

### Turn-scope red flags

**Filtering rule:** If a pattern is fully explained by engine-injected context (Step 6), it is NOT a red flag.

**Fabrication:** agent claimed actions but zero tool calls, tool output doesn't match claims, tool calls recorded but response ignores them. **Engine-injected caveat:** agent may reference data from injected context without tool calls — check template variables first.
**LLM issues:** error status, max_tokens, latency > 30s, low cache efficiency on subsequent calls, cache miss.
**Tool issues:** success=0, non_zero_exit, null skill_name on skill tools, high latency (> 10s), retry loops.
**Data integrity:** before_value mismatch, memory writes without reasoning.
**Runtime logs:** transient API error, telemetry write failure, exceeded max tool steps, compacting.
**KG (v28+):** > 2 `query_knowledge_graph` calls (iterating on misses), zero-entity extraction, resolution skewed to error.

---

## Scope: team

Audit a team run — agent utilization, LLM/tool telemetry, runtime logs, workspace timeline, model verification.

**ID format:** team name (e.g., `inner-circle`) or run ID (UUID).

### Step 1: Find the run

**If team name:** latest run by `started_at DESC`.
**If run ID:** direct lookup.

```sql
SELECT tr.id, tr.team_id, tr.goal, tr.status, tr.iteration, tr.max_iterations,
       substr(tr.started_at,1,19) as started, substr(tr.ended_at,1,19) as ended, tr.trace_id,
       substr(tr.deliverable,1,500) as deliverable_preview
FROM team_runs tr WHERE tr.<filter>
```

Store `id`, `team_id`, `trace_id`.

### Step 2: Team composition

```bash
cat ~/.mika/teams/<team_id>/team.toml
```

Extract: orchestrator name, agent list with roles and mandates.

### Step 3: Workspace timeline

```sql
SELECT tw.id, tw.agent_name, tw.entry_type, tw.iteration,
       substr(tw.created_at,1,19) as time,
       substr(tw.content,1,300) as content_preview
FROM team_workspace tw WHERE tw.run_id = '<run_id>'
ORDER BY tw.created_at
```

### Step 4: Per-agent session analysis

Find sessions:
```sql
SELECT s.id, s.agent_id, substr(s.started_at,1,19) as started, substr(s.ended_at,1,19) as ended
FROM sessions s
WHERE s.id LIKE 'team-<run_id>%'
ORDER BY s.started_at
```

**Note:** Team runs share a single `trace_id` across all agents. Use the run's `trace_id` rather than per-session filtering for comprehensive coverage.

For each session, query LLM calls:
```sql
SELECT lc.provider, lc.model,
       COUNT(*) as call_count,
       SUM(lc.input_tokens) as total_input, SUM(lc.output_tokens) as total_output,
       SUM(lc.cache_read_tokens) as total_cache_read,
       SUM(lc.latency_ms) as total_latency_ms,
       SUM(CASE WHEN lc.status = 'error' THEN 1 ELSE 0 END) as error_count,
       GROUP_CONCAT(DISTINCT lc.stop_reason) as stop_reasons
FROM llm_calls lc
WHERE lc.trace_id = '<run_trace_id>'
GROUP BY lc.provider, lc.model
```

Tool calls:
```sql
SELECT tc.tool_name, tc.tool_source, tc.skill_name,
       COUNT(*) as call_count,
       SUM(CASE WHEN tc.success = 1 THEN 1 ELSE 0 END) as successes,
       SUM(CASE WHEN tc.success = 0 THEN 1 ELSE 0 END) as failures,
       SUM(tc.latency_ms) as total_latency_ms
FROM tool_calls tc
WHERE tc.trace_id = '<run_trace_id>'
GROUP BY tc.tool_name, tc.tool_source
ORDER BY call_count DESC
```

Failed tool calls:
```sql
SELECT substr(tc.created_at,1,19) as time, tc.step, tc.tool_name, tc.tool_source,
       tc.skill_name, tc.error_message,
       substr(tc.input,1,200) as input_preview, substr(tc.output,1,200) as output_preview
FROM tool_calls tc
WHERE tc.trace_id = '<run_trace_id>'
  AND (tc.success = 0 OR tc.non_zero_exit = 1)
ORDER BY tc.created_at
```

**KG bucketing (v28+):** Run `shared:kg-precheck`, then `shared:kg-llm-bucketing` with `lc.trace_id = '<run_trace_id>'`.

### Step 5: Agent utilization

Compare team composition (Step 2) against actual sessions (Step 4). Agents with no session are **Unused**. Flag if > half non-orchestrator agents unused.

### KG Health (v28+)

*Skip if pre-check returned < 28 or no team member has `agent_kg_corpora` entries.*

**Tier 1 — Per-corpus extraction coverage:** Group by `docs_root_hash` first (one row per distinct corpus, eliminates duplicate-rows artifact where N agents sharing a corpus produce N copies). Use traffic-light glyph: 🟢 >= 80%, 🟡 50-79%, 🔴 < 50%.

```sql
WITH team_agents AS (
    SELECT DISTINCT s.agent_id FROM sessions s WHERE s.id LIKE 'team-<run_id>%'
),
team_corpora AS (
    SELECT DISTINCT akc.docs_root_hash, akc.agent_id
    FROM agent_kg_corpora akc
    WHERE akc.agent_id IN (SELECT agent_id FROM team_agents)
),
corpus_agents AS (
    SELECT docs_root_hash,
           COUNT(DISTINCT agent_id) AS agent_count,
           GROUP_CONCAT(DISTINCT agent_id) AS agents
    FROM team_corpora GROUP BY docs_root_hash
),
chunk_paths AS (
    SELECT DISTINCT kc.docs_root_hash, kc.source_doc_path
    FROM kg_chunks kc
    WHERE kc.docs_root_hash IN (SELECT docs_root_hash FROM team_corpora)
),
extracted_paths AS (
    SELECT DISTINCT ke.docs_root_hash, ke.source_doc_path
    FROM kg_extractions ke
    WHERE ke.docs_root_hash IN (SELECT docs_root_hash FROM team_corpora)
)
SELECT cp.docs_root_hash,
       ca.agent_count, ca.agents,
       COUNT(DISTINCT ep.source_doc_path) AS extracted,
       COUNT(DISTINCT cp.source_doc_path) AS total,
       printf('%.0f%%', 100.0 * COUNT(DISTINCT ep.source_doc_path) / NULLIF(COUNT(DISTINCT cp.source_doc_path), 0)) AS coverage_pct,
       (SELECT COUNT(*) FROM kg_extractions WHERE docs_root_hash = cp.docs_root_hash AND source_doc_hash IS NULL) AS stale_count
FROM chunk_paths cp
JOIN corpus_agents ca ON cp.docs_root_hash = ca.docs_root_hash
LEFT JOIN extracted_paths ep ON cp.docs_root_hash = ep.docs_root_hash
                             AND cp.source_doc_path = ep.source_doc_path
GROUP BY cp.docs_root_hash;
```

**Tier 2 — Per-agent resolution metrics:** Group by `agent_id`. Resolution stays per-agent because `kg_resolutions_log.agent_id` is preserved post-v27.

```sql
SELECT krl.agent_id,
       SUM(CASE WHEN krl.outcome = 'matched_exact' THEN 1 ELSE 0 END) AS matched_exact,
       SUM(CASE WHEN krl.outcome = 'matched_llm' THEN 1 ELSE 0 END) AS matched_llm,
       SUM(CASE WHEN krl.outcome = 'no_match' THEN 1 ELSE 0 END) AS no_match,
       SUM(CASE WHEN krl.outcome = 'error' THEN 1 ELSE 0 END) AS errors,
       COUNT(*) AS total
FROM kg_resolutions_log krl
WHERE krl.agent_id IN (SELECT DISTINCT s.agent_id FROM sessions s WHERE s.id LIKE 'team-<run_id>%')
  AND krl.resolved_at >= '<run_started_at>'
  AND krl.resolved_at <= '<run_ended_at_or_now>'
GROUP BY krl.agent_id
```

**`query_knowledge_graph` per agent:**
```sql
SELECT tc.agent_id,
       COUNT(*) as total_calls,
       SUM(CASE WHEN tc.output LIKE '%starting_entity_missing%' OR tc.output LIKE '%fallback_reason%' THEN 1 ELSE 0 END) as fallback_count
FROM tool_calls tc
WHERE tc.trace_id = '<run_trace_id>'
  AND tc.tool_name = 'query_knowledge_graph'
GROUP BY tc.agent_id
```

**Agent -> corpora mapping:**
```sql
SELECT agent_id, docs_root_hash, substr(created_at,1,19) as since
FROM agent_kg_corpora
WHERE agent_id IN (SELECT DISTINCT s.agent_id FROM sessions s WHERE s.id LIKE 'team-<run_id>%')
ORDER BY agent_id, created_at
```

**KG-originated LLM spend:**
```sql
WITH agent_corpora AS (
    SELECT agent_id, docs_root_hash FROM agent_kg_corpora
    WHERE agent_id IN (SELECT DISTINCT s.agent_id FROM sessions s WHERE s.id LIKE 'team-<run_id>%')
),
extraction_traces AS (
    SELECT ke.extraction_trace_id, ac.agent_id
    FROM kg_extractions ke
    JOIN agent_corpora ac ON ke.docs_root_hash = ac.docs_root_hash
)
SELECT
    lc.agent_id,
    CASE
        WHEN lc.trace_id IN (SELECT extraction_trace_id FROM extraction_traces WHERE agent_id = lc.agent_id) THEN 'extraction'
        WHEN lc.trace_id IN (SELECT resolution_trace_id FROM kg_resolutions_log WHERE agent_id = lc.agent_id) THEN 'resolution'
        ELSE 'conversational'
    END AS kind,
    lc.provider, lc.model, COUNT(*) AS calls, SUM(lc.input_tokens+lc.output_tokens) AS tokens
FROM llm_calls lc
WHERE lc.trace_id = '<run_trace_id>'
GROUP BY lc.agent_id, kind, lc.provider, lc.model
```

### Step 6: Model verification

Compare actual LLM call providers/models against per-agent config:
```bash
grep -E 'llm_provider|model' ~/.mika/agents/<agent_name>/config.toml 2>/dev/null
grep -E 'llm_provider|model' ~/.mika/config.toml 2>/dev/null
```

Flag: model mismatch, global fallback.

### Step 7: Check agent runtime logs

For ALL agents in the team, run `shared:log-grep-by-trace` with `<run_trace_id>`. If trace_id yields nothing, fall back to `shared:log-grep` with the run's time window. Merge entries cross-agent and sort by timestamp. Cap at 50 lines total.

Include per-agent log health: log file exists? size? warning/error count?

### Step 8: Callback chain

```sql
SELECT t.id, t.agent_id, t.trigger_type, t.action_type, t.label, t.status,
       substr(t.created_at,1,19) as created, t.parent_task_id, t.execution_trace_id
FROM tasks t
WHERE t.label LIKE 'team-agent-%'
  AND t.created_at >= (SELECT started_at FROM team_runs WHERE id = '<run_id>')
  AND t.created_at <= COALESCE((SELECT ended_at FROM team_runs WHERE id = '<run_id>'), '9999-12-31')
ORDER BY t.created_at
```

Callback delivery sessions:
```sql
SELECT m.session_id, m.agent_id, substr(m.created_at,1,19) as time, substr(m.content,1,200) as content
FROM messages m
WHERE m.session_id LIKE 'callback-%'
  AND m.agent_id IN (SELECT DISTINCT s.agent_id FROM sessions s WHERE s.id LIKE 'team-<run_id>%')
  AND m.created_at >= (SELECT started_at FROM team_runs WHERE id = '<run_id>')
  AND m.role = 'assistant'
ORDER BY m.created_at
```

Count: callback tasks, `send_message` calls, user-received messages. Flag callbacks > 1 per agent (indicates multiple delegation rounds with separate notifications).

### Step 9: Workspace files

```bash
ls -la ~/.mika/teams/<team_id>/workspace/<run_id>/ 2>/dev/null
ls -la ~/.mika/teams/<team_id>/workspace/<run_id>/.meta/ 2>/dev/null
```

Read key metadata: `goal.md`, `assignments.md`, `critic_feedback.md`, `deliverable.md`.

### Step 10: Render output

```
## Team Run Audit: <team_id>
**Run:** <id> | **Status:** <status> | **Iterations:** <iteration>/<max_iterations>
**Goal:** <goal (first 200 chars)>
**Duration:** <started> → <ended> (<elapsed>)
**Trace:** <trace_id>
```

Then: Team Composition, Workspace Timeline, LLM Usage, Tool Usage, Failed Tool Calls, Model Verification, KG Health (v28+), Agent Runtime Logs (cross-agent timeline + per-agent health), Callback Chain, Workspace Files, Deliverable Preview, Red Flags.

### Team-scope red flags

**Team dynamics:** > half agents unused, single agent delegated to, same agent multiple times.
**Model/provider:** LLM calls don't match per-agent config, all agents on same model when configs differ, expensive model for simple tasks.
**Callbacks:** multiple `send_message` per run, callback tasks not delivered.
**Cost & efficiency:** tokens > 500K, tool failure rate > 20%, LLM latency > 30s.
**Scope:** output beyond goal scope, workspace files larger than expected.
**Runtime logs:** missing log files, WARN/ERROR entries, exceeded max tool steps, large time gaps in cross-agent timeline, uneven log volume.
**KG (v28+):** one corpus < 80% while others at 100%, resolution errors skewed on one agent, same corpus configured for two agents with mismatched extraction activity, extraction > conversational LLM spend.

---

## Scope: milestone

Audit a milestone — composes `/mika-audit task` across a milestone's tickets, adds a prompt-variant matrix and orchestrator log roll-up.

**ID format:** milestone name (optional — defaults to next open milestone by earliest `due_on`).

### Step 1: Resolve the milestone and ticket list

**Milestone resolution:**

If argument present, use as milestone name. If omitted:
```bash
gh api /repos/senara-solutions/mika-platform/milestones?state=open \
  --jq 'sort_by(.due_on // "9999") | .[0].title'
```

If empty, exit with usage line.

**Ticket enumeration across all four repos (in parallel):**
```bash
gh issue list --milestone "<milestone>" --repo senara-solutions/mika-platform --state all --json number,title,state,labels --limit 100
gh issue list --milestone "<milestone>" --repo senara-solutions/mika           --state all --json number,title,state,labels --limit 100
gh issue list --milestone "<milestone>" --repo senara-solutions/mika-cloud     --state all --json number,title,state,labels --limit 100
gh issue list --milestone "<milestone>" --repo senara-solutions/mika-skills    --state all --json number,title,state,labels --limit 100
```

If all four return empty, exit with error. Also capture the milestone metadata (due date, description) from the `mika-platform` API response for use in the Step 5 summary header.

### Step 2: Per-ticket task resolution and delegation

For each `(repo, number)`:

**Resolve to task_id:**
```sql
SELECT id, agent_id, session_id, status,
       substr(created_at,1,19) as created, substr(updated_at,1,19) as updated
FROM tasks WHERE label LIKE '%<repo>#<number>%'
ORDER BY created_at DESC LIMIT 1
```

If no row, mark `not-dispatched`.

**Delegate:** For each ticket with a task_id, run `/mika-audit task <task_id>` via the Skill tool and capture its full markdown output. Do NOT re-run any of task-audit's SQL.

### Step 3: Variant matrix

For each task (skip not-dispatched):

**Resolve (provider, model) pairs:**
```sql
SELECT lc.provider, lc.model, COUNT(*) as n
FROM llm_calls lc JOIN messages m ON lc.trace_id = m.trace_id
WHERE m.session_id = '<session_id>'
GROUP BY lc.provider, lc.model ORDER BY n DESC
```

First row = dominant pair. > 1 row = "mixed variants" red flag.

**Resolve skill used:**
```sql
SELECT input
FROM tool_calls
WHERE tool_name = 'run_claude_pilot'
  AND trace_id IN (SELECT trace_id FROM messages WHERE session_id = '<session_id>')
ORDER BY created_at ASC
LIMIT 1
```

Parse the `input` JSON. Look for the skill name under the key `skill` first, then `skill_name`. If neither is present, fall back by agent role: `mika-dev` -> `self-dev`, `mika-qa` -> `qa-review`, any other -> `(inferred: <first token of agent_id>)`. Annotate the skill cell with `(inferred)` when the fallback was used.

**Note:** If the value stored in `llm_calls.model` ever contains a `/` (e.g. OpenRouter-style `provider/model` string recorded in the model column), the tuple path will not match and the fallback will be used. The sha256 column still surfaces that drift.

**Resolve prompt file path (two-level fallback):**
```bash
candidate="mika-skills/<skill>/generated/<provider>/<model>/system_prompt.md"
if [ ! -f "$candidate" ]; then candidate="mika-skills/<skill>/system_prompt.md"; fi
if [ ! -f "$candidate" ]; then candidate="(not found)"; fi
```

**Compute sha256:**
```bash
hash=$(sha256sum "$candidate" | cut -d' ' -f1 | cut -c1-16)
```

### Step 3b: KG utilization across the milestone (v28+)

Extract KG data from captured `/mika-audit task` outputs: query calls, fallback rate, compound extraction, LLM spend bucketing. If all task audits skipped KG, note: "No KG activity across the milestone."

### Step 4: Orchestrator log roll-up

**Milestone window:** `min(task.created)` -> `max(task.updated)` across all resolved tasks.

**mika-dev log scan:** For each log file in the window, count and extract top 5 ERROR/WARN entries. Also grep for KG-related spans (`"kg_"`).

Use `shared:log-grep` pattern for the scanning.

**Per-task claude-pilot session log:** Extract session_id from `run_claude_pilot` output, find matching log entries, produce one-line summary: `<repo>#<N>: duration=<mm:ss> exit=<status> errors=<n>`.

### Step 5: Render output

```
## Milestone Audit: <milestone>
**Due:** <due_date_or_none> | **Tickets:** <count> | **Window:** <start> → <end>
```

Then: Milestone Summary (by outcome/repo/wall-time), Per-Ticket Task Audits (captured outputs), Variant Matrix, KG Utilization (v28+), Orchestrator Roll-up, Red Flags.

### Milestone-scope red flags

**Variant:** mixed variants in one session, prompt hash mismatch across same-skill tasks, prompt file not found, model drift from calibration baseline.
**Orchestrator:** claude-pilot non-zero exit, mika-dev ERROR entries, exceeded max tool steps.
**Coverage:** not-dispatched tickets.
**KG (v28+):** average fallback rate > 30%, extraction triggered but zero entities, `kg_budget_exhausted`, same corpus hash double-counted across tickets.

---

## Scope: pr

Audit both sides of a PR — mika-dev's task that produced it and mika-qa's session that reviewed it.

**ID format:** PR reference — `https://github.com/senara-solutions/<repo>/pull/<N>` or `<repo>#<N>`.

### Step 1: Resolve the PR

Accept either form and parse `<repo>` and `<N>`.

```bash
gh pr view <arg> --repo senara-solutions/<repo> \
  --json number,title,state,author,createdAt,mergedAt,url,headRefName,baseRefName,mergeCommit
```

Store: repo, N, title, state, opened, merged, url, merge_sha.

If not found, exit with usage line.

### Step 2: Resolve the mika-dev task

```sql
SELECT id, substr(label,1,80) as label, status,
       substr(created_at,1,19) as created, substr(updated_at,1,19) as updated
FROM tasks
WHERE agent_id = 'mika-dev'
  AND (label LIKE '%<repo>#<N> %' OR label LIKE '%<repo>#<N>)%'
       OR label LIKE '%<repo>#<N>:%' OR label = '%<repo>#<N>'
       OR label LIKE '%<repo>#<N>'  ESCAPE '\\')
ORDER BY created_at DESC LIMIT 5
```

The multi-pattern match anchors `<repo>#<N>` to a word boundary so `mika#4` does NOT match tasks for `mika#443`, `mika#44`, etc. If the SQL dialect above is awkward, the equivalent check is "label contains `<repo>#<N>` followed by a non-digit character (or end of string)."

Pick the row with `created_at` closest to (but not > 24h after) `<opened>`. Fallback to most recent overall with annotation. Zero matches = **not-dispatched**.

### Step 3: Resolve the mika-qa session(s)

mika-qa sessions triggered by the PR-review webhook have `channel_type = 'github'` and `task_id = NULL` (verified 2026-04-15).

Candidate sessions:
```sql
SELECT id, substr(started_at,1,19) as started, substr(ended_at,1,19) as ended
FROM sessions
WHERE agent_id = 'mika-qa' AND channel_type = 'github'
  AND started_at >= '<opened>'
  AND started_at <= datetime('<merged or now>', '+24 hours')
ORDER BY started_at ASC
```

Filter to sessions referencing this PR:
```sql
SELECT DISTINCT m.session_id FROM messages m
WHERE m.session_id IN (<candidate_ids>)
  AND (m.content LIKE '%pull/<N>%' OR m.content LIKE '%<repo>#<N>%')
```

If broader pattern needed (`LIKE '%<url>%'`), document it. Zero matches = **not-reviewed**. Multiple matches = re-reviews.

### Step 4: Delegate to per-scope audits

- If `<dev_task_id>` set: run `/mika-audit task <dev_task_id>` via Skill tool, capture output.
- For each `<qa_session_id>`: run `/mika-audit session <qa_session_id>` via Skill tool, capture output.

If both sides missing, exit with explanation.

### Step 5: Render output

```
## PR Audit: <repo>#<N> — <title>
**State:** <state> | **Opened:** <opened> | **Merged:** <merged or "—"> | **URL:** <url>
```

**Comparison Summary table:** Side, Agent, Subject, Provider, Model, Outcome, Duration.

Then: Dev-Side Task Audit (captured output or "not-dispatched"), Qa-Side Session Audit(s) (captured outputs or "not-reviewed"), KG Usage in Review (v28+), Red Flags.

**KG Usage in Review (v28+):** Synthesize from sub-audits: dev-side extraction, qa-side queries, staleness check (dev extraction visible to qa?). If both sides reported no KG, omit.

### PR-scope red flags

**Dev side missing:** not-dispatched.
**Qa side missing:** not-reviewed.
**Dev and qa on different models:** cite both (informational).
**Multiple qa sessions:** count and list.
**Qa started before dev's final push:** compare timestamps.
**Time delta dev->qa > 1h:** webhook lag.
**Dev task status != 'delivered':** cite status.
**Qa session has ERROR LLM calls.**
**Qa verdict contradicts PR merge state:** e.g., changes_requested but merged.
**Dev extraction not visible to qa:** extraction fired but qa shows zero new chunks for same corpus.
**Qa never queried KG:** informational.

---

## Scope: dev-run

Audit a full dev run — task tree, claude-pilot session, mika-dev callback handling, mika-qa review, and pipeline outcome.

**ID format:** task ID (UUID, optional — defaults to latest mika-dev self_dev task). If first token is not a UUID, treat entire argument as observation and resolve latest work item.

### Step 1: Resolve the work item

**If task ID:**
```sql
SELECT id, label, status, source, metadata, substr(created_at,1,19) as created
FROM tasks WHERE id = '<task_id>'
```

**If no task ID:**
```sql
SELECT id, label, status, source, metadata, substr(created_at,1,19) as created
FROM tasks
WHERE agent_id = 'mika-dev' AND trigger_type = 'manual' AND source = 'self_dev'
ORDER BY created_at DESC LIMIT 1
```

Parse metadata JSON for: `claude_pilot` fields (session_id, cost_usd, duration_ms, turns, branch, pr_url), `qa_retry_count`.

Quick summary:
```
## Dev Run: <label>
**Task:** <id> | **Status:** <status> | **Created:** <timestamp>
**PR:** <pr_url or "none"> | **Branch:** <branch or "unknown">
**claude-pilot:** <turns> turns | $<cost> | <duration>
```

### Step 2: Run sub-audits in parallel

Launch via the Skill tool **in parallel** (single message, multiple tool calls):

1. `/mika-audit task <task_id>` — task tree, LLM/tool telemetry, claude-pilot logs, PR
2. `/mika-audit turn mika-dev` — callback handling and webhook-triggered turns
3. `/mika-audit turn mika-qa` — QA review turn

If observation provided, append to each sub-audit command.

### Step 3: Synthesize the pipeline narrative

**Pipeline Timeline:** Build from sub-audit data:

| # | Stage | Time | Duration | Agent | Status |
|---|-------|------|----------|-------|--------|
| 1 | Issue read + work item created | | | mika-dev | |
| 2 | claude-pilot launched | | | mika-dev | |
| 3 | claude-pilot running | | | claude-pilot | |
| 4 | Callback received | | | mika-dev | |
| 5 | Vincent notified + close-out | | | mika-dev | |
| 6 | PR created -> webhook fired | | | gateway | |
| 7 | mika-qa review | | | mika-qa | |
| 8 | PR review posted -> webhook fired | | | gateway | |
| 9 | Verdict received | | | mika-dev | |
| 10 | Verdict handled | | | mika-dev | |

Stages 6-10 may show "pending" if audit runs before cycle completes.

**Cross-Agent Verdict Check:** Compare mika-qa's posted verdict against mika-dev's action.

| Verdict | Expected action | Actual | Match? |
|---------|----------------|--------|--------|
| pass | merge or notify | ? | ? |
| hold[review] | claude-pilot fix | ? | ? |
| block[ci] | claude-pilot fix | ? | ? |
| block[security/pipeline] | notify Vincent | ? | ? |

**Aggregate Metrics:**
```
### Totals
- **Total cost:** $<sum of all agents>
- **Total duration:** <end - start>
- claude-pilot: <turns> turns, $<cost>, <duration>
- mika-dev (post-callback): <llm/tool calls>
- mika-qa: <llm/tool calls>
- KG ingestion: <tokens> (from sub-audit bucketing)
- KG queries: <N> calls, <M>% fallback rate
```

If leaf audits skipped KG, omit KG lines. Surface corpus hash when extraction reported.

### Dev-run-scope red flags

**Webhook:** PR created but qa never received event, qa ran but didn't post review, verdict not received by mika-dev, verdict mismatch.
**Close-out:** work item status not updated, no metadata, missing notification.
**Retry:** qa_retry_count or ci_fix_count > 2 without escalation.
**Cost:** claude-pilot > $20 or total > $25 for routine task.
**KG:** query_knowledge_graph returned starting_entity_missing during dev, extraction exceeded batch budget, compound doc not ingested.
**Stale data:** work item metadata missing fields.

---

## Reference

All scopes are accessed via `/mika-audit <scope>`:
- `/mika-audit task <agent_name | task_id>` — task tree, LLM/tool telemetry, claude-pilot logs, PR, pipeline artifacts
- `/mika-audit session <agent_name | session_id>` — turn timeline, aggregate metrics, runtime logs
- `/mika-audit turn <agent_name>` — last turn deep dive: message, tool calls, audit events
- `/mika-audit team <team_name | run_id>` — team run: agent utilization, workspace timeline, model verification
- `/mika-audit milestone [milestone_name]` — milestone audit: per-ticket task audits, variant matrix, orchestrator roll-up
- `/mika-audit pr <pr_url | repo#N>` — PR audit: dev-side task + qa-side session, comparison summary
- `/mika-audit dev-run [task_id]` — full autonomous run: pipeline timeline, cross-agent verdict check, aggregate metrics

Related commands:
- `/mika` — the quality pipeline (plan -> work -> review -> compound -> PR)
- `/mika-sprint` — dispatch a sprint to mika-dev
- `/mika-groom-ticket` — groom a ticket with mika-arch
