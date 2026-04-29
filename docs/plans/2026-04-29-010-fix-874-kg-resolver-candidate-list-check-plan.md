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
---

# Plan — mika#874: candidate-list check rejects valid LLM matches as no_match

## Phase 0 — Pin (F5)

Before coding, read and understand:

- `crates/mika-agent/src/kg/entity_resolver.rs:558-587` — `try_exact_match`. Stage-1 path; uses `kg_entities` direct DB query, returns `Option<DomainCandidate>`, scoped to "exactly one match" via `LOWER(entity_key) = LOWER(?1)`. **The new helper `try_domain_entity_by_key` differs by:** taking an externally-supplied `matched_key` (not the subject's `entity.entity_key`) and an `entity_type` for type-bounded scoping. Same DB layer, same return shape; different call site (Stage-2 post-LLM validation, not Stage-1).
- `crates/mika-agent/src/kg/entity_resolver.rs:803-835` — `get_domain_candidates`. The range-scan shape this fix mirrors (see Approach Change 1).
- `crates/mika-agent/src/kg/entity_resolver.rs:661-693` — the validation branch this fix patches.
- `crates/mika-agent/src/db.rs:1587-1601` — `kg_resolutions_log` CHECK constraint on `outcome` (load-bearing for the F4 decision below).

## Ticket-body citations (B-with-guardrail)

Each load-bearing claim cites a specific quote from mika#874's body.

- **Symptom:** > "LLM returned `entity_key=X` and the resolver computes `matched_key=X`, the row is recorded as `no_match`."
- **Log evidence:** the JSON fragment in the ticket — `"event":"resolution_matched_key_not_in_candidates"`, `"entity_key":"skill:shared_code_reuse"`, `"matched_key":"skill:shared_code_reuse"`, `"target":"mika_agent::kg::entity_resolver"`.
- **Verification window:** > "Verified 2026-04-29 against `~/.mika/data/mika.db` (post-restart, post-merge of #872)."
- **Scale:** > "Hundreds of valid resolutions lost per batch across mika-arch / mika-dev / mika-qa."
- **File named by ticket:** > "`crates/mika-agent/src/kg/entity_resolver.rs` (candidate comparison logic, the `matched_key_not_in_candidates` branch)."
- **Acceptance — primary:** > "A resolution batch where the LLM returns an `entity_key` that exists in `kg_entities` produces `matched_llm > 0` for those rows."
- **Acceptance — counted-as bug:** > "They are no longer counted as `no_match` in `resolution_pending_complete` events."
- **Acceptance — replay observable:** > "Replay against current pending backlog measurably reduces `kg_subject_resolutions` pending count for mika-arch."
- **Cross-link to #875:** > "Combined with #2 (Stage-1 path returns 0), this is the dominant cause of mika-arch's 28,997-subject pending backlog."

## Root Cause (F1) — pinned

The observed `matched_key == entity.entity_key` log signal can arise from two mechanically distinct causes. This plan defends against **Cause A** as the load-bearing invariant. **Cause B** is independent and separately observable.

- **Cause A — Valid match outside the in-prompt candidate window (this fix's invariant).** `get_domain_candidates(entity_type)` at `entity_resolver.rs:803-835` returns at most `MAX_DISAMBIGUATION_CANDIDATES = 50` rows ordered alphabetically. When the correct domain entity exists in `kg_entities` but falls outside that window, the LLM disambiguation prompt does not contain it. The LLM, primed with the subject's full `entity.entity_key` in its context, may still emit the correct `matched_key` from prior knowledge — but the post-LLM `candidates.iter().find(...)` validation at `entity_resolver.rs:664-666` rejects it because the in-prompt slice was truncated. **Defended invariant:** any LLM-returned `matched_key` that exists as a domain entity of the same type as the subject is a valid match, regardless of whether it appeared in the in-prompt slice.
- **Cause B — Stage-1 candidate query broken upstream (sibling #875, separate ticket).** If Stage-1 exact match returns 0 across recent batches, all subjects that should resolve via `matched_exact` instead fall through to Stage-2 LLM disambiguation. This loads Stage-2 with subjects whose domain match is the subject's own key, producing the `matched_key == entity.entity_key` log shape *additionally* through this path. **#874's fix is correct and ships independently** — once #875 also lands, Stage-1 carries the load again and Stage-2's residual `matched_key == entity.entity_key` traffic reduces, but the F2 range-bounded DB-fallback is still the right Stage-2 invariant.

The `matched_key == entity.entity_key` symptom alone does not discriminate A from B. The defended invariant ("matched_key exists in `kg_entities` with matching type → accept") is correct under both causes.

## Approach

When the LLM's `matched_key` is not in the in-prompt `candidates` slice, perform a type-bounded DB lookup against `kg_entities` before falling through to `Ok(None)`. If the matched key exists as a domain entity of the same `entity_type` as the subject, accept the resolution as `matched_llm` and emit a structured-log event (`resolution_matched_key_db_fallback_hit`). Otherwise, emit one of two reject events (cross-type or DB-miss) and return `Ok(None)`.

### Change 1 — Range-bounded DB-fallback helper (F2 commit)

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

The combined `WHERE` clause enforces both the type range bound (the same `>=` / `<` shape as `get_domain_candidates`) AND the case-insensitive equality on `entity_key`. The `kg_entities` UNIQUE index on `entity_key` covers this lookup.

### Change 2 — Validation branch with full event taxonomy (F3 commit)

Replace the existing `else` branch at `entity_resolver.rs:680-690` with three explicit outcome paths plus the existing accept path. **F3 Log Event Inventory table:**

| Path | Condition | Outcome | Event/Log | DB Row |
|------|-----------|---------|-----------|--------|
| 1 | matched_key in in-prompt candidates | accept | (no event — existing silent-success path at line 668-669) | `outcome='matched_llm'` |
| 2 | matched_key absent from candidates, present in `kg_entities` same type | accept (DB-fallback) | INFO `resolution_matched_key_db_fallback_hit` with fields: `agent_id`, `entity_key`, `matched_key`, `entity_type`, `trace_id` | `outcome='matched_llm'` (see F4 decision) |
| 3 | matched_key absent from candidates, exists in `kg_entities` but wrong type | reject (cross-type) | WARN `resolution_matched_key_cross_type_rejected` with fields: `agent_id`, `entity_key`, `matched_key`, `expected_type`, `found_type`, `trace_id` | `outcome='no_match'` |
| 4 | matched_key absent from candidates AND from `kg_entities` (or wrong type) | reject (no-match) | WARN `resolution_matched_key_not_in_candidates` (existing event preserved) extended with `db_fallback_attempted=true`, `db_fallback_hit=false`, `trace_id` | `outcome='no_match'` |

Path 3 is implementation-distinct from Path 4: Path 3 fires when `matched_key` matches a different-type `kg_entities` row (e.g., LLM returned `tool:foo` for a `skill:*` subject); Path 4 fires when `matched_key` does not match any row. The distinction requires a second DB lookup (case-insensitive scan across all types) to detect Path 3 versus Path 4. **Implementation simplification:** since Path 3 and Path 4 both write `outcome='no_match'`, the second DB lookup is observability-only. The plan keeps the second lookup because cross-type LLM hallucination is a high-value diagnostic signal — without Path 3's distinct event, ops cannot distinguish "LLM hallucinated wrong type" from "LLM hallucinated unknown key".

### Change 3 — Outcome enum decision (F4 — out-of-scope with rationale)

**F4 decision: schema-level metric separation is OUT OF SCOPE for this fix.** The DB-fallback path writes `outcome='matched_llm'` (existing variant), with structured log differentiation via the `resolution_matched_key_db_fallback_hit` event (Change 2 row 2).

**Rationale:**
- The CHECK constraint at `crates/mika-agent/src/db.rs:1591-1593` enumerates allowed values: `matched_exact, matched_llm, no_match, skipped_discovered_type, skipped_no_llm, error`. Adding `matched_llm_db_fallback` requires a schema bump (v28→v29) plus a SQLite table-rebuild migration (CHECK constraint changes cannot be done in place on SQLite — the documented pattern at `db.rs:3071+` and `db.rs:3359+` confirms this).
- A schema migration on a p0-critical fix expands blast radius beyond what the ticket asks for. Per `feedback_pipeline_match_severity.md` (match pipeline severity to ask), and per the milestone's "out of scope" callout in the parent body (> "KG schema redesign (current schema is correct)."), schema changes are explicitly disallowed at milestone scope.
- Metric separation is achievable via the structured log event `resolution_matched_key_db_fallback_hit`: ops queries against the JSON log stream (`grep resolution_matched_key_db_fallback_hit | jq -s 'group_by(.agent_id)'`) deliver the same separation. Audit-event rows are a parallel option; not included in this fix to minimize blast radius.
- A follow-up ticket — to be filed after this milestone closes — can add `matched_llm_db_fallback` as a proper enum variant with schema bump if log-based metrics prove insufficient. Filing reference: this plan's existence in the milestone#19 sequencing record makes the follow-up easy to discover.

### Files to change

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/entity_resolver.rs` | Insert `try_domain_entity_by_key` helper. Replace the `else` branch at line 680-690 with the three explicit outcome paths from Change 2. |
| `crates/mika-agent/src/kg/entity_resolver.rs` (tests) | Four unit tests, one per row of the inventory table — F3 paths 1, 2, 3, 4. Use the existing `#[cfg(test)] mod tests` pattern in this file plus the `tests/eval/kg_fixtures/` helpers (`seed_domain_entity`, `seed_subject_entity`) per the agent crate's testing conventions. |
| `crates/mika-agent/src/kg/entity_resolver.rs` (telemetry) | Add the two new log events `resolution_matched_key_db_fallback_hit` (INFO) and `resolution_matched_key_cross_type_rejected` (WARN) with the fields enumerated in Change 2. |

No schema migration. No public API change. No changes to `kg_resolutions_log` outcome enumeration.

### Test plan

- **Unit (mandatory):** four tests covering F3 paths 1–4 with sqlite-in-memory and the existing `kg_fixtures/` helpers.
- **Integration:** the existing `tests/eval/grounding_regressions/` suite should still pass — the resolver's behavior on grounded reasoning is unchanged for in-candidate matches.
- **Pre-deploy proxy test (F6):** since `outcome='matched_llm_db_fallback'` is out of scope, the F6 SQL probe is rephrased: `SELECT COUNT(*) FROM <log-stream> WHERE event='resolution_matched_key_db_fallback_hit' AND agent_id='mika-arch' > 0` after a manual replay of the resolver against the existing pending backlog. If the structured log stream is not directly SQL-queryable, a `grep | wc -l` on the JSON log file gates the same signal.
- **Replay verification (acceptance criterion 3, post-deploy):** run `resolve_pending(budget=500)` against the existing 28,997-subject backlog after deploy; observe `kg_subject_resolutions` row count for mika-arch increases (specifically: rows where the resolver previously emitted `matched_key_not_in_candidates` warns now emit `db_fallback_hit` info events and write resolution rows).

### Acceptance

| Ticket criterion | How verified |
|---|---|
| Resolution batch with LLM-returned `entity_key` existing in `kg_entities` produces `matched_llm > 0` | Unit test F3 row 2 (DB-fallback hit, same type) + post-deploy `outcome='matched_llm'` count rise |
| No longer counted as `no_match` in `resolution_pending_complete` events | Unit test F3 row 4 distinguishes from row 2; post-deploy `no_match` count drop with corresponding `db_fallback_hit` rise |
| Replay measurably reduces pending count for mika-arch | Manual replay produces `db_fallback_hit` events, with `kg_subject_resolutions` row count growth |

## Risks and unknowns

- **Hallucination floor.** Same as current code: the LLM can pick `skill:foo` (real, wrong) instead of `skill:bar` (correct). This fix does not increase that risk because Path 3 (cross-type) is rejected, and same-type DB-fallback acceptance has the same trust profile as same-type in-prompt acceptance. Chunk-context grounding (already in place at `entity_resolver.rs:606`, line 609) is the existing defense; not modified.
- **F7 sequencing relative to #875.** Independent. No ordering required. **Joint-metric note:** post-deploy, the `matched_exact` outcome count tracks #875's success (Stage-1 path returning hits again). The `matched_llm` outcome count + the `db_fallback_hit` event count tracks #874's success (Stage-2 path's expanded acceptance). The two metrics are orthogonal and ops should monitor both:
  - #875 healthy: `matched_exact` count > 0 across recent batches
  - #874 healthy: `matched_llm` count > 0 *with* `db_fallback_hit` events firing on the same batches
- **Migration of in-flight runs.** No migration needed — the resolver is idempotent. Subjects pending today get re-evaluated on next `resolve_pending()` call; the new code path applies to them automatically.

## Out of scope

Per the ticket body's explicit out-of-scope list and F4's rationale:

- KG schema redesign.
- Adding new corpora.
- Adding `matched_llm_db_fallback` as a new outcome enum variant (deferred — see F4 rationale).
- Tightening LLM prompt to reduce hallucination rate.
- Changes to the candidate-list cap (`MAX_DISAMBIGUATION_CANDIDATES`).
- Stage-1 resolver path (handled by sibling #875).

## Sequencing note for milestone#19

#874 and #875 are independent (F7) — no ordering required. Joint-metric monitoring distinguishes their respective success signals. The architect's milestone-level review should consider both in the sequencing record.
