---
title: "Layer 2 structured facts vs. MemPalace temporal-triple KG: schema audit and migration evaluation"
date: 2026-05-06
revised: 2026-05-07
revision_note: "Revised twice on 2026-05-07. First revision: sharpened the category-error framing as the headline argument; dropped three speculative additive proposals (events temporal columns, kg_relationships temporal columns, temporal_facts view) into a Considered-and-deferred section; promoted the idempotency-on-uniqueness pattern to a first-class recommendation. Second revision (later same day): demoted idempotency back into Considered-and-deferred after re-reading docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md confirmed the duplicate-after-compaction failure mode is already closed via three-layer defense (prompt instruction + DB UNIQUE partial indexes + tool-level constraint catching). A helper would be a DRY refactor with no failure-mode closure, which doesn't clear the project's YAGNI bar. The audit's recommendation collapses to: reject the migration, ship nothing. The category-error reasoning in the headline is unchanged. Pre-revision shapes preserved in git history."
category: audits
module: agent-core
component: layer-2-memory
status: evaluative — no implementation in this pass
related:
  - mika/docs/memory-classification.md
  - mika/docs/architecture/CLAUDE.md (Three-Layer Memory Model)
  - mika/docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md
  - mika-platform/docs/brainstorms/2026-05-06-mempalace-vs-mika-arch-context-leakage-assessment.md
  - mempalace v3.3.3 (workspace-untracked at mika-platform/mempalace/)
---

## TL;DR

**Do not migrate Layer 2 (people / commitments / preferences / events) to a uniform `(subject, predicate, object, valid_from, valid_to)` triple model.** The migration would be a **category error**: paying the system-wide price of triple-store generality for one table that wants temporal validity (`events`) while sacrificing constraint-bearing typed schemas on three that don't.

**Nothing ships from this audit.** Five candidate changes were considered (events temporal columns, kg_relationships temporal columns, a `temporal_facts` view, a confidence column on Layer 2, an idempotency-on-uniqueness helper); each is documented in **§ Considered and deferred** with the reason it didn't clear the project's YAGNI bar today. The audit's value is the design-context paper-trail, not a dispatch — when a future ticket touches Layer 2 schema or memory architecture, the analysis here pre-answers the obvious *"should we adopt MemPalace's pattern?"* question that would otherwise be re-derived under load.

## The category-error argument

Triple stores earn their keep when the entire domain wants temporal-validity semantics — Datomic shops accept losing application-layer constraint enforcement because they get bitemporal correctness *across the whole domain* in return. The trade is system-wide: pay the cost of weakening typed constraints once, gain time-travel queries everywhere.

mika's Layer 2 doesn't have that shape. Three of the four tables encode workflow semantics that a triple substrate would shed:

- **`commitments.status`** is a database-enforced state machine. The `CHECK (status IN ('pending','completed','cancelled'))` constraint catches invalid values at write time. The partial-unique index `idx_commitments_unique_pending ON (agent_id, description, due_date) WHERE status='pending'` enforces "at most one open commitment per (agent, description)." The `update_fact` tool's enum-guarded transition (`update_fact.rs:42`) is the **single chokepoint** through which status changes flow. In a triple store, all three become application-layer enforcement: a buggy `add_triple(commitment:42, has_status, peding, valid_from=now)` succeeds at the substrate level, fails silently at the agent level, and surfaces three months later as a degenerate timeline view.
- **`people.mention_count`** is an aggregate incremented on upsert, indexed by `(agent_id, canonical_name) UNIQUE`. The recency × frequency signal is what feeds the `key_people` core-memory block in the system prompt. Triple form sheds the cheap aggregate; recovering it requires materialized views.
- **`preferences`** is `(agent_id, category) PK` k/v lookup — natural read pattern is *"what's their preferred X?"*. Triple form is isomorphic but slower (predicate scan vs PK lookup) and gains nothing unless you also want *"who else prefers X?"* — a question mika doesn't ask today.

Only **`events`** has a temporal-validity semantic that the typed schema undershoots: a single `event_date TEXT` column can't represent multi-day spans. That is one table out of four.

Trading three constraint-bearing typed schemas for one temporal win is the wrong direction. The migration is a category error because it applies a system-wide pattern to a partial fit.

A secondary argument: even if the migration were attempted, the most likely mid-migration discovery is that a typed-row read view always needs to exist *over* the triple substrate — at which point the migration has reinvented Layer 2 with a triple substrate underneath. Backfilling `commitments` to triples requires re-encoding the status state machine as competing `(commitment:N, has_status, X) valid_from=Y valid_to=Z` triples. The CHECK constraint and partial-unique index do not survive that encoding without being rebuilt in application code as predicate-level uniqueness checks the caller has to remember to write.

## Today's Layer 2 schema (mika)

`crates/mika-agent/src/db.rs:1283-1326`. Four tables, one per category, plus a parallel `search_content` index for FTS5 + sqlite-vec hybrid search.

| Table | PK / Uniqueness | Temporality columns | Mutable state | Indexes |
|---|---|---|---|---|
| `people` | `(agent_id, canonical_name)` UNIQUE | `first_mentioned`, `last_mentioned` | `mention_count` (incremented on upsert) | implicit on UNIQUE |
| `commitments` | autoincrement `id` + partial-unique `(agent_id, description, due_date) WHERE status='pending'` | `created_at`, `due_date`, `completed_at` | `status` ∈ {pending, completed, cancelled} | `(agent_id, status)` |
| `preferences` | `(agent_id, category)` PK | `updated_at` | value (string) — UPSERT | implicit |
| `events` | autoincrement `id` | `event_date`, `created_at` | none | none |

Agent-callable surface (`crates/mika-agent/src/tools/store_fact.rs`, `update_fact.rs`, `search_memory.rs`):

- `store_fact { category, ... }` — JSON dispatch on `category` ∈ {person, commitment, preference, event}. Each branch upserts via category-specific helpers.
- `update_fact { id, category, updates }` — currently scoped to `commitment.status` transitions only (`update_fact.rs:23` description: *"Currently supports updating commitment status"*).
- `search_memory { query, category }` — hybrid FTS5 + sqlite-vec across the four indexed categories plus reminders + `core_memory` (with redirect-redundancy guard for `core_memory`).

## MemPalace's KG schema

`mempalace/mempalace/knowledge_graph.py:63-97`. Two tables — `entities` and `triples`. SQLite, WAL mode, separate DB file at `~/.mempalace/knowledge_graph.sqlite3`.

```sql
CREATE TABLE entities (
    id TEXT PRIMARY KEY,           -- lowercased, underscore-joined (e.g. "max", "alice_smith")
    name TEXT NOT NULL,
    type TEXT DEFAULT 'unknown',
    properties TEXT DEFAULT '{}',  -- JSON blob
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE triples (
    id TEXT PRIMARY KEY,           -- "t_<sub>_<pred>_<obj>_<sha256:12>"
    subject TEXT NOT NULL,         -- FK entities(id)
    predicate TEXT NOT NULL,       -- lowercased, underscore-joined
    object TEXT NOT NULL,          -- FK entities(id)
    valid_from TEXT,               -- ISO date or NULL
    valid_to TEXT,                 -- ISO date or NULL (NULL = currently valid)
    confidence REAL DEFAULT 1.0,
    source_closet TEXT,            -- provenance: which closet
    source_file TEXT,              -- provenance: which file
    source_drawer_id TEXT,         -- provenance: which drawer (RFC 002 §5.5)
    adapter_name TEXT,             -- provenance: which extraction adapter
    extracted_at TEXT DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_triples_subject ON triples(subject);
CREATE INDEX idx_triples_object  ON triples(object);
CREATE INDEX idx_triples_predicate ON triples(predicate);
CREATE INDEX idx_triples_valid   ON triples(valid_from, valid_to);
```

Write surface: `add_entity()`, `add_triple()` (idempotent — returns existing `id` if `(subject, predicate, object) WHERE valid_to IS NULL` already exists; see `knowledge_graph.py:189-196`), `invalidate()` (sets `valid_to`).

Read surface: `query_entity(name, as_of, direction)` — outgoing/incoming/both, with point-in-time filter. `query_relationship(predicate, as_of)`. `timeline(entity_name)` — ordered by `valid_from ASC NULLS LAST`.

Design tells:

- **Single uniform shape.** Every fact is a triple. No category dispatch, no per-type tools.
- **Append-only with logical invalidation.** Setting `valid_to` doesn't delete; superseding facts keep history queryable.
- **Provenance is first-class.** Four columns dedicated to *"where did this come from"* (closet/file/drawer/adapter) — MemPalace's job is bridging verbatim conversation prose → structured facts and answering *"where in the actual transcript was this said?"*.
- **No state machine, no aggregates.** Counting mentions, tracking commitment status, enforcing transition rules — all foreign to this model. They'd be application-code concerns sitting above the substrate.

## The third graph in the room: mika's existing KG

mika already has a graph layer at v25–v31 (`crates/mika-agent/src/db/kg_schema.rs`, `crates/mika-agent/src/kg/`). Three layers:

- **Domain graph** — `kg_entities` + `kg_relationships`. Deterministic startup-time builder projects skills/tools/agents/problem_types/concepts. Edges: `DEPENDS_ON`, `PROVIDES`. **Not temporal.**
- **Subject graph** — `kg_subject_entities` + `kg_subject_relationships`. LLM-extracted from `docs/solutions/**/*.md`. Edges: `SOLVED_BY`, `USES`, `CALLS`, `INDICATES`, `PREVENTS`, `CAUSED_BY`, `MENTIONS`. **Not temporal.**
- **Resolution layer** — `kg_subject_resolutions` + `kg_resolutions_log`. Bridges subject → domain.

Both edge tables are non-temporal (verified by grep against `kg_schema.rs`). The KG's purpose is reference-architecture queries (*"what skill provides this tool?"*, *"what solutions reference this failure mode?"*) — point-in-time questions weren't the design driver.

What this matters for: mika **already has a graph shape for relational facts**, separate from Layer 2. The temporal-columns question for `kg_relationships` is a question about the existing KG, not about Layer 2 → triples. It is also currently a question without a write-side counterpart: there is no agent-callable `add_relationship` tool. The graph is written deterministically at startup (domain) or asynchronously by the LLM extractor (subject). No agent-triggered relational-write surface exists today.

That gap — *no agent-callable relational write surface anywhere in mika* — is real but orthogonal to this audit. It's deferred to its own brainstorm if a concrete need surfaces. As of 2026-05-07 the need is anticipatory, not felt.

## Comparison by use case

| Question the agent wants to answer | Best home today | Best home if migrated |
|---|---|---|
| "Do I owe Alice a deliverable next week?" | `commitments` indexed on `(agent_id, status, due_date)` | Triples: subject=user, predicate=owes, object=alice, valid_from + due_date as separate column. **Worse** — loses status state machine. |
| "What channel does the user prefer?" | `preferences[channel]` PK lookup | Triples: subject=user, predicate=prefers_channel, object=telegram. **Comparable**, slower (predicate scan vs PK). |
| "Who am I talking with most often this week?" | `people` ORDER BY `mention_count` DESC, `last_mentioned` filter | Triples: would require materialized aggregate. **Worse** — loses cheap aggregate. |
| "What did Vincent say about the auth migration last March?" | `events` LIKE / FTS — but no good answer for date-range bounded questions | Triples with `valid_from` / `valid_to`. **Better** — the one clear win. |
| "Who is Alice's manager, as of 2026-01-15?" | Not supported (no relational facts in Layer 2) | Triples or `kg_relationships` + `valid_from` / `valid_to`. **Better in either**, but the existing KG is the natural home, not Layer 2. |
| "What was true about Max in January?" | Not supported | Triple temporal query. **Better** — but is this a use case mika has?

Three of five representative questions are answered better by today's typed-row design because the questions encode workflow semantics (state machine, aggregate) that triples would shed. The bottom two questions are the win-condition for triples, and they cluster on relational + temporal — which is the existing KG's territory, not Layer 2's.

## Considered and deferred

Five candidate changes appeared across drafts of this audit. Each has been re-evaluated against the project's YAGNI bar and deferred. They are recorded here because the design context is fresh — when a future ticket surfaces a concrete need, the analysis is already done.

### Per-category idempotent upsert helper (deferred)

The single borrowable idea from MemPalace is the **read-then-write idempotency pattern** on `add_triple` (`mempalace/mempalace/knowledge_graph.py:189-196`): read existing row by natural key; return its id if found, otherwise insert. Schema-neutral; doesn't require triples.

A per-category helper in `crates/mika-agent/src/db.rs` would replace ad-hoc UPSERT logic across the four Layer 2 write paths in `store_fact.rs` with one signature shape per table.

**Why deferred:** the duplicate-after-compaction failure mode (`docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md`) is already closed three layers deep as of March 2026:

- **Layer 1** — system-prompt instruction telling the agent to check existing state before writing.
- **Layer 2** — DB UNIQUE partial indexes preventing exact duplicates: one-shot reminders `(agent_id, label)`, dated events `(agent_id, description, event_date)`, pending commitments `(agent_id, description, due_date) WHERE status='pending'`. People are protected by `(agent_id, canonical_name) UNIQUE`; preferences by their PK.
- **Layer 3** — tool-level constraint catching that turns DB violations into informational `"already exists"` returns rather than errors.

A helper today would be DRY-shaped: signature consistency across four call sites + explicit *"return existing id on duplicate"* instead of catch-violation-and-pretend-success. Both are real refactor benefits but neither is failure-mode closure. The four call sites aren't fragile, aren't blocking new work, and `store_fact`'s contract doesn't change — the win is invisible to the agent.

When a felt pain surfaces (e.g., an agent skill that wants to chain operations on the existing id and currently can't because `store_fact` returns informational success without surfacing the row id), file then. Until that pain is concrete, refactor-shaped tickets fail the project's YAGNI bar.

### `events.valid_from` / `events.valid_to` (deferred)

`events.event_date` is a single nullable date; multi-day episodes can't be represented faithfully. Adding `valid_from` / `valid_to` would make `events` the one Layer 2 type whose schema actually fits the temporal-validity semantic.

**Why deferred:** no current read path against the events table degrades because of the missing span. `search_memory` runs FTS + vector, not date-range temporal queries. No agent skill, heartbeat, summary, or timeline rendering today asks *"what events overlapped this date range?"* — the schema spend would precede its first reader. Ship when a concrete reader exists; the reader's filter/projection/sort shape will inform the column design better than speculation.

### `kg_relationships.valid_from` / `valid_to` (deferred)

Temporal-validity columns on the existing KG would let mika answer *"who was Alice's manager as of 2026-01-15?"* — but the question presupposes an agent-callable relational write surface that does not exist today. Adding temporal columns to a table the agent can't write to fixes nothing.

**Why deferred:** the underlying gap is "no agent-callable relational-write surface anywhere in mika." That's a real design question (table-vs-tool design, write-channel choice, identity-vs-temporal-fact split) that warrants its own brainstorm — not a pre-emptive schema addition. Surface a concrete agent need first.

### `temporal_facts` view (deferred)

A `UNION ALL` view over commitments + events would give future timeline queries one read surface. The view is cheap (DDL-only, no storage cost).

**Why deferred:** dead code without a reader. The argument *"any future timeline query has one read surface"* is structural-change-for-imagined-future. When the first reader needs a unified timeline, the view design will be informed by what the reader actually queries (filters, projections, sort) instead of being the union the audit author happens to think of today. The cost of getting the view shape wrong now and having a reader build around it is higher than the cost of designing it later.

### Confidence column on Layer 2 (deferred)

MemPalace's `triples.confidence REAL DEFAULT 1.0` reflects that triples come from extraction adapters of varying reliability. mika's Layer 2 has no equivalent column anywhere.

**Why deferred:** mika's Layer 2 write paths are agent-triggered with implicit confidence-1.0 semantics. The act of calling `store_fact` *is* the confidence assertion. Adding a `confidence` column the agent has to populate creates a new prompt-engineering surface (when does the agent emit 0.7 vs 1.0?) without a clear consumer. Confidence as a column belongs to extractor pipelines (`kg_subject_relationships.confidence` already exists for that reason — `mika/CLAUDE.md` § KG Subject Extractor). Until Layer 2 acquires an extractor write path, a confidence column is one nobody knows how to fill.

## Migration path if a full triple model is ever adopted (appendix)

Recorded for completeness; the audit recommends against pursuing this. If a future ticket overrides the recommendation:

1. **Define a `facts_triples` table** in mika's main DB. Single SQLite per customer is a load-bearing platform invariant per `mika/CLAUDE.md` — must not be a separate file.
2. **Backfill from existing tables** with category-specific projections (`people` → has_relationship/has_notes triples; `commitments` → has_description/has_status triples with status as a temporal sequence; `preferences` → prefers_<category> triples; `events` → occurred triples with valid_from = event_date).
3. **Dual-write window**: `store_fact` and `update_fact` write both old and new for ≥1 release; migrations land safely behind a flag.
4. **Read switch**: `search_memory` reads triples first, falls back to typed tables on miss.
5. **Drop typed tables** after a release of clean dual-write metrics.

Step 2 alone requires re-encoding the commitment state machine into a sequence of competing `has_status` triples — losing the CHECK constraint, the partial-unique index, and the `update_fact`-enforced enum. The likely mid-migration discovery is that step 4 is a layer that *always* needs to exist (a typed-row read view *over* triples), at which point the migration has reinvented Layer 2 with a triple substrate underneath. This is the strongest argument against pursuing the migration.

## Risks and notes

- **MemPalace as a dependency is a separate question.** This audit is about whether to adopt MemPalace's *schema pattern* in mika's own DB. The question of whether to actually depend on `mempalace` as a Python library is covered by `mika-platform/docs/brainstorms/2026-05-06-mempalace-vs-mika-arch-context-leakage-assessment.md`. Vendor risk + cross-process boundary + storage-substrate doubling apply there, not here.
- **MemPalace's provenance columns reflect its mining pipeline.** `triples.source_closet` / `source_file` / `source_drawer_id` / `adapter_name` exist because MemPalace bridges verbatim transcripts → triples via adapters. mika's Layer 2 facts are agent-asserted via tools, not mined; the provenance shape that fits is `audit_events` (already exists, captures `trace_id` and reasoning per write — see `crates/mika-agent/src/db.rs:1329-1345`). Adopting MemPalace's provenance columns 1:1 would create a parallel audit trail with no engine support.
- **`memory-classification.md` is the contract here.** Layer 2 is classified agent-triggered (LLM decides write/read), not deterministic. A schema migration that breaks the existing `store_fact` / `update_fact` / `search_memory` agent-callable surface is more disruptive than the schema change it embodies. The idempotency-helper recommendation preserves all three tools' shapes.

## Citations

- `crates/mika-agent/src/db.rs:1283-1326` — Layer 2 table DDL
- `crates/mika-agent/src/db.rs:1329-1345` — `audit_events` table (existing provenance surface)
- `crates/mika-agent/src/tools/store_fact.rs:11-89` — category-dispatch tool
- `crates/mika-agent/src/tools/update_fact.rs:11-56` — commitment-status-only update tool
- `crates/mika-agent/src/tools/search_memory.rs:14-101` — hybrid search read surface
- `crates/mika-agent/src/db/kg_schema.rs` — existing KG schema (no temporal columns; verified by grep)
- `mempalace/mempalace/knowledge_graph.py:63-97` — MemPalace KG DDL
- `mempalace/mempalace/knowledge_graph.py:189-196` — idempotency-on-uniqueness pattern (the borrowable idea)
- `mempalace/mempalace/knowledge_graph.py:240-326` — `query_entity` / `query_relationship` (with `as_of` filter)
- `mika/docs/memory-classification.md` — deterministic vs. agent-triggered taxonomy
- `mika/docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md` — three-layer defense already deployed (Mar 2026); confirms the idempotency helper would be DRY-shaped, not structural
- `mika/CLAUDE.md` § Three-Layer Memory Model — current Layer 1/2/3 contract
