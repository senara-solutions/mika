# Plan: Layer 1 — Task Ledger: canonical OperationalItem schema + write paths

**Ticket:** mika issue#1262
**Foundation:** `docs/architecture/operational-partner-frame.md` (merged via mika#1265)
**Type:** feat
**Branch:** `feat/1262/project-operational-partner-foundation`

---

## Summary

Implement the `operational_items` SQLite table (schema v38→v39), the Rust domain types, the write paths from all 7 operationally-relevant subsystems, and the read API (`OperationalItem::query()`). Gated behind `MIKA_OPERATIONAL_PARTNER=1` feature flag for reads; writes are always-on once deployed.

---

## Phase 1: Schema & Domain Types

### 1.1 Migration v38→v39

Add `operational_items` table:

```sql
CREATE TABLE operational_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
    owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
    owner_name TEXT,  -- NULL for user/mika, populated for person/agent
    priority REAL NOT NULL DEFAULT 0.0,
    user_importance REAL NOT NULL DEFAULT 0.0,
    due_at TEXT,  -- ISO 8601
    blocked_by TEXT,  -- FK to operational_items.id (not enforced, for soft reference)
    next_action TEXT,  -- FK to operational_items.id
    evidence_refs TEXT,  -- JSON array of {kind, id} objects
    confidence REAL NOT NULL DEFAULT 1.0,
    source_table TEXT,  -- originating table name (messages, tasks, tool_calls, etc.)
    source_id TEXT,  -- originating row id
    agent_id TEXT NOT NULL,  -- agent that wrote this item
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
CREATE UNIQUE INDEX idx_operational_items_source_unique ON operational_items(agent_id, source_table, source_id) WHERE source_table IS NOT NULL AND source_id IS NOT NULL;
```

The `source_table`/`source_id` pair enables deduplication — a given source row produces at most one `OperationalItem` per agent. The unique partial index enforces this at the DB level.

### 1.2 Rust Types

New module: `crates/mika-agent/src/operational/mod.rs` (with submodules).

```rust
// crates/mika-agent/src/operational/mod.rs
pub mod types;
pub mod write;
pub mod query;
pub mod calibration;

// crates/mika-agent/src/operational/types.rs
pub enum OperationalKind { Goal, Task, Commitment, Decision, Blocker, Evidence, NextAction }
pub enum OperationalStatus { Now, Waiting, Delegated, Scheduled, AtRisk, Done }
pub enum Owner { User, Mika, Person(String), Agent(String) }
pub struct EvidenceRef { pub kind: EvidenceRefKind, pub id: String }
pub enum EvidenceRefKind { OperationalItem, Message, ToolCall, GithubIssue, GithubPr, File, External }

pub struct OperationalItem {
    pub id: String,
    pub kind: OperationalKind,
    pub title: String,
    pub status: OperationalStatus,
    pub owner: Owner,
    pub priority: f32,
    pub user_importance: f32,
    pub due_at: Option<String>,
    pub blocked_by: Option<String>,
    pub next_action: Option<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub confidence: f32,
    pub source_table: Option<String>,
    pub source_id: Option<String>,
    pub agent_id: String,
    pub created_at: String,
    pub updated_at: String,
}

pub struct NewOperationalItem { /* same fields minus id, created_at, updated_at */ }
pub struct OperationalItemFilter {
    pub agent_id: String,
    pub kind: Option<OperationalKind>,
    pub status: Option<OperationalStatus>,
    pub owner_type: Option<String>,
    pub limit: Option<u32>,
    pub sort_by_priority: bool,
}
```

### 1.3 Calibration Constants

Per Decision C: `crates/mika-agent/src/operational/calibration.rs`

```rust
pub const COMMITMENT_WEIGHT_USER: f32 = 50.0;
pub const COMMITMENT_WEIGHT_MIKA: f32 = 35.0;
pub const COMMITMENT_WEIGHT_THIRD_PARTY: f32 = 20.0;
pub const STALE_TIME_CAP: f32 = 30.0;
pub const STALE_TIME_MULTIPLIER: f32 = 5.0;
pub const DEPENDENCY_RISK_PER_BLOCKED: f32 = 10.0;
pub const DEPENDENCY_RISK_CAP: f32 = 40.0;
pub const CONFIDENCE_PENALTY_MULTIPLIER: f32 = 50.0;
pub const URGENCY_MAX: f32 = 100.0;
pub const USER_IMPORTANCE_MAX: f32 = 50.0;
```

These constants are Layer 2's concern for runtime computation, but defining them here ensures Layer 1 stores the right fields and Layer 2 can consume them without migration.

---

## Phase 2: DB Methods

New file: `crates/mika-agent/src/db/operational.rs` (following the `kg_schema.rs` pattern for DB module separation).

Methods on `Database`:

- `insert_operational_item(&self, item: &NewOperationalItem) -> Result<String>` — INSERT with UUID generation, returns id.
- `upsert_operational_item_by_source(&self, item: &NewOperationalItem) -> Result<String>` — INSERT OR REPLACE keyed on `(agent_id, source_table, source_id)`. This is the primary write-path method — idempotent for re-delivery.
- `update_operational_item_status(&self, id: &str, status: OperationalStatus) -> Result<()>` — status transition with `updated_at` bump.
- `update_operational_item_priority(&self, id: &str, priority: f32) -> Result<()>` — priority cache update (Layer 2 will call this).
- `query_operational_items(&self, filter: &OperationalItemFilter) -> Result<Vec<OperationalItem>>` — the canonical read query.
- `get_operational_item_by_source(&self, agent_id: &str, source_table: &str, source_id: &str) -> Result<Option<OperationalItem>>` — lookup for status-update paths.
- `count_blocked_items(&self, blocked_by_id: &str) -> Result<u32>` — for dependency_risk scoring.

All methods wrapped via `AsyncDatabase` channel dispatch following existing patterns.

---

## Phase 3: Write Path Module

New file: `crates/mika-agent/src/operational/write.rs`

A single entry-point function per subsystem, each taking the source data + `&Database` + `agent_id`:

### 3.1 Chat → Task/Goal/Commitment/Decision/Blocker

**Not implemented in Layer 1.** The foundation doc §7 shows these write paths require LLM classification (distinguishing Task from Goal from Commitment from Decision from Blocker in user text). Layer 1 implements the schema and the write API; the classification step is Layer 2's concern (the What's Next engine includes the LLM-as-classifier). Layer 1 provides the `insert_operational_item` method that Layer 2's classifier will call.

**Rationale:** Decision D in the foundation doc says "single transaction in v1" — but the transaction atomicity is between the source-of-truth write and the OperationalItem write. For chat messages, the source-of-truth write is `save_message()`. The classification that determines *which kind* of OperationalItem to write requires LLM inference, which cannot run inside the same DB transaction. The write paths that don't need classification (subsystems 3.2–3.7 below) do run atomically.

### 3.2 Reminders → Task

Hook: `create_task()` in `db.rs` when `trigger_type = 'reminder'` or `action_type = 'send_message'`/`'resume_agent'` with `next_fire_at` set.

```rust
pub fn write_reminder_item(db: &Database, agent_id: &str, task: &Task) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: extract_reminder_label(&task.label),
        status: OperationalStatus::Scheduled,
        owner: Owner::User,
        due_at: task.next_fire_at.clone(),
        confidence: 1.0,  // explicit user action
        source_table: Some("tasks".to_string()),
        source_id: Some(task.id.clone()),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}
```

### 3.3 GitHub Webhooks → Task/Status Updates

Hook: `server/mod.rs` webhook handlers (ready-label, PR review, CI status).

- **Ready label:** Create `kind=Task, status=Now, owner=Agent("mika-dev")`.
- **PR review submitted:** Find existing item by `source_table="github_pr", source_id=<pr_url>`. Update status to `Waiting` (pending operator response).
- **CI status (failure):** Find existing item by source. Update status to `AtRisk`.
- **CI status (success):** Find existing item. Update status to `Now` (unblocked).

```rust
pub fn write_github_webhook_item(db: &Database, agent_id: &str, event: &WebhookEvent) -> Result<()> {
    match event {
        WebhookEvent::IssueLabeled { issue_number, repo, .. } => { /* kind=Task, status=Now */ }
        WebhookEvent::PrReview { pr_url, .. } => { /* find and update status */ }
        WebhookEvent::CheckSuite { pr_url, conclusion, .. } => { /* find and update status */ }
    }
    Ok(())
}
```

### 3.4 Skills (auto-groom, dispatch) → Delegation

Hook: `skills/executor.rs` after successful `validate_dispatch_readiness()` + subprocess spawn.

```rust
pub fn write_dispatch_item(db: &Database, agent_id: &str, task: &Task, skill_name: &str) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: format!("Dispatched: {} (task {})", skill_name, &task.id[..8]),
        status: OperationalStatus::Delegated,
        owner: Owner::Agent(derive_delegate_agent(skill_name)),
        confidence: 1.0,
        source_table: Some("tasks".to_string()),
        source_id: Some(task.id.clone()),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}
```

### 3.5 Team Runs → Delegated Task

Hook: `teams/mod.rs` at team-run start.

```rust
pub fn write_team_run_item(db: &Database, agent_id: &str, run: &TeamRun) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: format!("Team run: {}", &run.goal),
        status: OperationalStatus::Delegated,
        owner: Owner::Agent("team".to_string()),
        confidence: 1.0,
        source_table: Some("team_runs".to_string()),
        source_id: Some(run.id.clone()),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}
```

### 3.6 Callbacks (claude-pilot completion) → Status Transitions

Hook: `task_engine/dispatcher.rs` in `handle_task_complete()` after `try_extract_callback_metadata()`.

```rust
pub fn write_callback_completion(db: &Database, agent_id: &str, task: &Task, has_pr_url: bool) -> Result<()> {
    if let Some(item) = db.get_operational_item_by_source(agent_id, "tasks", &task.id)? {
        let new_status = if has_pr_url {
            OperationalStatus::Done
        } else {
            OperationalStatus::AtRisk
        };
        db.update_operational_item_status(&item.id, new_status)?;
    }
    Ok(())
}
```

### 3.7 Manual Task Creation (create_task tool) → Task

Hook: `tools/create_task.rs` after successful task insert.

```rust
pub fn write_manual_task_item(db: &Database, agent_id: &str, task: &Task) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: task.label.clone(),
        status: OperationalStatus::Now,
        owner: Owner::User,
        source_table: Some("tasks".to_string()),
        source_id: Some(task.id.clone()),
        confidence: 1.0,
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}
```

---

## Phase 4: Read API

### 4.1 Rust Query API

`crates/mika-agent/src/operational/query.rs`:

```rust
impl OperationalItem {
    pub fn query(db: &Database, filter: &OperationalItemFilter) -> Result<Vec<OperationalItem>> {
        db.query_operational_items(filter)
    }
}
```

### 4.2 HTTP Endpoint (gated behind feature flag)

New endpoint in `server/`: `GET /api/v1/operational-items`

- Query params: `kind`, `status`, `limit` (default 20, max 100), `sort` (priority_desc | created_at_desc)
- Auth: Dashboard token (read-only)
- Feature gate: `MIKA_OPERATIONAL_PARTNER=1` — returns 404 when disabled

### 4.3 A2A Endpoint (future, out of Layer 1 scope)

Documented as a stub in the module. Layer 2 will wire this up when the A2A surface needs operational state.

---

## Phase 5: Feature Flag & Integration

### 5.1 Feature Flag

`MIKA_OPERATIONAL_PARTNER` in `Settings`:
- **Writes:** Always-on once the migration lands. Every write path fires regardless of flag. This ensures the ledger is populated from day one.
- **Reads:** Gated. The HTTP endpoint returns 404, the agent-prompt injection (Layer 2) is skipped, the CLI commands (`mika next`, `mika status` operational view) are hidden.

### 5.2 Write-Path Atomicity (Decision D)

Per the foundation doc, writes are single-transaction with the source-of-truth table. For each hook site:

- **Reminders, manual tasks:** The `create_task()` call and the `upsert_operational_item_by_source()` call run inside the same `rusqlite::Transaction`. This requires adding a `Transaction`-accepting variant or using the existing `with_transaction` pattern.
- **Webhooks, callbacks, team runs:** These already run inside the `AsyncDatabase` actor's closure. The operational write is appended to the same closure.
- **Failure mode:** If the operational write fails, log a warning and continue. The source-of-truth write must not be blocked by a failure in the augmentation layer.

### 5.3 mika#1258 Sequencing (Decision D consideration)

The foundation doc notes Layer 1 and mika#1258 (async_db backpressure) should land together. However, mika#1258 is still open and its fix (DB-as-actor pattern) is independent of Layer 1's schema. **Decision:** Layer 1 lands first with today's `sync_channel(512)` pattern. The operational writes are small (one INSERT per event) and won't meaningfully increase backpressure. When mika#1258 ships DB-as-actor, the operational writes route through it cleanly — the `with_db` closure interface is unchanged.

---

## Phase 6: Testing

### 6.1 Unit Tests

In `crates/mika-agent/src/operational/`:
- `types.rs` — serialization/deserialization round-trip for each enum variant.
- `write.rs` — each write-path function tested with one creation case and one update case.

### 6.2 Integration Tests

New file: `crates/mika-agent/tests/eval/operational_ledger.rs`

- **Per write path (7 × 2 = 14 tests):** Each subsystem's write path tested with:
  1. Positive case: operationally-relevant event → item created with correct kind/status/owner.
  2. Update case: subsequent event on same source → status transitions correctly (not duplicate).
- **Idempotency:** Same event delivered twice → single item (unique index enforced).
- **Cross-path:** Reminder created → callback completes → status transitions from Scheduled → Done.

### 6.3 Migration Test

Schema migration v38→v39 tested via the existing migration test pattern (in-memory DB, verify table/index/constraint existence).

---

## Phase 7: Documentation

- Update `CLAUDE.md` § "Database" with v38→v39 migration entry.
- Update `CLAUDE.md` § "Schema Version" with `operational_items` table.
- Add `docs/architecture/operational-items-write-paths.md` documenting each hook site for future maintainers.

---

## Deliverables Checklist (mapped to AC)

| AC | Deliverable | Phase |
|----|-------------|-------|
| AC1 | Foundation doc merged | ✅ Done (mika#1265) |
| AC2 | Migration v38→v39, no behavior change to existing tables | Phase 1 |
| AC3 | All 7 write-path subsystems updated | Phase 3 (note: chat classification deferred to Layer 2) |
| AC4 | Regression tests per write path | Phase 6 |
| AC5 | Read API via `OperationalItem::query()` | Phase 4 |
| AC6 | CLAUDE.md documentation updated | Phase 7 |
| AC7 | Architect review | This grooming process |

---

## Open Questions for Architect

1. **Chat write path deferral:** The plan defers LLM-based classification of user messages (Task vs Goal vs Commitment vs Decision vs Blocker) to Layer 2. Is this acceptable, or should Layer 1 include a simple heuristic classifier?

2. **Evidence kind as write path:** The foundation doc lists `kind=Evidence` writes from `agent_loop/` and `tool_execution/`. These are high-frequency (every tool call could produce an Evidence item). Should Layer 1 include these, or defer to Layer 2 when the scoring formula needs evidence density?

3. **NextAction ephemeral writes:** Foundation doc §7 says NextAction items "may be replaced on next computation." Should Layer 1 implement a `replace_next_actions()` method that deletes stale NextActions, or is this purely Layer 2?

---

## Risk Assessment

- **Low risk:** Schema is additive (new table, no existing table changes). Feature-flagged reads. Writes are fire-and-forget with log-and-continue on failure.
- **Medium risk:** 7 hook sites across the codebase — each is a small change, but the surface area is broad. Mitigated by the fire-and-forget pattern and per-path unit tests.
- **Dependency:** None blocking. mika#1258 is a post-hoc optimization, not a prerequisite.
