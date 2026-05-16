---
ticket: mika#1158
title: Inject closed-set domain roster into KG subject extractor (Shape 1 follow-up to mika#1154)
type: feat
labels: [enhancement, p2-normal, agent-core, kg]
created: 2026-05-16
plan_seq: 003
status: drafted
revision: pass-1-pending
base_commit: 31c1b0a5c12db57e6898c341fdeb3d7adad2c4d9
related:
  - mika#1154 (Shape 3 — `no_match` split; primary fix, merged via #1159)
  - mika#1152 (parent investigation — resolver-sonnet baseline)
  - mika/docs/plans/2026-05-16-002-feat-kg-subject-extractor-roster-plan.md (parent plan, committed at 751dfe30)
  - mika/docs/solutions/kg-investigations/2026-05-16-resolver-sonnet-baseline.md
---

# Plan: Inject closed-set domain roster into subject extractor (mika#1158)

## Recap

The resolver-sonnet baseline (mika#1152) found 0/87 sampled `no_match`
outcomes had a domain-graph counterpart. Four phantom examples cited:
`agent:vincent`, `agent:ci`, `agent:tower_http`, `tool:tailwind`
(operator name, abstract CI role, Rust crate, CSS framework). Shape 3
(mika#1154, shipped via #1159) split `no_match` into `no_candidate_of_type`
+ `no_match` for observability. Shape 1 (this ticket) addresses the
*structural source*: the extractor LLM is roster-blind — `build_extraction_prompt()`
hands the model only an approved-types list and an approved-relationships
list, with no reference to the actually-canonical entities sitting in
`kg_entities`. We close that gap by injecting the live roster into the
extraction prompt and adding a `discovered: true` carveout for clearly-named
non-roster entities that should surface for operator review.

Decision A (2026-05-12, removed `query_knowledge_graph` from mika-arch)
means urgency is anticipatory, not firefighting — there is no current
consumer of resolved entities. Shape 3's permanent observability still
fires if the extractor regresses; Shape 1 (this PR) reduces phantom
production at the source so Shape 3's `no_candidate_of_type` count
trends toward zero on phantom-class examples.

---

## Phase 0 — Pinned source slices

Base commit on this worktree: `31c1b0a5c12db57e6898c341fdeb3d7adad2c4d9`
(`docs(solutions/kg-investigations): resolver sonnet-baseline experiment
ratifies Decision A`).

All file:line refs in this plan are pinned to this SHA. Implementer must
re-pin if the PR rebases past a non-trivial drift.

### Pin A — `subject_extractor.rs:38–47` (approved subject types)

```rust
pub const APPROVED_ENTITY_TYPES: &[&str] = &[
    "skill",
    "tool",
    "agent",
    "problem_type",
    "solution_path",
    "failure_mode",
    "pattern",
    "concept",
];
```

8 types. **Five overlap with domain** (`KG_DOMAIN_ENTITY_TYPES`,
`db/kg_schema.rs:186`); three are *discovered types* (`solution_path`,
`failure_mode`, `pattern`) that the resolver already short-circuits via
`SKIPPED_DISCOVERED_TYPE` (`entity_resolver.rs:62, 496`).

**Why pinned**: this is the partition. The roster constraint applies only
to the five overlapping types. Discovered types are out of scope for
roster matching — they have no domain counterpart by design.

### Pin B — `db/kg_schema.rs:186` (canonical domain types)

```rust
pub const KG_DOMAIN_ENTITY_TYPES: &[&str] = &["skill", "tool", "agent", "problem_type", "concept"];
```

5 types. Authoritative roster scope. **No new const** — the plan reuses
this rather than introducing a parallel "roster-constrained types"
constant. Single source of truth.

### Pin C — `subject_extractor.rs:171–323` (`validate_extraction_output`)

The complete validator. Two existing checks are load-bearing for this
plan:

- Line 183: `if !APPROVED_ENTITY_TYPES.contains(&entity.entity_type.as_str())` —
  type approval check. We add a *roster* check after this for non-discovered
  entities of `KG_DOMAIN_ENTITY_TYPES`.
- Line 194: `if entity.name.contains(':')` — colon rejection. This collides
  with hierarchical domain concept names like `concept:cross-repo:companion-pr-pattern`
  (where `name = "cross-repo:companion-pr-pattern"`). **Open question Q-pass1-A.**

### Pin D — `subject_extractor.rs:803–861` (`build_extraction_prompt`)

Current signature: `fn build_extraction_prompt(&self, annotated_text: &str) -> (String, String)`.
Sync, takes annotated text, returns `(system, user)`. Plan changes this
to take an `&RosterSnapshot` parameter — pre-fetched in `extract_pending`
once per batch, not per-doc — and adds a roster section between the
"Approved entity types" line and "Rules:" line.

**Why pre-fetch and pass in**: `extract_document` is called for every
pending doc in a batch; the roster is identical across all docs in a
single batch (rebuilt only at server boot via `domain_builder.rs`).
Hoisting the fetch to `extract_pending` eliminates per-doc DB roundtrips.

### Pin E — `domain_builder.rs:12–21` (sole-writer contract)

```rust
//! ## Sole-Writer Contract
//!
//! This module is the **sole writer** of entity_keys in the `skill:*`, `tool:*`,
//! `agent:*`, `problem_type:*`, and `concept:*` namespaces.
```

**Why pinned**: This contract is the architectural ground for Shape 2's
historical rejection (in mika#1154's grooming). It also constrains *this*
plan: `discovered: true` subject entities **never** cross into `kg_entities`.
Operator review is visibility, not auto-promotion.

### Pin F — `db.rs:1579–1588` (`kg_entities` DDL)

```rust
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

`kg_entities` is per-DB (one DB per agent container), no `agent_id` or
`docs_root_hash` scoping. Roster query is a simple `SELECT type, name`
filtered by `type IN (...)`.

### Pin G — `db.rs:1617–1631` (`kg_subject_entities` DDL)

```rust
CREATE TABLE kg_subject_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    docs_root_hash TEXT NOT NULL,
    docs_root TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (...),
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (...),
    trace_id TEXT,
    CHECK (entity_key = type || ':' || name),
    UNIQUE (docs_root_hash, entity_key)
);
```

**Why pinned**: the storage shape for discovered entities. Plan adds two
columns via v34→v35 migration: `discovered INTEGER NOT NULL DEFAULT 0`
and `discovery_reason TEXT`. SQLite ALTER TABLE ADD COLUMN with NOT NULL
+ DEFAULT is allowed without rebuild — see "Schema migration" below.

### Pin H — `entity_resolver.rs:56–65` (resolution outcome constants)

```rust
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

Plus the matching CHECK constraint on `kg_resolutions_log.outcome`. Plan
adds one new outcome: `SKIPPED_DISCOVERED_SUBJECT` (parallel to
`SKIPPED_DISCOVERED_TYPE`). Requires CHECK constraint widening
(non-trivial in SQLite — see "Schema migration").

Note: `MATCHED_LLM_DB_FALLBACK` is the precedent we follow — added by
v30→v31 via table rebuild (#874).

### Pin I — `subject_extractor.rs:455` (prompt-build call site)

```rust
let prompt = self.build_extraction_prompt(&annotated_text);
```

In `extract_document`, called per-doc. After this plan: `extract_document`
receives an `&RosterSnapshot` from `extract_pending` and forwards it to
`build_extraction_prompt`. No per-doc DB roundtrip for the roster.

---

## Open questions — disposition

### Q1 (from ticket): Should `skill:*` be in the roster?

**Disposition: YES — include all five `KG_DOMAIN_ENTITY_TYPES` uniformly.**

Reasoning:
- `domain_builder.rs` is the sole writer of `skill:*` and rebuilds the
  roster at every server boot from the live `SkillRegistry`. A renamed
  or versioned skill appears in the roster on the next boot. The
  roster is *not* stale relative to the engine; it is stale relative
  to in-flight skill mutations only (a window measured in seconds).
- The carveout (`discovered: true` with `discovery_reason`) handles the
  legitimate case of "a doc references a skill that doesn't exist yet
  in the registry" — the LLM emits the entity with `discovered: true`,
  it surfaces for operator review, and (if operator confirms) eventually
  the skill is added and a future rebuild catches it.
- Excluding `skill:*` would create an asymmetry the prompt would have
  to explain ("constrain these four types, ignore the fifth") — more
  complex than uniform constraint with carveout.

Operator can disagree by ESCALATE on second pass.

### Q2 (from ticket): Hard filter or soft prior?

**Disposition: SOFT PRIOR with `discovered: true` carveout.**

Hard filter (refuse non-roster outright) is brittle: it loses signal
about clearly-named entities that *should* be promoted. Soft prior
preserves the signal in `kg_subject_entities` while keeping the
sole-writer contract intact (discovered subjects never auto-promote
to `kg_entities`).

### Q3 (from ticket): Prompt size cost

**Disposition: empirical measurement is part of acceptance.** Estimate:

| Type | Count (approx.) | Bytes (estimate) |
|---|---|---|
| `skill:*` | ~30 (bundled + community) | ~600 |
| `tool:*` | ~60 builtins + N MCP | ~1500 |
| `agent:*` | ~11 | ~200 |
| `problem_type:*` | 5 | ~100 |
| `concept:*` | 20 (7 cross-repo + 13 infra) | ~500 |
| **Total** | ~126 entities | ~2900 bytes |

Order-of-magnitude: ~750 tokens added to the system prompt. Negligible
on Claude / GPT-class, possibly meaningful on cheap-tier models with
small context windows. **Acceptance criterion AC-7 requires the PR to
report the measured size for the chosen target model.**

### Q4 (from ticket): Sequencing with Shape 3

**Disposition: satisfied.** mika#1154 / PR #1159 already merged
(verified at grooming time via `gh pr view 1159 --json state`).
Shape 3's `no_candidate_of_type` outcome is now in production and
serves as the regression-guard signal Shape 1's effect would be
measured against.

### Q-pass1-A (new): Hierarchical concept names and the colon-rejection rule

The validator at `subject_extractor.rs:194` rejects entity names
containing `:`. But domain concept entities use hierarchical names like
`concept:cross-repo:companion-pr-pattern` (where `type=concept`,
`name="cross-repo:companion-pr-pattern"`). The subject layer cannot
currently produce a name matching this domain entity.

This is a pre-existing mismatch, not introduced by this plan. **For
this plan's scope**: roster entries for `concept:*` are presented to
the LLM as `concept:<full-name>` strings in the prompt — but the LLM
cannot emit them via `name` because of the validator. Three options:

- **(a) Skip `concept` in roster injection for this PR**, file a
  follow-up to resolve the colon-rejection mismatch. Pro: smallest
  surface, no validator change. Con: doesn't catch concept phantoms,
  which the four examples don't include but future ones might.
- **(b) Relax the validator to permit colons inside `concept` names only.**
  Pro: smallest behavior change that unblocks concept matching.
  Con: special-cases one type; needs a test that subject FKs and
  entity_key formatting still work with colon-bearing names.
- **(c) Use a colon-flattening convention** (e.g., emit
  `cross_repo_companion_pr_pattern`, store as flat name, map at
  resolution). Pro: avoids the schema/validator issue. Con: invents a
  new convention; loses the hierarchical-naming signal.

**Plan's lean: (a) — skip `concept` in roster injection for this PR.**
Defer the colon-rejection fix to a follow-up ticket. Rationale: the
four cited phantom examples (`agent:vincent`, `agent:ci`,
`agent:tower_http`, `tool:tailwind`) are `agent:*` and `tool:*` —
*non-concept types*. Roster-constraining the four non-concept domain
types handles 100% of the named examples while leaving the concept-name
architectural question for an isolated future PR.

**Roster scope adjusted accordingly**: `KG_DOMAIN_ENTITY_TYPES`-minus-`concept`
= `["skill", "tool", "agent", "problem_type"]` (4 types). Concept
remains in `APPROVED_ENTITY_TYPES` — the LLM can still emit `concept`
entities; they just go straight to `discovered: true` (since they can
never roster-match under the current validator).

Architect feedback welcome on whether to pick (b) instead — it's a
small, contained validator relaxation. Q-pass1-A is the explicit
loop-back point.

### Q-pass1-B (new): Discovered storage — column or table?

**Disposition: two new columns on `kg_subject_entities`.**

```sql
ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0;
ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT;
```

Rationale:
- Keeps subject rows colocated; queries that scan subject entities don't
  fork into "regular" vs "discovered" paths.
- The `discovered` flag controls **resolver behavior** (Pin H
  outcome routing) — colocating with the entity is the natural shape.
- Discovery reason is operator-review metadata, lives next to its
  subject row.

Alternative considered: separate `kg_discovered_subjects` table. Rejected
because it requires a JOIN at every read site and forks the sole-writer
contract surface unnecessarily.

### Q-pass1-C (new): Resolver interaction with discovered subjects

**Disposition: discovered subjects skip resolution entirely.**

Add `SKIPPED_DISCOVERED_SUBJECT` to the resolver outcome enum (Pin H)
and the `kg_resolutions_log.outcome` CHECK constraint. When
`entity_resolver` pulls a pending entity and `discovered = 1`, it short-circuits
exactly like `SKIPPED_DISCOVERED_TYPE` — writes the outcome log row and
does no Stage 1 / Stage 2 work.

This **preserves the sole-writer contract** (no resolution row means no
implicit promotion path) and **gives operators a clean review surface**
(`SELECT * FROM kg_subject_entities WHERE discovered = 1`).

### Q-pass1-D (new): Race between domain_builder and extractor

`domain_builder.rs` runs once at server boot before agent init.
`SubjectExtractor` instances are constructed per-agent and run extraction
either at startup (background spawn) or via the 30-min periodic tick
(#1052). The roster is fetched in `extract_pending`; by then,
`domain_builder` has run at least once on this boot.

**Disposition: cache roster per `extract_pending` invocation, no
in-flight refresh.** Each tick's roster snapshot reflects the boot's
domain graph. Skills/tools added after boot won't appear in this tick's
roster — they will appear in the next boot's tick. Acceptable: KG
extraction is a soft, eventual-consistency surface.

Edge case: if `kg_entities` is empty (e.g., fresh DB before
`domain_builder` finishes), `extract_pending` logs a `WARN` (event:
`extraction_empty_roster`) and skips the roster injection for this batch.
The extractor falls back to current behavior (no roster section in
prompt). Validation runs in **lenient mode** — non-roster non-discovered
entities are accepted (because there is no roster to check against).
This matches today's behavior precisely; it's strictly safer than hard-
refusing all entities.

---

## Design

### Roster snapshot

New module-level type:

```rust
/// Snapshot of the canonical domain roster used for extraction-time
/// constraint. Fetched once per `extract_pending` batch.
#[derive(Debug, Clone)]
pub struct RosterSnapshot {
    /// Set of `(type, lowercase_name)` pairs, one per canonical entity
    /// of a roster-constrained type.
    members: HashSet<(String, String)>,
    /// Rendered prompt section, ready to interpolate into
    /// `build_extraction_prompt`. Empty when `members.is_empty()`.
    rendered_section: String,
}

impl RosterSnapshot {
    pub fn contains(&self, entity_type: &str, name: &str) -> bool { ... }
    pub fn is_empty(&self) -> bool { ... }
    pub fn rendered_section(&self) -> &str { ... }
    pub fn member_count(&self) -> usize { ... }
}
```

Roster-constrained types (per Q-pass1-A): `["skill", "tool", "agent", "problem_type"]`.
Concept is *not* roster-constrained in this PR.

### Roster query

```sql
SELECT type, name
FROM kg_entities
WHERE type IN ('skill', 'tool', 'agent', 'problem_type')
ORDER BY type, name
```

Single query per batch. Fetched in `SubjectExtractor::load_roster_snapshot()`,
called once from `extract_pending` before the per-doc loop.

### Rendered prompt section

```
Canonical entity roster — the live set of entities present in this agent's
domain graph. When the document references an entity of one of these types
(skill, tool, agent, problem_type), the entity's name MUST match exactly
one roster entry (case-insensitive). If the document references an entity
of these types whose name is NOT in the roster, emit it with
`"discovered": true` and a `"discovery_reason"` explaining why it should
be added to the canonical set. If you are uncertain, omit the entity.

Entities of type `concept`, `solution_path`, `failure_mode`, `pattern`
are NOT roster-constrained.

Roster:
  agent: mika-arch
  agent: mika-dev
  agent: mika-qa
  agent: mika-relay
  ...
  problem_type: ci_failure
  problem_type: duplicate_pr
  ...
  skill: dev-pilot
  skill: self-dev
  ...
  tool: pr_merge_with_gate
  tool: run_claude_pilot
  ...
```

Position in prompt: between the existing "Approved entity types" /
"Approved relationship types" section and the "Rules:" section. Single
contiguous block.

### Prompt-side schema extension

The schema example in the prompt gains two optional fields:

```json
{
  "entities": [
    {
      "type": "<entity_type>",
      "name": "<lowercase_underscore_name>",
      "description": "<brief description>",
      "chunk_indices": [<int>, ...],
      "confidence": <0.0-1.0>,
      "discovered": <bool, optional, default false>,
      "discovery_reason": "<reason, required when discovered=true>"
    }
  ],
  ...
}
```

### `ExtractedEntity` struct extension

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub chunk_indices: Vec<usize>,
    pub confidence: f64,
    #[serde(default)]
    pub discovered: bool,
    #[serde(default)]
    pub discovery_reason: Option<String>,
}
```

`#[serde(default)]` on both new fields keeps prior corpora and tests
forward-compatible — older LLM outputs without these fields parse as
`discovered=false`, `discovery_reason=None`.

### Validation extension (Pin C)

In `validate_extraction_output`, after the existing type-approval check
(line 183), add:

```rust
let is_roster_constrained =
    matches!(entity.entity_type.as_str(), "skill" | "tool" | "agent" | "problem_type");

if is_roster_constrained {
    if entity.discovered {
        // Discovered path: must have non-empty reason.
        if entity.discovery_reason.as_ref().map_or(true, |r| r.trim().is_empty()) {
            entity_errors.push(format!(
                "entity[{i}]: discovered=true requires non-empty discovery_reason"
            ));
        }
    } else if !roster.contains(&entity.entity_type, &entity.name) {
        // Non-discovered must match roster.
        entity_errors.push(format!(
            "entity[{i}]: type='{}' name='{}' not in canonical roster — \
             either match a roster entry or emit with discovered=true + discovery_reason",
            entity.entity_type, entity.name
        ));
    }
} else {
    // Non-roster-constrained types (concept, solution_path, failure_mode, pattern)
    // are unchanged. discovery_reason still validated when discovered=true.
    if entity.discovered
        && entity.discovery_reason.as_ref().map_or(true, |r| r.trim().is_empty())
    {
        entity_errors.push(format!(
            "entity[{i}]: discovered=true requires non-empty discovery_reason"
        ));
    }
}
```

`roster: &RosterSnapshot` is a new required parameter on
`validate_extraction_output`. The empty-roster branch (Q-pass1-D) is
handled at the *caller* (in `extract_document`): if `roster.is_empty()`,
the validator is called with a sentinel empty `RosterSnapshot` and the
`is_roster_constrained` predicate is effectively bypassed because no
non-discovered entity can fail roster lookup when there are zero entries
to look up.

Actually — to avoid a subtle bug where lenient-mode silently accepts
phantoms, the validator gets a third parameter `roster_enforced: bool`,
defaulted by the caller to `roster.member_count() > 0`. When
`roster_enforced == false`, the roster lookup is skipped (lenient mode).
This makes the lenient-mode intent explicit in the call signature.

### Storage extension (Pin G)

v34→v35 migration in `db.rs`:

```rust
"ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0",
"ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT",
```

Additive, no rebuild. Per Mika convention "additive ALTER TABLE" (see
v28→v29, v30→v31 precedents in `crates/mika-agent/CLAUDE.md`).

Write-site (in `subject_extractor.rs`'s entity upsert): `INSERT` carries
the two new columns from the validated `ExtractedEntity`.

### Resolver extension (Pin H)

v35→v36 migration: widen `kg_resolutions_log.outcome` CHECK constraint
to include `'skipped_discovered_subject'`. Mirrors v29→v30 (#874): SQLite
table rebuild (CREATE TABLE _new + INSERT SELECT + DROP + RENAME +
recreate indexes). Existing v27→v28 / v30→v31 helpers in `db.rs` are the
template.

`entity_resolver.rs`:

- New `outcome::SKIPPED_DISCOVERED_SUBJECT` constant (`"skipped_discovered_subject"`).
- New `ResolutionResult::SkippedDiscoveredSubject` variant.
- `resolve_single_entity()` pre-flight: after `KG_DOMAIN_ENTITY_TYPES`
  filter (existing Pin C from parent plan), check
  `entity.discovered == true` → return `SkippedDiscoveredSubject`. Loaded
  into `PendingEntity` via the existing SELECT statement (just adds the
  column to the projection).
- `apply_result()` arm for `SkippedDiscoveredSubject` — writes
  `outcome = 'skipped_discovered_subject'`, increments
  `ResolutionStats.skipped_discovered_subject` counter.

### Empirical prompt-size measurement (AC-7)

Acceptance includes:
1. Build a representative roster: a populated dev-mode agent's
   `kg_entities` table.
2. Render the new prompt section.
3. Tokenize against the configured extraction model (read
   `MIKA_KG_EXTRACTION_MODEL`, fallback `MIKA_KG_INGESTION_MODEL`).
4. Report in PR description: `roster_entries=N, rendered_chars=X, estimated_tokens≈Y`.

This is a one-shot measurement, not a runtime guard. If `Y` is large
enough to warrant a budget knob, file as a follow-up. For this PR, just
surface the number so future readers and operators see the cost.

---

## Schema migrations

Two migrations, both additive in spirit:

### v34 → v35 (additive ALTER TABLE)

```rust
// db.rs migration block for v35
tx.execute(
    "ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0",
    [],
)?;
tx.execute(
    "ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT",
    [],
)?;
```

Per-column `column_exists` guard pattern (see v30→v31 precedent) for
crash-recovery safety.

### v35 → v36 (CHECK constraint widening — table rebuild)

```sql
CREATE TABLE kg_resolutions_log_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
        'no_match', 'no_candidate_of_type',
        'skipped_discovered_type', 'skipped_discovered_subject',
        'skipped_no_llm', 'error'
    )),
    resolution_trace_id TEXT NOT NULL,
    source_extraction_trace_id TEXT,
    model TEXT,
    duration_ms INTEGER,
    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, subject_entity_id)
);
INSERT INTO kg_resolutions_log_new SELECT * FROM kg_resolutions_log;
DROP TABLE kg_resolutions_log;
ALTER TABLE kg_resolutions_log_new RENAME TO kg_resolutions_log;
-- Recreate indexes.
```

**Includes** `no_candidate_of_type` from #1154 (which already widened the
CHECK at v33 → v34 in #1159's merged PR). Verify the v33→v34 widening
landed before writing this; if not, this v35→v36 widening is the only
needed widening for those two outcomes. (Note: mika-agent CLAUDE.md
schema-version section currently shows v34 as the latest landed; check
whether `no_candidate_of_type` is already in the CHECK at the base
commit before drafting the migration body.)

---

## Files touched

- `crates/mika-agent/src/kg/subject_extractor.rs` — roster snapshot
  type, `load_roster_snapshot()` method, `build_extraction_prompt`
  signature change, validator signature change, upsert column write,
  unit tests.
- `crates/mika-agent/src/kg/entity_resolver.rs` — new outcome variant +
  constant, `PendingEntity` projection update, pre-flight branch,
  `apply_result` arm, stats counter.
- `crates/mika-agent/src/db.rs` — v34→v35 ALTER TABLE migration, v35→v36
  CHECK constraint widening migration (if needed — see note above).
- `crates/mika-agent/src/db/kg_schema.rs` — possibly new helper for
  roster-constrained type partition; documented in module-level doc
  comment.
- `crates/mika-agent/CLAUDE.md` — schema version section: add v34→v35
  and v35→v36 entries.

---

## Acceptance criteria

1. **Roster fetch.** `SubjectExtractor::load_roster_snapshot()` exists,
   queries `kg_entities` for `type IN ('skill', 'tool', 'agent',
   'problem_type')`, and returns a `RosterSnapshot` with one entry per
   row.
2. **Prompt injection.** `build_extraction_prompt(annotated_text, &roster)`
   includes the rendered roster section between approved-types and rules
   sections **when** `roster.member_count() > 0`. Empty roster falls
   through to the prior prompt body.
3. **Schema fields.** `ExtractedEntity` has `discovered: bool` (default
   false) and `discovery_reason: Option<String>` with `#[serde(default)]`
   on both.
4. **Validation — roster matched.** A roster-constrained entity with
   matching `(type, name)` and `discovered=false` validates successfully.
5. **Validation — roster missed without discovered.** A roster-constrained
   entity that does *not* match the roster and has `discovered=false`
   fails validation with a clear error message.
6. **Validation — discovered without reason.** Any entity with
   `discovered=true` and missing/empty `discovery_reason` fails validation.
7. **Validation — discovered with reason.** A roster-constrained entity
   with `discovered=true` and non-empty `discovery_reason` validates
   successfully (even if name does not match roster).
8. **Validation — non-roster-constrained types unaffected.** A `concept`,
   `solution_path`, `failure_mode`, or `pattern` entity validates per
   prior rules regardless of roster contents.
9. **Lenient mode.** When `kg_entities` has zero rows of any
   roster-constrained type, `extract_pending` logs `WARN
   extraction_empty_roster` once per batch, and the validator runs with
   `roster_enforced=false` (no roster lookup, all rows that pass other
   rules accepted).
10. **Storage.** `kg_subject_entities` has `discovered` and
    `discovery_reason` columns (after v34→v35 migration). Discovered
    rows have `discovered=1` and a non-empty `discovery_reason`.
11. **Resolver outcome.** A pending subject entity with `discovered=1`
    produces `kg_resolutions_log.outcome = 'skipped_discovered_subject'`
    and no `kg_subject_resolutions` row.
12. **Sole-writer preserved.** No code path writes `kg_entities` rows
    for discovered subjects. Verified by `domain_builder.rs:12–21`
    contract grep + integration test.
13. **Concept passthrough.** A `concept` entity in extraction output is
    rejected by Pin C's existing colon-rejection rule (no change). The
    Q-pass1-A follow-up is filed at PR close.
14. **Empirical prompt-size report.** PR description includes
    `roster_entries=N, rendered_chars=X, estimated_tokens≈Y` against the
    configured `MIKA_KG_EXTRACTION_MODEL`.
15. **No regression.** Existing 30+ subject_extractor tests pass
    unchanged (forward-compatible `#[serde(default)]` keeps old fixtures
    valid). Existing 30+ entity_resolver tests pass unchanged.
16. **Migration safety.** Mika boot at v34 transitions to v35 → v36
    cleanly; restart at v36 is a no-op. Recovery script (per the v27
    pattern) is not needed because both migrations are additive +
    forward-only.

---

## Operational notes

**Epoch boundary.** Existing `kg_subject_entities` rows have
`discovered=0` and `discovery_reason=NULL` (default). They are *not*
backfilled. Phantom rows produced before this PR remain in the table
with `outcome='no_candidate_of_type'` (post-#1154) or `outcome='no_match'`
(pre-#1154). Operator-side cleanup is out of scope for this PR (file as
follow-up if needed).

**Roster freshness window.** The roster is snapshot per `extract_pending`
invocation, which fires at startup spawn + 30-min periodic tick (#1052).
Skills/tools/agents/problem_types added at runtime (e.g., via
`mika skills install`) appear in the next tick's roster only after the
next server boot (because `domain_builder` is boot-time only). This is
acceptable: the staleness window is bounded by the next restart, and
the `discovered: true` carveout absorbs in-window misses.

**Cost ceiling.** Roster injection adds an estimated ~750 tokens per
extraction LLM call (Q3). Across `MIKA_KG_BATCH_BUDGET=500` per agent
per tick and 48 ticks/day, that's ~36k token-equivalents extra per
agent per day — negligible at OpenRouter cheap-tier pricing.

---

## Open questions for second-pass architect review

**Q-pass2-A**: Is Q-pass1-A's lean (a) — defer concept-name colon
question to a follow-up — the right call, or should the validator
relaxation (option b) ship in this PR? Option (b) is contained but
expands scope.

**Q-pass2-B**: Is the `roster_enforced` boolean parameter on the
validator the right shape, or should "empty roster" be implicit (caller
constructs an empty `RosterSnapshot` and the validator treats empty as
"don't enforce")? The boolean makes the intent explicit at the call
site; implicit-via-empty is less code but harder to grep.

**Q-pass2-C**: Should the periodic tick (#1052) also re-fetch the
roster, or is "roster snapshot is boot-fixed per tick" acceptable?
Current plan caches per-batch. Alternative is to make `load_roster_snapshot`
a hot path called per-tick — adds one DB query per tick, but catches
late-boot writes to `kg_entities` (e.g., if domain_builder is split
into incremental passes in the future).

**Q-pass2-D**: Resolver outcome name — `skipped_discovered_subject` vs
something shorter like `discovered_subject`? Plan uses the longer form
for symmetry with `skipped_discovered_type`. Architect may prefer the
shorter form.

---

## Risks

- **Lenient-mode silence.** If `kg_entities` is unexpectedly empty
  (e.g., `domain_builder` fails silently or a fresh DB before first
  rebuild), the validator silently accepts all entities. The `WARN`
  log is the only signal. Operator must watch for
  `extraction_empty_roster` events. Mitigation: add the event to the
  observability signals list in `crates/mika-agent/CLAUDE.md` § Post-restart
  safety check.
- **LLM prompt-adherence drift.** The `discovered: true` carveout
  relies on the LLM following the prompt instruction to emit
  `discovered=true` rather than fabricate a roster-shaped entity to
  satisfy the constraint. Defense: validator hard-rejects non-roster
  non-discovered entities; semantic retry path already exists (Pin from
  `call_llm_with_retry`).
- **Concept passthrough confusion.** Per Q-pass1-A, concept entities
  go straight to `discovered=true` (or fail the colon-rejection rule).
  This is the same behavior as today, but documented explicitly in the
  prompt. If the LLM is confused by the asymmetry, validation will
  catch it (filter to warnings, not whole-batch rejection).
- **CHECK constraint migration.** v35→v36 requires a table rebuild.
  Precedent exists (v29→v30) and the table is small (one row per
  resolution). Low risk, but operator must verify post-deploy.

---

## Out of scope

- Concept colon-rejection fix (filed as Q-pass1-A follow-up at PR close).
- Operator-side review UI for discovered subjects (separate ticket;
  filed at PR close if not already present).
- Re-running extraction on existing rows after this PR ships (epoch
  boundary — see Operational notes).
- Per-corpus roster scoping (`kg_entities` is per-DB, not per-corpus
  today; multi-corpus roster fan-out, if ever needed, is a separate ticket).
- Promotion path from discovered to canonical (sole-writer contract
  forbids; operator action via `domain_builder` rebuild after registry
  changes is the canonical promotion).
- Subject-extractor LLM model swap (separate experiment).

---

## Compounding hooks

1. **Roster-as-prompt-constraint pattern.** This PR is the first
   instance of "inject a structural roster from the domain graph back
   into the prompt that produces subject-layer rows." If the pattern
   recurs (e.g., for relationship roster or chunk-context roster),
   compound to `docs/solutions/architecture-patterns/`.
2. **Soft-prior with carveout.** The `discovered: true` shape is a
   reusable pattern for "constrain LLM output to a closed set, but
   preserve signal about out-of-set candidates." Worth a solutions
   entry if the next reviewer says "yeah we want this in two other
   places."
3. **Lenient-mode boot race.** The `extraction_empty_roster` WARN +
   lenient fallback is the same shape as several other "is the registry
   warm yet?" checks in the engine. Pattern worth surfacing if not
   already documented.
