---
title: "Background Agent Mode Design Checklist"
date: 2026-03-03
category: code-review-patterns
severity: high
components:
  - mika-agent
  - agent-loop
  - tools
  - database-schema
  - scheduler
  - prompt-engineering
tags:
  - reflection-system
  - background-agents
  - schema-drift
  - missing-indexes
  - unbounded-queries
  - code-duplication
  - disk-io-optimization
  - documentation-maintenance
related_prs:
  - "#59"
time_to_resolve: "~1 hour (13 issues resolved in 2 parallel waves)"
impact: "Prevents runtime errors, unbounded memory growth, audit gaps, and performance degradation in background agent modes"
---

# Background Agent Mode Design Checklist

## Problem Statement

During code review of PR #59 ("feat: add periodic memory reflection system"), 13 issues were identified across the new background reflection agent feature. The reflection system runs as a nightly silent agent loop that reviews the day's conversations, produces memory edits, and records an audit trail.

The issues ranged from **P1 contract violations** (LLM calls tools without a required field because the field was absent from the JSON schema) to **P2 operational gaps** (unbounded query results, no table pruning, disk I/O on every scheduler tick) to **P3 polish items** (duplicated logic, magic numbers, imprecise prompt language).

These issues form a **predictable pattern** — the same categories will recur every time a new `SilentTrigger` variant is added (e.g., `WeeklySummary`, `MorningBriefing`).

## Root Cause Analysis

### 1. Schema-Prompt Drift (2 issues)

The reflection prompt required an `evidence` field in every memory-editing tool call. The enforcement logic existed in `execute()` but the field was never declared in each tool's `input_schema`. The LLM's JSON generation was unconstrained — it would never see `evidence` in its available-fields list.

Separately, the prompt told the agent to "remove stale information" implying a delete operation, but no `delete_fact` tool exists. The correct approach is marking commitments cancelled via `update_fact` or storing consolidated replacements via `store_fact`.

Both issues stem from prompt and schema being written independently with no systematic cross-check.

### 2. Missing Defensive Patterns (4 issues)

Four defensive gaps were introduced because the reflection feature added new query paths without applying safety patterns already present elsewhere:

- `get_conversations_since` had no `LIMIT` clause — unbounded result sets
- `conversations` table had no index on `created_at` — full table scan
- Memory events digest had no size cap — arbitrarily large prompt injection
- `reflection_runs` table had no pruning — unlike `heartbeat_sends` (7-day) and `memory_events` (90-day)

### 3. Code Duplication (2 issues)

The midnight UTC boundary computation was copy-pasted into 4 call sites: `count_heartbeat_sends_today`, `last_reflection_run_today`, `check_and_fire_reflection`, and the reflection digest prep in `agent.rs`. The evidence validation block was copied identically into 3 tool `execute()` methods.

### 4. Performance Oversight (1 issue)

`check_and_fire_reflection` called `load_identity_async` on every 60-second poll tick — reading and parsing `identity.toml` from disk each time. The identity file changes only when the user explicitly reconfigures. The same pattern was already avoided for reminders (config loaded once at construction).

### 5. Documentation Drift (4 issues)

`CLAUDE.md` schema version not updated (9 → 10), magic number `50_000` unnamed, edit cap prompt said "5 memory edits" when only core memory edits are capped, and evidence check duplicated with no helper.

## Solution

### Schema-Prompt Alignment

Added `evidence` field to `input_schema` for all 3 memory tools:

```json
"evidence": {
    "type": "string",
    "description": "Required in reflection mode: cite a specific conversation timestamp and quote as justification"
}
```

Rewrote the reflection prompt to only reference available tools:

```rust
// Before: references nonexistent delete capability
"1. HOUSEKEEPING: ...Remove stale information that's no longer relevant.\n\n"

// After: references real tool capabilities
"1. HOUSEKEEPING: ...using update_fact (mark stale commitments as cancelled)\n\
   or store_fact (store consolidated versions of fragmented information).\n\n"
```

Added explicit "Available tools" section listing `update_core_memory`, `store_fact`, `update_fact`, `search_memory`.

### Defensive Query Patterns

```sql
-- Added index in v10 migration
CREATE INDEX IF NOT EXISTS idx_conversations_created_at ON conversations(created_at);

-- Added LIMIT to prevent unbounded result sets
SELECT ... FROM conversations WHERE created_at >= ?1 ... ORDER BY id LIMIT 500
```

Memory events digest gained the same truncation pattern as conversations:

```rust
if buf.len() + line.len() > MAX_REFLECTION_DIGEST_CHARS {
    buf.push_str("... (truncated)\n");
    break;
}
```

Pruning added at scheduler startup:

```rust
pub fn prune_old_reflection_runs(&self, days: u32) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days as i64 * 86_400);
    Ok(self.conn.execute("DELETE FROM reflection_runs WHERE ran_at < ?1", [cutoff])?)
}
```

### Extracted Utilities

**Midnight computation** — deduplicated from 4 sites to 1:

```rust
/// Compute today's midnight in the given timezone, converted to UTC.
pub fn today_midnight_utc(timezone: &str) -> chrono::DateTime<chrono::Utc> {
    let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let now_local = Utc::now().with_timezone(&tz);
    let local_midnight = NaiveDate::from_ymd_opt(now_local.year(), now_local.month(), now_local.day())
        .unwrap_or_else(|| Utc::now().date_naive())
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid");
    local_midnight.and_local_timezone(tz).earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}
```

**Evidence check** — deduplicated from 3 tool files to 1 helper:

```rust
pub(crate) fn check_reflection_evidence(ctx: &ToolContext<'_>, input: &Value) -> Option<ToolOutput> {
    if ctx.is_reflection {
        let evidence = input["evidence"].as_str().unwrap_or("").trim();
        if evidence.is_empty() {
            return Some(ToolOutput::error(
                "Reflection mode requires an evidence field citing specific conversation content.",
            ));
        }
    }
    None
}
```

### Cached Config

`ReminderScheduler` gained a `reflection_config: Option<ReflectionConfig>` field, loaded once at construction. The 60s poll no longer touches disk:

```rust
// Before: disk read every tick
let identity = crate::prompt::load_identity_async(&self.home_dir).await;

// After: cached field
let config = match self.reflection_config { Some(ref c) if c.enabled => c, _ => return };
```

## Checklist: Adding a New Background Agent Mode

Use this checklist when adding a new `SilentTrigger` variant (e.g., `WeeklySummary`, `MorningBriefing`):

### Schema & Database
- [ ] New tables have indexes on all filter columns (`created_at`, `status`, etc.)
- [ ] All new queries include `LIMIT` clause (default: 500)
- [ ] New tables have pruning strategy defined and called in `recover()`
- [ ] Schema version bumped in both migration code and `CLAUDE.md`

### Prompt & Tool Contract
- [ ] Every tool name mentioned in prompt exists in `ToolRegistry`
- [ ] Every runtime-validated field is declared in tool `input_schema`
- [ ] Prompt does not reference capabilities no tool provides
- [ ] Edit/rate limits in prompt match actual enforcement logic

### Code Quality
- [ ] Check for existing utilities before writing: `today_midnight_utc()`, `check_reflection_evidence()`
- [ ] Digest/prompt builders have size caps (`MAX_REFLECTION_DIGEST_CHARS`)
- [ ] Named constants for magic numbers with doc comments
- [ ] No config file I/O in polling loops — cache at construction

### Audit & Observability
- [ ] Evidence/reasoning persisted in audit log for all memory-mutating tools
- [ ] `tracing::info!` on trigger fire with session_id
- [ ] `tracing::warn!` on trigger failure with error context

### Documentation
- [ ] `CLAUDE.md` schema version updated
- [ ] `CLAUDE.md` describes new trigger in "Silent mode agent loop" section
- [ ] New config options documented in "Environment Variables" section

## Anti-patterns to Watch For

| Anti-pattern | Example | Fix |
|---|---|---|
| **Schema-prompt drift** | Tool validates `evidence` but schema doesn't declare it | Always edit schema and prompt in same commit |
| **Unbounded queries** | `SELECT * FROM conversations WHERE created_at >= ?1` | Add `LIMIT`, add index on filter column |
| **Table without pruning** | `reflection_runs` grows forever | Define retention policy, add prune call in `recover()` |
| **Config I/O in loop** | `load_identity_async()` every 60s | Cache at construction, reload only on explicit trigger |
| **Duplicated time logic** | Midnight computation in 4 files | Extract to `today_midnight_utc()` utility |
| **Prompt capability mismatch** | "delete facts" but no `delete_fact` tool | List available tools explicitly in prompt |
| **Magic numbers** | `50_000` with no name | Named constant with token-budget rationale |

## Key Code Changes

| File | Change |
|---|---|
| `crates/mika-agent/src/db.rs` | Extracted `today_midnight_utc()`; added index + LIMIT + `prune_old_reflection_runs()` |
| `crates/mika-agent/src/agent.rs` | Named `MAX_REFLECTION_DIGEST_CHARS`; capped memory digest; fixed prompt |
| `crates/mika-agent/src/scheduler.rs` | Cached `reflection_config`; added pruning call; removed redundant casts |
| `crates/mika-agent/src/tools/mod.rs` | Added `check_reflection_evidence()` helper |
| `crates/mika-agent/src/tools/store_fact.rs` | Added evidence to schema + audit log |
| `crates/mika-agent/src/tools/update_fact.rs` | Added evidence to schema + audit log |
| `crates/mika-agent/src/tools/update_core_memory.rs` | Added evidence to schema |
| `crates/mika-agent/src/async_db.rs` | Added `prune_old_reflection_runs()` async wrapper |
| `crates/mika-agent/src/server/mod.rs` | Populated `reflection_config` at server init |
| `crates/mika-cli/src/commands/chat.rs` | Populated `reflection_config` at CLI init |
| `CLAUDE.md` | Updated schema version to 10 |

## Cross-References

- **PR:** #59 (feat: add periodic memory reflection system)
- **Commits:** `9f1c3e2` (feature), `cf5088d` (review fixes)
- **Plan:** `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md`
- **Brainstorm:** `docs/brainstorms/2026-03-03-periodic-memory-reflection-brainstorm.md`
- **Related:** `docs/solutions/runtime-errors/reminders-never-fire-at-scheduled-time.md` (same scheduler pattern)
- **Related:** `docs/solutions/database-issues/sqlite-datetime-format-mismatch.md` (v9 migration precedent)
- **ADRs:** `docs/adr/001-axum-http-server-architecture.md` (agent_lock pattern)
- **Review findings:** `todos/411-complete-*` through `todos/423-complete-*` (13 individual findings)
