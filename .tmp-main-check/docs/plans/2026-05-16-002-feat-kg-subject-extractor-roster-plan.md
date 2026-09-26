---
ticket: mika#1154
title: KG subject extractor produces subjects with no domain-graph counterpart
type: feat
labels: [enhancement, p1-important, agent-core]
created: 2026-05-16
plan_seq: 002
status: drafted
revision: pass-1-iterate-applied
base_commit: f459130f17a73d9e65eadbdd1a7bb2eaf0e3b999
related:
  - mika#1152 (parent investigation — closed by sonnet-baseline experiment)
  - mika#1076 (corpora backfill — umbrella-paused)
  - mika#1077 (no_match instrumentation — umbrella-paused)
  - mika#1091 (gpt-5-nano MaxTokens — umbrella-paused)
  - docs/solutions/kg-investigations/2026-05-16-resolver-sonnet-baseline.md
  - docs/brainstorms/2026-05-11-kg-query-tool-deprecation-question.md
---

# Plan: KG subject-extractor roster grounding (mika#1154)

## Recap (one paragraph)

The resolver-sonnet baseline (mika#1152) found 0/87 sampled `no_match` outcomes
have a domain-entity match by `entity_key` or by name. Four cited examples
(`agent:vincent`, `agent:ci`, `agent:tower_http`, `tool:tailwind`) confirm the
resolver isn't failing to disambiguate — the extractor is producing subjects
with no canonical counterpart and never could (operator names, abstract roles,
Rust crates, CSS frameworks are not platform agents/tools). Decision A
(2026-05-12) removed `query_knowledge_graph` from mika-arch, so this ticket's
urgency is anticipatory, not firefighting.

---

## Phase 0 — Pinned source slices

Base commit on `main`: **`f459130f17a73d9e65eadbdd1a7bb2eaf0e3b999`**
(`fix(tui): supervise agent-worker tokio::spawn (mika#1149) (#1151)`).

All file:line refs in this plan are pinned to this SHA. If the implementation
PR rebases past main, the implementer must re-pin and adjust the plan.

### Pin A — `domain_builder.rs:12–21` (sole-writer contract)

Verbatim from `crates/mika-agent/src/kg/domain_builder.rs`:

```rust
//! ## Sole-Writer Contract
//!
//! This module is the **sole writer** of entity_keys in the `skill:*`, `tool:*`,
//! `agent:*`, `problem_type:*`, and `concept:*` namespaces. No other code path
//! writes these entity_keys. See `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`.
//!
//! Additionally, this module **deletes** `kg_resolutions_log` rows with
//! `outcome='no_match'` when entity types gain new entities (#960). This is a
//! consequent of the domain-graph mutation, not an independent concern — same
//! pattern as `prune_stale_entities`.
```

**Why pinned**: This contract is the architectural ground for Shape 2's
rejection. The doc-comment is not a runtime assertion; its force is
convention plus the cross-reference to `a2a-dual-write-duplicate-rows.md`.
If a future PR proposes observation-driven emission, this contract is the
text to cite.

### Pin B — `entity_resolver.rs:56–66` (outcome constants)

Verbatim:

```rust
/// Resolution outcome values matching the CHECK constraint on `kg_resolutions_log.outcome`.
mod outcome {
    pub const MATCHED_EXACT: &str = "matched_exact";
    pub const MATCHED_LLM: &str = "matched_llm";
    pub const MATCHED_LLM_DB_FALLBACK: &str = "matched_llm_db_fallback";
    pub const NO_MATCH: &str = "no_match";
    pub const SKIPPED_DISCOVERED_TYPE: &str = "skipped_discovered_type";
    pub const SKIPPED_NO_LLM: &str = "skipped_no_llm";
    pub const ERROR: &str = "error";
}
```

**Comment says**: "matching the CHECK constraint on
`kg_resolutions_log.outcome`." This implies a SQL CHECK constraint on the
outcome column. The plan must audit that CHECK constraint (see § DB schema
audit below); adding `NO_CANDIDATE_OF_TYPE` requires either widening the
CHECK or migrating the column.

### Pin C — `entity_resolver.rs:551–557` (upstream type filter)

Verbatim from `resolve_single_entity()`:

```rust
async fn resolve_single_entity(
    &self,
    entity: &PendingEntity,
    llm_call_allowed: bool,
) -> (ResolutionResult, bool) {
    // D8: Discovered types skip resolution entirely.
    if !KG_DOMAIN_ENTITY_TYPES.contains(&entity.entity_type.as_str()) {
        return (ResolutionResult::SkippedDiscoveredType, false);
    }
```

**Why pinned**: This is the upstream filter that makes `SkippedDiscoveredType`
mutually exclusive with `NoMatch` (and the proposed `NoCandidateOfType`).
Settles F2.

### Pin D — `entity_resolver.rs:570–584` (Stage 1 exact match decision)

Stage 1 decision branch:

```rust
match self.try_exact_match(entity).await {
    Ok(Some(domain)) => {
        // Exact match found. If extraction confidence > threshold, resolve.
        if entity.confidence > EXACT_MATCH_CONFIDENCE_THRESHOLD {
            return (
                ResolutionResult::ExactMatch {
                    domain_entity_id: domain.id,
                    confidence: entity.confidence,
                },
                false,
            );
        }
        // Low confidence — escalate to LLM for verification.
    }
    Ok(None) => {
        // No exact match — escalate to LLM.
    }
```

**Why pinned**: The path into Stage 2 has two distinct entry conditions:
(a) Stage 1 returned `Ok(Some)` with low confidence — *a domain candidate
matching the subject's exact entity_key exists*; (b) Stage 1 returned
`Ok(None)` — *no candidate with this exact entity_key exists*. The plan's
revised Shape 3 implementation pivots on this distinction (see §
Implementation strategy).

### Pin E — `entity_resolver.rs:614–622` (the no_match write decision)

The `Ok(None)` branch from `disambiguate_with_llm` that becomes `NoMatch`:

```rust
Ok(None) => (ResolutionResult::NoMatch, true),
Err(e) => (
    ResolutionResult::Error(format!("LLM disambiguation failed: {e}")),
    true,
),
```

### Pin F — `entity_resolver.rs:670–675` (empty-candidates short-circuit)

Inside `disambiguate_with_llm`:

```rust
// 1. Fetch domain candidates of the same type (bounded to MAX_DISAMBIGUATION_CANDIDATES).
let candidates = self.get_domain_candidates(&entity.entity_type).await?;

if candidates.is_empty() {
    // No domain entities of this type → no match possible.
    return Ok(None);
}
```

**Why pinned**: This is the third distinct path that produces `NoMatch` in
current code. Type passed the upstream filter, but `kg_entities` has zero
entities of that type. The four cited examples (`agent:*`, `tool:*`) do NOT
land here — agents/tools have plenty of entries. This path is rare but real.

### Pin G — `entity_resolver.rs:1087–1118` (get_domain_candidates query shape)

```rust
async fn get_domain_candidates(&self, entity_type: &str) -> Result<Vec<DomainCandidate>> {
    let range_start = format!("{entity_type}:");
    let range_end = format!("{entity_type};"); // ';' is one codepoint above ':' in ASCII

    self.db
        .with_db(move |db| {
            let mut stmt = db.conn.prepare(
                "SELECT id, entity_key, properties_json FROM kg_entities
                 WHERE entity_key >= ?1 AND entity_key < ?2
                 ORDER BY entity_key ASC
                 LIMIT ?3",
            )?;
```

**Why pinned**: Range scan on the UNIQUE entity_key index. Fast. No
expression index needed for this existing query. Settles part of F3.

### Pin H — `entity_resolver.rs:481–503` (apply_result match block)

```rust
ResolutionResult::NoMatch => {
    let model = self.llm.as_ref().map(|l| l.model_name().to_string());
    self.write_log(
        entity.id,
        outcome::NO_MATCH,
        extraction_trace_id,
        model.as_deref(),
        Some(duration_ms),
    )
    .await;
    stats.no_match += 1;
}
ResolutionResult::SkippedDiscoveredType => {
    self.write_log(
        entity.id,
        outcome::SKIPPED_DISCOVERED_TYPE,
        extraction_trace_id,
        None,
        Some(duration_ms),
    )
    .await;
    stats.skipped_discovered += 1;
}
```

**Why pinned**: Outcome-writing dispatch. The new `NoCandidateOfType` arm
slots in here.

### Pin I — `db.rs:1579–1589` (kg_entities DDL, primary boot path)

```sql
CREATE TABLE kg_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_key TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (entity_key = type || ':' || name)
);
CREATE INDEX idx_kg_entities_type ON kg_entities(type);
```

The CHECK constraint `entity_key = type || ':' || name` is load-bearing:
case-insensitive lookup on `entity_key` is equivalent to joint
case-insensitive lookup on `(type, name)`. This is why Stage 1's
`LOWER(entity_key) = LOWER(?)` query suffices for `(type, name)` matching —
Stage 1's `Ok(None)` already proves "no `(type, name)` match in
`kg_entities` (case-insensitive)."

### Pin J — `kg_resolutions_log` CHECK constraint (DB schema audit deferred to implementer)

**TODO during implementation**: locate the `kg_resolutions_log` DDL (the
comment at Pin B references a CHECK constraint on `outcome`). Adding
`no_candidate_of_type` requires either:
- (a) the existing CHECK constraint enumerates allowed strings — must be
  widened via migration, or
- (b) the column is plain TEXT — no schema change needed.

The implementer must report which case applies before writing the
implementation. The plan documents both branches in § Implementation
strategy.

---

## DB schema audit

Run during Phase 0 of implementation:

```bash
grep -n "kg_resolutions_log" crates/mika-agent/src/db.rs
```

Confirm two things:
1. Does the DDL include `CHECK (outcome IN (...))` enumerating allowed values?
2. If yes, the migration must `ALTER TABLE` (SQLite limitation: requires
   `CREATE TABLE _new` + copy + drop + rename pattern, OR widen via raw
   `PRAGMA writable_schema`) to add `'no_candidate_of_type'`.
3. If no, the outcome column is plain TEXT and the new value works
   immediately.

The implementer reports the answer in the PR description.

---

## Shape selection rationale

The ticket enumerates three shapes. Code grounding (Phase 0 pins) collapses
the choice.

### Shape 2 (widen roster via observation) — REJECTED

- **Why rejected**: Violates the sole-writer contract at Pin A
  (`domain_builder.rs:12–21`). The doc-comment names the five namespaces
  (`skill:*`, `tool:*`, `agent:*`, `problem_type:*`, `concept:*`) as
  exclusively written by `domain_builder.rs`. Switching to
  observation-driven emission would (a) accept extractor output as canonical
  truth (let the LLM define what counts as a Mika tool), (b) pollute the
  catalog (`tool:tailwind` becomes canonical, then resolves to itself, then
  "matches" — metric goes green while underlying error worsens), and (c)
  remove the only invariant distinguishing domain truth from extraction
  output.
- **Citation**: Pin A names the contract verbatim, including the
  cross-reference to `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`.

### Shape 1 (constrain extractor to closed-set roster) — DEFERRED to follow-up ticket

- **Why deferred** (unchanged from pass 1): Larger surface, prompt-size
  experiment, needs its own architect pass on the `discovered: true` carveout
  design.
- **Filed at grooming close** with scope: roster injection in
  `build_extraction_prompt()` (`subject_extractor.rs:800+`, see brief).

### Shape 3 (split `no_match` outcome) — PRIMARY FIX (this ticket)

- **What ships in this PR**: Add `NO_CANDIDATE_OF_TYPE` outcome alongside
  existing `NO_MATCH`. Track Stage 1's result (found vs not found) through to
  the `disambiguate_with_llm` decision, and select the outcome accordingly.
  Details in § Implementation strategy.

---

## Outcome precedence (F2 resolution)

The three relevant outcomes are mutually exclusive **by upstream control
flow**, not by guard ordering at the write site:

| Outcome | Triggered when | Pinned site |
|---|---|---|
| `SkippedDiscoveredType` | `entity.entity_type ∉ KG_DOMAIN_ENTITY_TYPES` | Pin C (line 555–557) — top of `resolve_single_entity` |
| `NoCandidateOfType` (NEW) | type in domain, but Stage 1 returned `Ok(None)` AND Stage 2 returned `Ok(None)` | New arm in `apply_result` Pin H |
| `NoMatch` | type in domain, Stage 1 returned `Ok(Some)` (low-confidence escalation) AND Stage 2 returned `Ok(None)` | Pin E (existing return at line 616) — meaning narrows |

**Mutual exclusion proof**:
- Reaching `apply_result` with `NoMatch` or `NoCandidateOfType` requires
  passing the Pin C type filter (else outcome = `SkippedDiscoveredType`).
- Distinguishing `NoCandidateOfType` from `NoMatch` requires tracking Stage
  1's `Ok(None)` vs `Ok(Some)` result through to the outcome decision.
- The `candidates.is_empty()` short-circuit at Pin F is structurally a
  `NoCandidateOfType` case (no entities of type in `kg_entities`), even
  though it falls below the Pin C filter — it triggers when type is in
  `KG_DOMAIN_ENTITY_TYPES` (a static const) but `kg_entities` has zero rows
  of that type at runtime.

**No guard ordering at write site needed**: the existing `match result`
block (Pin H) dispatches by enum variant. The implementer adds a
`ResolutionResult::NoCandidateOfType` variant; the match arms remain
parallel. The decision happens in `resolve_single_entity` and
`disambiguate_with_llm`, not in `apply_result`.

---

## Implementation strategy (F3 resolution)

**Key insight from Pin I**: The `kg_entities` CHECK constraint guarantees
`entity_key = type || ':' || name`. Therefore Stage 1's query at line ~631
(`LOWER(entity_key) = LOWER(?1)`) is equivalent to a case-insensitive
`(type, name)` lookup. **Stage 1's `Ok(None)` already proves "no
`(type, name)` match in `kg_entities` case-insensitive."**

This collapses the F3 question:

- **F3 as raised**: "The new query `SELECT COUNT(*) FROM kg_entities WHERE
  type = ? AND lower(name) = lower(?)` needs an expression index."
- **F3 reframed**: No new query is needed. Stage 1's existing result
  (`Ok(Some)` vs `Ok(None)`) carries the same information. Track it
  through to the outcome decision and no migration / no new index is
  required.

### Implementation outline (this PR)

**Files touched**:
- `crates/mika-agent/src/kg/entity_resolver.rs` — outcome enum +
  `ResolutionResult` enum + result tracking + match arms.
- (Conditional) `crates/mika-agent/src/db.rs` — only if Pin J audit
  reveals a CHECK constraint on `kg_resolutions_log.outcome` that must be
  widened.

**Change list**:

1. Add `pub const NO_CANDIDATE_OF_TYPE: &str = "no_candidate_of_type";` in
   the `outcome` mod (Pin B).
2. Add `ResolutionResult::NoCandidateOfType` enum variant (existing enum
   at `entity_resolver.rs:115–135` or thereabouts — implementer pins
   exact range).
3. In `resolve_single_entity` (Pin C site), record whether Stage 1
   returned `Ok(Some)` via a local boolean (e.g., `stage1_found`). Pass
   it into `disambiguate_with_llm` as a parameter, OR map the outcome
   downstream:
   - If Stage 2 returns `Ok(None)` AND `stage1_found == false` →
     `ResolutionResult::NoCandidateOfType`.
   - If Stage 2 returns `Ok(None)` AND `stage1_found == true` →
     `ResolutionResult::NoMatch` (existing semantic narrowed).
4. In `disambiguate_with_llm`'s `candidates.is_empty()` early return (Pin
   F): change the return signal to indicate "no candidates of type" — the
   caller maps this to `NoCandidateOfType`.
5. Add `apply_result` arm (Pin H) for `NoCandidateOfType` — same shape as
   `NoMatch` but writes `outcome::NO_CANDIDATE_OF_TYPE`. Increment a new
   `stats.no_candidate_of_type` counter.
6. Add `stats.no_candidate_of_type` field to `ResolutionStats` struct;
   include in the `info!` summary log at line ~410.
7. (Conditional, depending on Pin J audit) Migration to widen
   `kg_resolutions_log.outcome` CHECK constraint.

### What this PR does NOT touch

- `kg_entities` table or its indices — no new query, no new index needed.
- `domain_builder.rs` — sole-writer contract preserved.
- `subject_extractor.rs` — extractor prompt untouched (Shape 1 follow-up).
- Historical `no_match` rows — not backfilled (see § Operational notes).

---

## Acceptance criteria

1. New outcome string constant `NO_CANDIDATE_OF_TYPE` is defined in the
   `outcome` mod (Pin B).
2. New enum variant `ResolutionResult::NoCandidateOfType` exists.
3. When Stage 1 returns `Ok(None)` AND Stage 2 returns `Ok(None)`, the
   resolver writes `outcome = 'no_candidate_of_type'` to
   `kg_resolutions_log`.
4. When Stage 1 returns `Ok(Some)` (low-confidence escalation) AND Stage 2
   returns `Ok(None)`, the resolver writes `outcome = 'no_match'`
   (existing semantic narrowed).
5. When `candidates.is_empty()` short-circuits at Pin F, the resolver
   writes `outcome = 'no_candidate_of_type'`.
6. `ResolutionStats.no_candidate_of_type` counter is incremented and
   surfaced in the `info!` summary log.
7. Unit tests cover all three branches:
   - Phantom: Stage 1 `Ok(None)` → Stage 2 `Ok(None)` →
     `no_candidate_of_type`.
   - Disambig failure: Stage 1 `Ok(Some)` (low-confidence) → Stage 2
     `Ok(None)` → `no_match`.
   - Empty type bucket: `candidates.is_empty()` →
     `no_candidate_of_type`.
8. No regression in existing 30+ resolver tests.
9. If Pin J audit reveals a CHECK constraint requiring widening, the PR
   includes the migration; otherwise no DB change.
10. Follow-up ticket for Shape 1 (roster injection) filed at grooming
    close and linked from this ticket's close comment.

---

## Operational notes

**Epoch boundary** (per NF3): Historical `kg_resolutions_log` rows are not
backfilled. Outcome distribution metrics (e.g., "% phantom vs %
disambig-failure") are only meaningful for rows written after this PR
deploys. Any dashboard or audit comparing pre/post-ship distributions
must filter on `created_at >= <deploy timestamp>`.

**Sequencing rationale counter** (per NF1): The argument "if Shape 1
stops phantom production, Shape 3's split becomes mostly empty on the
`no_candidate_of_type` side — was building the instrument worth it?" is
answerable: Shape 3 also serves as a **regression guard** after Shape 1
ships. If a future change degrades the extractor (e.g., model swap,
prompt edit, corpus drift), the `no_candidate_of_type` count rises
visibly. Shape 3 is permanent observability, not a one-shot
measurement-then-discard.

**Shape 1 follow-up scope note** (per NF2): The follow-up ticket's
initial scope statement must include the open question "should `skill:*`
entities participate in the roster constraint, or be allowed
`discovered: true` due to versioning/rename surface?" — this question
should not be lost between grooming sessions.

---

## Open questions for second-pass architect review

**Q-pass2-A**: Is the Stage-1-result-tracking interpretation of the issue
body's split semantics correct? Specifically: does
`no_candidate_of_type` = "Stage 1 returned `Ok(None)` (no exact
`(type, name)` match)" align with the issue author's intent? The
alternative reading is "the LLM saw zero candidates" (Pin F path only),
which would make the new outcome much rarer and not catch the four cited
examples. I lean toward the broader interpretation; want confirmation.

**Q-pass2-B**: F3 — confirm that the no-new-index conclusion is correct.
The reasoning: Pin I's CHECK constraint makes
`LOWER(entity_key) = LOWER(?)` equivalent to case-insensitive
`(type, name)` matching. Stage 1 already does this and returns
`Ok(None)` for misses. The new outcome reuses this signal. If the
architect sees a case I missed where an additional name-based lookup is
needed, please name it.

**Q-pass2-C**: If Pin J reveals the CHECK constraint on
`kg_resolutions_log.outcome` enumerates allowed strings, the migration
to widen it is non-trivial in SQLite (CREATE TABLE _new + copy + drop +
rename, or writable_schema). Is that within scope for this PR, or
should the implementer file a separate "widen CHECK" ticket and gate
this PR on it?

---

## Risks

- **Issue body framing ambiguity**: The issue body's "no_candidate_of_type"
  wording is ambiguous between (a) "no entities of this type at all" and
  (b) "no exact `(type, name)` match." This plan picks (b) based on the
  experiment's findings (which used name-based checking). If architect
  prefers (a), the implementation reduces to just splitting at Pin F's
  `candidates.is_empty()` short-circuit. State the preferred reading in
  pass-2 review.
- **CHECK constraint migration cost**: If Pin J audit shows a CHECK
  constraint, the migration is non-trivial. Plan provides both branches
  (with and without migration); architect can disposition.
- **Anticipatory work with no current consumer**: Decision A removed
  mika-arch's `query_knowledge_graph`. Justification: (a) extraction
  continues regardless and accumulates rows in `kg_subject_entities`;
  (b) Shape 3 is cheap and permanent observability; (c) re-enabling a
  consumer requires this anyway. If architect disagrees, ESCALATE.

---

## Out of scope

- Shape 1 (extractor roster injection) — filed as follow-up ticket at
  grooming close.
- Shape 2 (observation-driven domain emission) — rejected; sole-writer
  contract violation.
- Backfilling historical `no_match` rows to `no_candidate_of_type` (see
  § Operational notes epoch boundary).
- Any change to `query_knowledge_graph` re-enable on mika-arch (Decision
  A stands).
- Subject-extractor LLM model swap (separate experiment).
- Cross-type name-existence diagnostic ("name exists under wrong type")
  — would require a third outcome variant; not in this PR's scope. File
  as separate ticket if metrics demand it post-deploy.

---

## Compounding hooks

1. **Sole-writer contracts are load-bearing**: Pin A's contract was the
   only structural argument against Shape 2. The pattern (doc-comment
   naming the writer + cross-reference to a prior dual-write incident) is
   worth surfacing in `docs/solutions/best-practices/` if not already
   present.
2. **Issue body framing vs code structure**: The issue body's split
   keywords didn't map cleanly to code structure until Phase 0 pinning
   forced reconciliation. Pattern: "always pin the code paths before
   committing to a verbal split-the-outcomes refactor." Worth a
   `docs/solutions/workflow-issues/` entry if it recurs.
3. **CHECK-constraint-as-load-bearing-comment**: Pin B's comment ("matching
   the CHECK constraint on `kg_resolutions_log.outcome`") was the only
   pointer to the schema-level coupling. Without the comment, the
   implementer would miss the migration question. Pattern: "comments
   that name schema invariants live next to the code that depends on
   them." Useful for self-dev compounding.
