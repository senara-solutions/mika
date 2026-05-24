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
- `update_operational_item_status(&self, id: &str, status: NonTerminalStatus) -> Result<()>` — status transition for non-terminal statuses only (`Now / Waiting / Delegated / Scheduled / AtRisk`). The type-level constraint via `NonTerminalStatus` enum prevents passing `Done` at compile time. `updated_at` bumped on success.
- `complete_operational_item(&self, id: &str, evidence_ref: EvidenceRef) -> Result<()>` — the **only** path to write `status = Done`. Requires an Evidence reference so the terminal transition is always audit-traceable. Returns `Err(AlreadyTerminal)` if the item is already Done. Per foundation Decision G: "Done is terminal."
- `reopen_operational_item(&self, id: &str, evidence_ref: EvidenceRef) -> Result<()>` — the **only** path out of `status = Done`. Transitions back to `Now` (re-derivation takes over on subsequent reads). Requires an Evidence reference per foundation §4 "explicit re-open with audit-trail Evidence link."
- `update_operational_item_priority(&self, id: &str, priority: f32) -> Result<()>` — priority cache update (Layer 2 will call this).
- `query_operational_items(&self, filter: &OperationalItemFilter) -> Result<Vec<OperationalItem>>` — the canonical read query.
- `get_operational_item_by_source(&self, agent_id: &str, source_table: &str, source_id: &str) -> Result<Option<OperationalItem>>` — lookup for status-update paths.
- `count_blocked_items(&self, blocked_by_id: &str) -> Result<u32>` — for dependency_risk scoring.

```rust
// Type-level guard: prevents callers from passing `Done` to update_operational_item_status.
pub enum NonTerminalStatus { Now, Waiting, Delegated, Scheduled, AtRisk }
impl From<NonTerminalStatus> for OperationalStatus { /* trivial */ }
```

All methods wrapped via `AsyncDatabase` channel dispatch following existing patterns. Transaction-accepting variants (`*_tx` suffixed) added where atomicity per §5.2 requires the same `&Transaction` reference to be threaded through source-of-truth + operational writes in one closure.

**EvidenceRefKind enum closure at the DB layer.** Foundation Decision F settles `EvidenceRefKind` as a closed enum (Rust-only). The SQL schema stores `evidence_refs` as `TEXT` (JSON-serialized array) for flexibility, but **all writes MUST go through the Rust `EvidenceRef` type via the methods above** — direct SQL writes that bypass the Rust serializer are not supported and are documented as a CLAUDE.md invariant. Adding a per-row SQLite CHECK constraint validating each JSON element's `kind` is rejected as scope creep (would need a JSON-aware constraint function in SQLite) — the Rust-type-as-only-writer contract is the simpler enforcement. CLAUDE.md § "Conventions" gets a new bullet: *"Writes to `operational_items.evidence_refs` MUST go through `crates/mika-agent/src/operational/types.rs::EvidenceRef`. Direct SQL writes that bypass the Rust type are unsupported and will break the closed-enum guarantee from foundation Decision F."*

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
pub fn write_callback_completion(db: &Database, agent_id: &str, task: &Task, has_pr_url: bool, pr_url: Option<&str>) -> Result<()> {
    if let Some(item) = db.get_operational_item_by_source(agent_id, "tasks", &task.id)? {
        if has_pr_url {
            // Terminal transition — requires Evidence reference per Decision G.
            let evidence = EvidenceRef {
                kind: EvidenceRefKind::GithubPr,
                id: pr_url.unwrap_or_default().to_string(),
            };
            db.complete_operational_item(&item.id, evidence)?;
        } else {
            // Non-terminal: keep available for next pilot run.
            db.update_operational_item_status(&item.id, NonTerminalStatus::AtRisk)?;
        }
    }
    Ok(())
}
```

### 3.8 mika-arch DECISION NEEDED → Decision (foundation §7)

Hook: `tools/run_gh.rs` or wherever mika-arch's grooming output is consumed by the agent loop, on detection of a `DECISION NEEDED — <ID>` marker in the architect's response (the `mika-arch-groom-ticket` / `mika-arch-second-review` skills emit these as part of plan-doc reviews).

```rust
pub fn write_arch_decision_item(db: &Database, agent_id: &str, marker: &ArchDecisionMarker, session_id: &str) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Decision,
        title: format!("Stamp marker {} ({}) on {}", marker.id, marker.title, marker.doc_path),
        status: OperationalStatus::Now,
        owner: Owner::User,
        evidence_refs: vec![EvidenceRef {
            kind: EvidenceRefKind::External,
            id: format!("mika-arch-session:{}", session_id),
        }],
        confidence: 1.0,  // architect explicitly surfaced this
        source_table: Some("tool_calls".to_string()),
        source_id: Some(marker.tool_call_id.clone()),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}
```

The `ArchDecisionMarker` struct captures `{ id: String, title: String, doc_path: String, tool_call_id: String }` parsed from the architect's response. Detection is via regex against the standard marker pattern `**DECISION NEEDED — <ID> (<title>):**` in the architect's response body.

**Layer 1 / Layer 3 ownership split:** The detection regex and parser live in Layer 1 (this ticket) as part of the write-path module. The deeper integration — where in the agent loop the detection fires, and how the resulting Decision items surface back to the operator — is Layer 3's concern (`tool_execution/` module per foundation §6). Layer 1 ships the writer; Layer 3 wires the trigger.

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

### 5.2 Write-Path Atomicity (Decision D) — choice (a) true atomicity

Per foundation Decision D, writes are single-transaction with the source-of-truth table. Choosing **(a) true atomicity** per mika-arch's first-pass F1 framing:

- **Reminders, manual tasks:** The `create_task()` call and the `upsert_operational_item_by_source()` call run inside the same `rusqlite::Transaction`. Implementation: each subsystem's hook site is rewritten to use `Database::with_transaction(|tx| { ... })`, passing the same `&Transaction` reference to both the source-of-truth write and the operational write. New `with_transaction`-accepting variants are added to `db/operational.rs` methods where needed (e.g., `upsert_operational_item_by_source_tx`).
- **Webhooks, callbacks, team runs:** These already run inside the `AsyncDatabase` actor's closure via `with_db`. The operational write is appended to the **same closure**, sharing the closure's `&mut Database` reference. SQLite's implicit transaction semantics ensure both writes commit or roll back together.
- **Failure mode (revised):** If the operational write fails inside the transaction, **the entire transaction rolls back, including the source-of-truth write.** The caller surfaces the error to its caller (HTTP 500 for webhook handlers, error return for tool dispatch, etc.) and the operation is retried per the existing retry semantics of each source path. GitHub will redeliver webhooks on 5xx, scheduled tasks retry via the engine tick loop, and user-facing tools surface the error to the user. This is the canonical interpretation of Decision D ("correctness is the v1 priority") and respects the augmentation-not-replacement rule from foundation §3: if the augmentation can't be written, the source row shouldn't be either — otherwise the source becomes the only truth and the OperationalItem layer drifts behind silently.

**Rejected interpretation (pilot's rev 1):** "Log a warning and continue" if the operational write fails. This makes the writes separable rather than atomic, contradicts Decision D, and produces a silent inconsistency between the source and the operational layer — exactly the dual-source-of-truth bug Decision D was settled to prevent.

### 5.3 mika#1258 Sequencing (Decision D consideration) — SETTLED

Foundation Decision D notes Layer 1 and mika#1258 (async_db backpressure) should sequence together. This plan **explicitly settles the sequencing**: Layer 1 lands FIRST with the existing `sync_channel(512)` pattern; mika#1258 lands SECOND as a transparent transport migration.

**Rationale:**
- The operational writes are small (one INSERT or UPDATE per event) and add ~7 new write paths × ~10 events/day per typical agent = ~70 additional writes/day. Negligible relative to current `tool_calls` + `messages` write volume; will not meaningfully saturate the existing channel.
- The `with_db` and `with_transaction` closure interfaces are stable across the sync_channel-vs-actor implementation change. Migration from sync_channel to DB-as-actor (when mika#1258 ships) is internal to `AsyncDatabase` and does not require Layer 1 code changes.
- Atomicity per §5.2 is achieved by SQLite transaction semantics inside the `with_db` closure, not by the channel transport. Both transport implementations preserve transaction boundaries.

**Sequencing implication for the project queue:** Layer 1 (this ticket) does NOT depend on mika#1258 landing first. mika#1258 can land before, during, or after Layer 1 with no impact on Layer 1's semantics or correctness.

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

---

## Changelog

- **2026-05-24 (rev 2)** — CC resolved mika-arch first-pass findings (session `d2df893e`, Disposition: ITERATE). **F1 (BLOCKING):** §5.2 rewritten to choose option (a) true atomicity — log-and-continue failure mode removed; operational write failure rolls back the entire transaction; rejected interpretation explicitly noted. §5.3 strengthened to explicitly settle mika#1258 sequencing as "Layer 1 first, mika#1258 transparent transport migration after." **F2 (NON-BLOCKING):** Phase 2 documents `EvidenceRefKind` closure as Rust-only enforced via the type-as-only-writer contract; CLAUDE.md § Conventions gets a new invariant bullet. **F3 (NON-BLOCKING):** Phase 2 splits status updates into three terminal-aware methods — `update_operational_item_status(id, NonTerminalStatus)` for non-terminal transitions, `complete_operational_item(id, evidence)` for Done writes (the only path to Done; requires Evidence ref per Decision G), and `reopen_operational_item(id, evidence)` for the audit-trailed re-open path. Type-level guard via new `NonTerminalStatus` enum prevents passing `Done` at compile time. **F4 (NON-BLOCKING):** Added Phase 3.8 covering the mika-arch DECISION NEEDED → Decision write path, including the `ArchDecisionMarker` parser and the Layer 1 / Layer 3 ownership split. §3.6 callback completion updated to use the new `complete_operational_item` method with Evidence ref.
- **2026-05-24 (rev 1)** — Initial plan by autonomous dev-groom session 741fa338. Pilot exited PIPELINE_INCOMPLETE before committing; CC recovered the uncommitted plan from the worktree for architect review.
