---
title: "fix(kg/resolver): candidate-list check rejects valid LLM matches as no_match"
issue: senara-solutions/mika#874
type: fix
milestone: 19
status: draft
created: 2026-04-29
groomed_by: /mika-groom-milestone (per-sub-issue inline draft, B-with-guardrail)
arch_first_pass_session: 3ac2706e-dbc6-428e-8111-b85ee1075645
arch_first_pass_disposition: ITERATE
arch_first_pass_findings_key: mika874_first_pass_findings
external_consult_basis: peer-review brief on F4 + ops-surface evidence (Signal C — sqlite3 query against kg_resolutions_log is canonical operator workflow per mika/CLAUDE.md "Post-restart safety check #757")
---

# Plan — mika#874: candidate-list check rejects valid LLM matches as no_match

## Phase 0 — Pin (F5)

Before coding, read and understand:

- `crates/mika-agent/src/kg/entity_resolver.rs:558-587` — `try_exact_match`. Stage-1 path; uses `kg_entities` direct DB query, returns `Option<DomainCandidate>`. **The new helper `try_domain_entity_by_key` differs by:** taking an externally-supplied `matched_key` (not the subject's `entity.entity_key`) and an `entity_type` for type-bounded scoping. Same DB layer, same return shape; different call site (Stage-2 post-LLM validation, not Stage-1).
- `crates/mika-agent/src/kg/entity_resolver.rs:803-835` — `get_domain_candidates`. The range-scan shape this fix mirrors (Approach Change 1).
- `crates/mika-agent/src/kg/entity_resolver.rs:661-693` — the validation branch this fix patches.
- `crates/mika-agent/src/kg/entity_resolver.rs:53-55` — outcome const list. Adding `MATCHED_LLM_DB_FALLBACK` here (Change 4).
- `crates/mika-agent/src/kg/entity_resolver.rs:1159` — stats JSON formatter. Stringly-typed downstream consumer of outcome values; needs the new key added (see SOLID note below).
- `crates/mika-agent/src/db.rs:1587-1601` — current `kg_resolutions_log` CREATE shape (the v25→v26 / v26→v27 codepath).
- `crates/mika-agent/src/db.rs:3340-3420` — v26→v27 migration shape. The newer precedent we mirror for v28→v29 (per peer-review note: prefer v26→v27 over v25→v26 because it absorbed lessons from #786/#787 transaction-boundary issues).

## Ticket-body citations (B-with-guardrail)

- **Symptom:** > "LLM returned `entity_key=X` and the resolver computes `matched_key=X`, the row is recorded as `no_match`."
- **Log evidence:** the JSON fragment in the ticket — `"event":"resolution_matched_key_not_in_candidates"`, `"entity_key":"skill:shared_code_reuse"`, `"matched_key":"skill:shared_code_reuse"`, `"target":"mika_agent::kg::entity_resolver"`.
- **Verification window:** > "Verified 2026-04-29 against `~/.mika/data/mika.db` (post-restart, post-merge of #872)."
- **Scale:** > "Hundreds of valid resolutions lost per batch across mika-arch / mika-dev / mika-qa."
- **File named by ticket:** > "`crates/mika-agent/src/kg/entity_resolver.rs` (candidate comparison logic, the `matched_key_not_in_candidates` branch)."
- **Acceptance — primary:** > "A resolution batch where the LLM returns an `entity_key` that exists in `kg_entities` produces `matched_llm > 0` for those rows."
- **Acceptance — counted-as bug:** > "They are no longer counted as `no_match` in `resolution_pending_complete` events."
- **Acceptance — replay observable:** > "Replay against current pending backlog measurably reduces `kg_subject_resolutions` pending count for mika-arch."
- **Cross-link to #875:** > "Combined with #2 (Stage-1 path returns 0), this is the dominant cause of mika-arch's 28,997-subject pending backlog."

## Root Cause (F1)

The observed `matched_key == entity.entity_key` log signal can arise from two mechanically distinct causes. This plan defends against **Cause A** as the load-bearing invariant. **Cause B** is independent and separately observable.

- **Cause A — Valid match outside the in-prompt candidate window (this fix's invariant).** `get_domain_candidates(entity_type)` at `entity_resolver.rs:803-835` returns at most `MAX_DISAMBIGUATION_CANDIDATES = 50` rows ordered alphabetically. When the correct domain entity exists in `kg_entities` but falls outside that window, the LLM disambiguation prompt does not contain it. The LLM, primed with the subject's full `entity.entity_key` in context, may still emit the correct `matched_key` from prior knowledge — but the post-LLM `candidates.iter().find(...)` validation at `entity_resolver.rs:664-666` rejects it because the in-prompt slice was truncated. **Defended invariant:** any LLM-returned `matched_key` that exists as a domain entity of the same type as the subject is a valid match, regardless of whether it appeared in the in-prompt slice.
- **Cause B — Stage-1 candidate query broken upstream (sibling #875, separate ticket).** If Stage-1 returns 0, all subjects fall through to Stage-2 LLM disambiguation, loading Stage-2 with subjects whose domain match is the subject's own key — producing the `matched_key == entity.entity_key` log shape *additionally* through this path. **#874's fix is correct and ships independently** of #875; once both ship, Stage-1 and Stage-2 both work and the backlog drains.

The symptom alone does not discriminate A from B. The defended invariant ("matched_key exists in `kg_entities` with matching type → accept") is correct under both causes.

## Approach

When the LLM's `matched_key` is not in the in-prompt `candidates` slice, perform a type-bounded DB lookup against `kg_entities` before falling through to `Ok(None)`. If the matched key exists as a domain entity of the same `entity_type` as the subject, accept the resolution and write `outcome='matched_llm_db_fallback'` (NEW enum variant — see Change 4) so SQL ops can distinguish DB-fallback acceptance from in-prompt acceptance. Cross-type matches reject. DB-misses reject with the existing warn extended.

### Change 1 — Range-bounded DB-fallback helper (F2)

Per F2, this fix commits to **option (b): range-bounded DB query**, mirroring `get_domain_candidates`'s range scan shape. Wrong-type `matched_key` values never reach the fallback acceptance path — the type bound is in the SQL `WHERE` clause, not in post-fetch filtering, so a buggy comparison cannot leak across types.

New helper in `crates/mika-agent/src/kg/entity_resolver.rs`:

```rust
/// Defensive DB lookup for an LLM-returned matched_key not in the in-prompt
/// candidates slice. Type-bounded via range scan to refuse cross-type matches
/// at the SQL level (mirrors get_domain_candidates at line 803-835).
async fn try_domain_entity_by_key(
    &self,
    entity_type: &str,
    matched_key: &str,
) -> Result<Option<DomainCandidate>> {
    let range_start = format!("{entity_type}:");
    let range_end = format!("{entity_type};");
    let key = matched_key.to_string();

    self.db
        .with_db(move |db| {
            let mut stmt = db.conn.prepare(
                "SELECT id, entity_key, properties_json FROM kg_entities
                 WHERE entity_key >= ?1 AND entity_key < ?2
                   AND LOWER(entity_key) = LOWER(?3)
                 LIMIT 1",
            )?;
            let result = stmt
                .query_row(rusqlite::params![range_start, range_end, key], |row| {
                    Ok(DomainCandidate {
                        id: row.get(0)?,
                        entity_key: row.get(1)?,
                        properties_json: row.get(2)?,
                    })
                });
            match result {
                Ok(c) => Ok(Some(c)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow::Error::from(e)),
            }
        })
        .await
}
```

A second helper detects cross-type DB hits (same key, wrong type) for diagnostic logging:

```rust
/// Diagnostic-only: detect whether matched_key exists in kg_entities under a
/// DIFFERENT type. Used to emit the cross_type_rejected event.
async fn try_domain_entity_any_type(
    &self,
    matched_key: &str,
) -> Result<Option<String>> {
    // Returns the entity_type prefix (text up to first ':') if found; else None.
    // Implementation: SELECT entity_key FROM kg_entities WHERE LOWER(entity_key) = LOWER(?1) LIMIT 1.
}
```

### Change 2 — Validation branch with full event taxonomy (F3)

**F3 Log Event Inventory** — schema and logs answer different questions; the table makes the orthogonality explicit.

| Path | Condition | DB outcome | Log event | Field set |
|------|-----------|-----------|-----------|-----------|
| 1 | matched_key in in-prompt candidates | `matched_llm` | (silent — existing accept path at line 668-669) | n/a |
| 2 | matched_key absent from candidates, present in `kg_entities` SAME type | `matched_llm_db_fallback` (NEW — Change 4) | INFO `resolution_matched_key_db_fallback_hit` | `event`, `agent_id`, `entity_key`, `entity_type`, `matched_key`, `domain_entity_id`, `trace_id` |
| 3 | matched_key absent from candidates, exists in `kg_entities` DIFFERENT type | `no_match` | WARN `resolution_matched_key_cross_type_rejected` | `event`, `agent_id`, `entity_key`, `entity_type`, `matched_key`, `found_type`, `trace_id` |
| 4 | matched_key absent from candidates AND from `kg_entities` (any type) | `no_match` | WARN `resolution_matched_key_not_in_candidates` (existing event, extended) | `event`, `agent_id`, `entity_key`, `entity_type`, `matched_key`, `db_fallback_attempted=true`, `db_fallback_hit=false`, `trace_id` |

**Schema vs logs — primary purposes (per peer-review note):**

> Schema separation is primary for **counting**: `SELECT outcome, COUNT(*) FROM kg_resolutions_log GROUP BY outcome` answers "how many DB-fallback hits per agent / per restart / per backlog drain?" — the load-bearing operator query (Signal C in `mika/CLAUDE.md` Post-restart safety check #757).
>
> Log events are primary for **diagnosis**: when the count tells you DB-fallback fired N times, the log events tell you *which* `agent_id`, *which* `entity_type`, *which* `matched_key`, and *which* `trace_id` correlate. They are not redundant defense-in-depth — they answer a different question (why and where, vs how often).

A future maintainer who sees both layers should not assume one is removable. The Inventory's two columns (DB outcome / Log event) make the contract explicit.

**Field-name stability (locked).** All three new events share the same key set and types, matching the existing `domain_builder.rs:158` and `entity_resolver.rs` conventions:
- `event` (string, snake_case event name)
- `agent_id` (string)
- `entity_key` (string, the SUBJECT's entity_key)
- `entity_type` (string, the SUBJECT's entity_type)
- `matched_key` (string, the LLM-returned key)
- `trace_id` (string)
- Event-specific: `domain_entity_id` (i64) on accept; `found_type` (string) on cross_type; `db_fallback_attempted` + `db_fallback_hit` (bool) on the no-match extension.

No drift to `agent` / `kind` / `entity` shorthands. All snake_case, all consistent with existing kg/ events.

### Change 3 — Schema migration v28 → v29

Mirrors the v26→v27 shape at `db.rs:3340+` — an `ALTER TABLE RENAME` + `CREATE TABLE` + `INSERT INTO ... SELECT *` + `DROP` + index recreate, all inside a single `transaction_with_behavior(TransactionBehavior::Immediate)`.

```sql
-- v28 -> v29: add 'matched_llm_db_fallback' to kg_resolutions_log.outcome CHECK
ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v28_backup;

CREATE TABLE kg_resolutions_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
        'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
    )),
    resolution_trace_id TEXT NOT NULL,
    source_extraction_trace_id TEXT,
    model TEXT,
    duration_ms INTEGER,
    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, subject_entity_id)
);

INSERT INTO kg_resolutions_log
    (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
     source_extraction_trace_id, model, duration_ms, resolved_at)
SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
       source_extraction_trace_id, model, duration_ms, resolved_at
FROM kg_resolutions_log_v28_backup;

DROP TABLE kg_resolutions_log_v28_backup;

CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

INSERT INTO schema_version (version) VALUES (29);
```

**Transaction-boundary discipline (per peer-review note):**
- Single `Immediate` transaction wraps the entire DDL batch (matches v26→v27 at `db.rs:3398-3402`).
- `INSERT INTO ... SELECT (id, ...)` explicitly preserves the `id` column. Since `id` is `INTEGER PRIMARY KEY AUTOINCREMENT`, this preserves rowid alias semantics.
- `idx_kg_res_log_pending` recreated post-INSERT (the only index on the table).
- FK references on `agent_id` (→ `agents`) and `subject_entity_id` (→ `kg_subject_entities`) preserved by recreating the column declarations identically.
- No triggers to consider (table has none).
- The CREATE shape at `db.rs:1587-1601` (the v25 baseline) is also updated so fresh installs land at v29 without going through migration.

Same v26→v27 PRAGMA pattern: `PRAGMA foreign_keys = OFF` before, `PRAGMA foreign_keys = ON` after — already part of the migration codepath; no new toggle needed.

### Change 4 — Outcome const + downstream consumer update

`crates/mika-agent/src/kg/entity_resolver.rs:53-55`:

```rust
mod outcome {
    pub const MATCHED_EXACT: &str = "matched_exact";
    pub const MATCHED_LLM: &str = "matched_llm";
    pub const MATCHED_LLM_DB_FALLBACK: &str = "matched_llm_db_fallback";  // NEW
    pub const NO_MATCH: &str = "no_match";
    pub const SKIPPED_DISCOVERED_TYPE: &str = "skipped_discovered_type";
    pub const SKIPPED_NO_LLM: &str = "skipped_no_llm";
    pub const ERROR: &str = "error";
}
```

**Downstream stringly-typed consumers of outcome values (rg sweep, locked):**

The seventh variant produces a silent-miss surface in exactly **one** non-test site outside `entity_resolver.rs`: the stats JSON at `entity_resolver.rs:1159` —

```rust
r#"{{"total":{},"matched_exact":{},"matched_llm":{},"no_match":{},"skipped_discovered":{},"skipped_no_llm":{},"errors":{}}}"#
```

Plan adds a `matched_llm_db_fallback` count to this JSON shape and to the `ResolutionStats` struct that populates it. No other crate-wide `match outcome.as_str()` arms branch on these strings — verified by `rg "matched_exact"|"matched_llm"|"no_match"` against `crates/mika-agent/src/`.

**SOLID smell flagged for follow-up (do NOT fix in this PR):** outcome values are a domain enum leaking as bare strings through the const list, the CHECK constraint, the stats JSON formatter, and any future consumer. A typed enum (`#[derive] enum Outcome { ... }` with a `to_db_str` shim and a `TryFrom<&str>`) would catch this drift at compile time. File a follow-up after milestone#19 closes; not in scope for #874's p0 fix.

### Files to change

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/entity_resolver.rs` | Insert `try_domain_entity_by_key` + `try_domain_entity_any_type` helpers. Replace the `else` branch at line 680-690 with the four explicit outcome paths from Change 2. Add `MATCHED_LLM_DB_FALLBACK` to the outcome const block. Add `matched_llm_db_fallback` field to `ResolutionStats` and to the stats JSON formatter at line 1159. Add the two new structured log events. |
| `crates/mika-agent/src/db.rs` | Schema bump to v29 (Change 3 SQL). Update the v25 baseline CREATE at line 1587-1601 to match the v29 CHECK shape. |
| `crates/mika-agent/src/kg/entity_resolver.rs` (tests) | Five unit tests: F3 path 1 (in-list accept), path 2 (DB-fallback hit same-type → `matched_llm_db_fallback`), path 3 (cross-type rejected → `no_match`), path 4 (DB miss → `no_match` + extended warn), plus a migration test (v28 → v29 preserves rows + accepts new variant on insert). |
| `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` | Bump the schema-version pin assertion to v29. Update fixture seeders if they reference the outcome enumeration. |

### Test plan

- **Unit (mandatory):** five tests above. Use existing `kg_fixtures/` helpers (`seed_domain_entity`, `seed_subject_entity`).
- **Migration test:** v28→v29 round-trip — seed pre-migration DB at v28, run migration, assert (a) row count preserved, (b) all `id` values preserved, (c) `INSERT INTO kg_resolutions_log VALUES (..., 'matched_llm_db_fallback', ...)` succeeds, (d) `INSERT ... 'invalid_value'` still fails CHECK.
- **Integration:** existing `tests/eval/grounding_regressions/` should still pass.
- **Pre-deploy proxy query (F6, schema-aware now):** after the worktree builds, `INSERT INTO kg_resolutions_log VALUES (..., 'matched_llm_db_fallback', ...)` succeeds on a fresh DB; before the migration lands, the same INSERT fails CHECK. Trivial to gate.
- **Replay verification (acceptance criterion 3, post-deploy):** run `resolve_pending(budget=500)` against the existing 28,997-subject backlog. Observe (a) `SELECT COUNT(*) FROM kg_resolutions_log WHERE outcome='matched_llm_db_fallback' AND agent_id='mika-arch'` > 0, (b) the corresponding INFO `resolution_matched_key_db_fallback_hit` events in `server.log`.

### Acceptance

| Ticket criterion | How verified |
|---|---|
| Resolution batch with LLM-returned `entity_key` existing in `kg_entities` produces `matched_llm > 0` for those rows | Unit test F3 row 2 (DB-fallback hit, same type) writes `matched_llm_db_fallback` (a strict superset of "produces matched_llm > 0" — the ticket's criterion is satisfied; the new variant is additionally trackable) |
| No longer counted as `no_match` in `resolution_pending_complete` events | Unit tests F3 row 2 vs row 4 distinguish; post-deploy stats JSON includes both `matched_llm` and `matched_llm_db_fallback`, with `no_match` count reduced |
| Replay measurably reduces pending count for mika-arch | Manual replay produces `db_fallback_hit` events + new outcome rows; `kg_subject_resolutions` row count grows |

## Risks and unknowns

- **Migration risk.** SQLite CHECK expansion via table rebuild has known failure classes around transaction boundaries. Plan mirrors the v26→v27 shape (Immediate transaction, single `execute_batch`, post-migration count log). If the pre-migration DB has any rows whose `outcome` value is not in the v28 set (shouldn't happen — CHECK enforces it), the INSERT silently filters them; an explicit `SELECT COUNT(*)` before/after diff in the migration log should catch this. Add the count assertion to the migration's INFO log line per the v26→v27 precedent at `db.rs:3408+`.
- **Hallucination floor.** Same as current code: LLM can pick `skill:foo` (real, wrong) instead of `skill:bar` (correct). This fix doesn't increase that risk — Path 3 (cross-type) rejects, Path 2 same-type DB-fallback has the same trust profile as today's same-type in-prompt acceptance. Chunk-context grounding at `entity_resolver.rs:606-609` is the existing defense.
- **F7 sequencing — independent (verified by reading #875's body).** #875 fixes Stage-1 to produce `matched_exact > 0`; it uses the *existing* enum value, not a new one. No batching opportunity with this fix's schema change. They ship independently.

### Joint-metric note (F7) — three distinct outcomes + tripwire

Post-fix observability shape on `kg_resolutions_log`:

| Outcome | Path | Expected relative volume (post #874+#875) |
|---------|------|--------------------------------------------|
| `matched_exact` | Stage-1 (sibling #875 fix) | Should dominate |
| `matched_llm` | Stage-2 in-prompt accept | Bulk of LLM-resolved remainder |
| `matched_llm_db_fallback` | Stage-2 DB-fallback accept (this fix) | Small tail |

**Tripwire:** if `matched_llm_db_fallback` volume approaches or exceeds `matched_llm` volume across a sustained window (say, 3 consecutive batches), the in-prompt 50-cap is too tight in practice and `MAX_DISAMBIGUATION_CANDIDATES` should be revisited. Adding the tripwire here gives ops a specific signal rather than three counters to stare at.

Cost-prediction sanity: the same OpenRouter pricing math from `mika/CLAUDE.md` Signal D applies — DB-fallback adds at most one extra DB round-trip per Stage-2 call (negligible vs LLM call cost), so per-restart $ envelope is unchanged.

## Out of scope

- KG schema redesign beyond the v28→v29 CHECK constraint expansion (the milestone parent body's "schema redesign" out-of-scope clause is read permissively per peer review — adding one allowed value to a CHECK constraint is not restructuring).
- Adding new corpora.
- Tightening LLM prompt to reduce hallucination rate.
- Changes to the candidate-list cap (`MAX_DISAMBIGUATION_CANDIDATES`) — the tripwire above triggers a future ticket if needed.
- Stage-1 resolver path (handled by sibling #875).
- Refactoring outcome values to a typed Rust enum (SOLID smell flagged in Change 4 — file follow-up after milestone#19 closes).

## Sequencing note for milestone#19

#874 and #875 are independent (F7) — no ordering required. Joint-metric monitoring distinguishes their respective success signals. The architect's milestone-level review should consider both in the sequencing record.
