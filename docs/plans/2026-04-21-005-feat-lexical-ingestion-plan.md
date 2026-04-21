---
title: "feat: lexical graph ingestion — chunk solution docs, embed, link via search_content"
type: feat
status: active
date: 2026-04-21
---

# Lexical graph ingestion — chunk solution docs, embed, link via search_content

## Overview

Populate the lexical layer of Mika's Knowledge Graph (milestone mika#14, ticket mika#689). For each agent, scan `docs/solutions/**/*.md`, chunk with markdown-aware splits, and produce `kg_chunks` rows + `search_content` + FTS5 + async embedding rows via the existing Layer 3 pipeline. Writes are per-agent (per #686 D1), idempotent by content hash, and ordered after the domain graph rebuild so downstream linkage tickets (#690, #691) have consistent state to consume.

No LLM calls in #689. No direct chunk→domain entity linkage at ingest time (per #686 D9 — `kg_chunks.entity_id` does not exist; linkage goes through subject → resolution). Chunks are pure lexical units; their connection to the domain graph happens in #690/#691.

## Problem Frame

#686 landed the schema and #687 populated the domain graph. Neither gives agents semantic access to institutional knowledge — the `docs/solutions/**` tree and `/ce:compound`-produced docs are currently searchable only via grep. The lexical layer makes them first-class KG citizens: chunked, embedded, indexed, queryable via hybrid search alongside Layer 2 memory.

The payoff is that #690's subject extraction has structured chunk rows to operate on (instead of re-reading docs from disk every time), and #692's `self-knowledge` upgrade can surface documented lessons with proper retrieval instead of static lookup.

## Requirements Trace

- R1. Startup-time per-agent ingestion of `docs/solutions/**/*.md`.
- R2. Compound-hook ingestion: `/ce:compound` writing a new doc triggers immediate ingestion for the authoring agent only.
- R3. Markdown-aware chunking: 2000 chars with 200-char overlap, split on `---` (frontmatter) and `##` (section headers).
- R4. Content-hash-based idempotency: unchanged docs on re-run are a no-op.
- R5. Deletion handling: docs removed from disk produce symmetric cleanup of `kg_chunks` + `search_content` + FTS5 + vec entries.
- R6. Composed write contract: `kg_chunks` + `search_content` committed in a single transaction, embeddings backfilled async (per conventions C1.1).
- R7. Ordering: ingestion runs strictly after #687's domain rebuild in the startup sequence.
- R8. Observability: per-agent ingestion durations and chunk counts logged for future optimization data.

## Scope Boundaries

- Scan and ingestion of `docs/solutions/**/*.md` per agent.
- Compound hook that calls into ingestion when `/ce:compound` writes a new doc.
- Deterministic chunking (2000 chars, 200 overlap, markdown-aware).
- Doc-level content-hash idempotency.
- Composed write: `kg_chunks` row + `index_content()` call + transactional atomicity.
- Symmetric deletion handling for removed docs + `unindex_content()` helper if not already present.
- Per-agent per-ingestion-run log lines for observability.

### Deferred to Separate Tasks

- Subject graph extraction from chunk text (NER + fact triples): **mika#690**.
- Entity resolution (subject → domain linkage, the canonical chunk→domain path): **mika#691**.
- Query tool consuming KG chunks: **mika#688**.
- `self-knowledge` upgrade using KG chunks: **mika#692**.
- Shared-chunk optimization for `docs/solutions/**` (single chunk row for N agents instead of N copies): **deferred as a potential future optimization**, not a design question. See D3 — framed explicitly as "accept duplication now, optimize with real data if it ever matters."
- Ingestion of other doc trees (`docs/adr/`, `docs/architecture/`, `docs/plans/`): out of scope. If/when needed, extend the scan path list in a small follow-up.
- Chunk-level content-hash invalidation (vs the doc-level default): deferred, see D4 — only makes sense if specific content classes are large and frequently edited.

## Context & Research

### Cross-cutting conventions

This plan cites `docs/architecture/kg-implementation-conventions.md` as the authoritative source for cross-cutting decisions. Sections that apply to #689:

- **C1.1 (async-embedding contract):** Mandatory. Ingestion commits `kg_chunks` + `search_content` + FTS5 synchronously in a single transaction. Embeddings are generated asynchronously by the existing backfill path. Callers may see FTS5-only results in the window between ingestion commit and backfill catchup.
- **C3.2 (observability — lexical ingestion):** Per-document audit_events (NOT per-chunk), `tool_name: "ingest_document"`, target_key `kg_chunk:<source_doc_path>`, counts in before_value/after_value.

Sections C2 (non-interactive LLM) doesn't apply — #689 does no LLM calls.

### #686 and #687 dependencies

- **#686 schema decisions used:** `kg_chunks` shape (no `entity_id` per D9; `source_doc_hash NOT NULL` per D10; UNIQUE(agent_id, source_doc_path, seq_id) per D7; trace_id per D6).
- **#687 dependency:** Domain rebuild must complete before per-agent lexical ingestion starts in the startup sequence. Ingestion doesn't FK into `kg_entities` directly (per D9 — no entity_id column), so this is a soft ordering rather than a hard constraint — but keeping it ordered preserves the "each layer's startup work composes cleanly" invariant for #690+'s benefit.

### Relevant Code and Patterns

- **Existing indexing pipeline:** `crates/mika-agent/src/db.rs:6107` (`index_content()`) — the write helper that `search_content` and `fts_search` go through for all source types. `#689` adds `source_type="kg_chunk"` as another caller. Backfill of embeddings is already hooked via startup path.
- **`search_content` schema:** `crates/mika-agent/src/db.rs:1049-1059`. Current `source_type` values: `person`, `commitment`, `preference`, `event`. `source_id` is INTEGER; for `kg_chunk` it's `kg_chunks.id`.
- **Embedding client:** `crates/mika-common/src/embedding.rs`. OpenAI text-embedding-3-small, 512 dims. Not called directly by #689 — embeddings land via backfill.
- **No existing `unindex_content()`:** grep for `unindex_content` returns nothing. #689 introduces it as a symmetric helper to `index_content()`. Delete-from-kg_chunks path uses it to also delete from search_content + fts_search + vec_search in the same transaction.
- **Solution-doc structure:** `docs/solutions/**/*.md` files have YAML frontmatter (title, category, date, tags, severity, modules_affected) followed by `---` separator and markdown body with `##` section headers. Sample: `docs/solutions/database-issues/iso8601-timestamp-migration.md`.

### Institutional Learnings

- `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md` — Sole-writer designation: #689 is the sole writer of `kg_chunks` rows and of `source_type="kg_chunk"` entries in `search_content`. No other path should write these.
- `docs/solutions/database-issues/iso8601-timestamp-migration.md` — ISO 8601 timestamps; all `created_at` columns use `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`.
- `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — One trace_id per ingestion invocation; stamped on every `kg_chunks` row in that batch for post-hoc investigation.
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — Column list constants, no `SELECT *`. `KG_CHUNK_COLUMNS` lives in `kg_schema.rs` from #686.
- `docs/solutions/logic-errors/startup-backfill-skips-embedding-generation.md` — Backfill is idempotent by `embedding_json IS NULL`; any new `source_type` inherits the existing backfill path automatically.

## Key Technical Decisions

### D1. Startup scan per agent + compound hook (authoring agent only)

Per Vincent's feedback on the trigger model. On server startup, after `apply_overrides()` and after #687's domain rebuild completes, for each agent in `agent_configs`: scan `docs/solutions/**/*.md`, chunk new/changed docs (per D4 idempotency), write chunks.

Additionally: `/ce:compound` writes a doc → the compound handler invokes ingestion for **the authoring agent only**. This keeps freshly-compounded lessons immediately queryable by the agent that just wrote them without waiting for restart. Peer agents see the new doc at next restart (bounded staleness per #687 D4 pattern).

Failure policy for compound hook: **fail-silently with warn log**. If the ingestion call fails (DB error, whatever), log `warn!(event="compound_ingest_failed", doc=<path>, error=%e)` and continue. The doc is on disk and will be picked up at next restart, so ingestion is eventually consistent. The authoring agent's UX tolerates "the doc I just wrote isn't searchable yet, but will be after restart at latest" — acceptable per the C1.1 best-effort policy.

### D2. Chunking: 2000 chars, 200 overlap, markdown-aware boundaries

Per ticket body. Chunker:
1. Splits on frontmatter-ending `---` first (so frontmatter is its own chunk or merged with the preamble).
2. Within body, splits on `##` section headers.
3. Further splits any section over 2000 chars into windows of 2000 with 200-char overlap.
4. Assigns monotonic `seq_id` starting at 0 for each doc.

Chunking is deterministic: same input → same chunks. This is load-bearing for D4's content-hash idempotency (changing the chunker algorithm requires re-ingestion of all existing docs; chunker stability is part of the write contract).

Chunker lives in a standalone module (`crates/mika-agent/src/kg/chunker.rs`). No external dependencies beyond `regex` (already in Cargo.toml).

### D3. Per-agent ingestion of shared docs: accept duplication, instrument for future data

Per #686 D1, `kg_chunks.agent_id` is NOT NULL. For `docs/solutions/**/*.md` (shared docs on disk), every agent gets its own chunk rows — N copies of byte-identical text, N embeddings.

**This is accepted as current behavior, not defended as the final architecture.** The embedding cost is trivial (~$0.02 for 200 docs × 500 tokens × 10 agents = 1M tokens at $0.02/1M). Storage is similarly cheap. The system stays architecturally simple — every chunk query is a single agent-scoped lookup, no special cases.

**Framed as a deferred optimization, not an architectural position.** If KG storage or embedding cost ever surfaces as a real problem (e.g., container scales to 100+ agents, or solution-doc tree grows 10x), the optimization is to make `agent_id` nullable and treat NULL as "shared chunk, visible to all agents." That's a #686-schema-level change, not a #689 concern, and should be driven by actual usage data rather than speculation.

Observability hook (per R8, C3.2): every per-agent ingestion run logs `event=ingest_duration agent=<id> chunks_added=<N> chunks_deleted=<M> docs_scanned=<K> duration_ms=<T>`. These numbers are the input to the future optimization decision, if it ever becomes one.

### D4. Doc-level content-hash idempotency

Per #686 D10, `kg_chunks.source_doc_hash TEXT NOT NULL` stores a SHA-256 hash of the source doc content after normalization:

1. Strip UTF-8 BOM if present.
2. Normalize line endings to LF (replace `\r\n` and `\r` with `\n`).
3. Strip trailing whitespace from each line.
4. Enforce single trailing newline (strip all trailing `\n`, then append exactly one).
5. SHA-256 over the normalized byte sequence.

Algorithm per doc on ingestion:

1. Read doc from disk, normalize, compute hash.
2. Query: `SELECT DISTINCT source_doc_hash FROM kg_chunks WHERE agent_id = ? AND source_doc_path = ?`.
3. If result is `{computed_hash}` (single match) → skip: doc is unchanged.
4. If result is `{}` → new doc, proceed to chunk + ingest.
5. If result is `{different_hash}` or multiple hashes (shouldn't happen given UNIQUE, but defensive) → delete all chunks for (agent_id, source_doc_path), chunk, re-insert.

**Doc-level granularity (not chunk-level)** chosen deliberately. Chunk-level would only re-embed actually-changed chunks but requires stable chunk boundaries across runs, which any chunking change would break. Doc-level is robust to chunker updates. If a future doc class is very large with localized edits (unlikely for solution docs at 100-500 lines), chunk-level invalidation is the optimization at that point.

### D5. Chunk → domain entity linkage is NOT #689's concern

Per #686 D9, `kg_chunks` has no `entity_id` column. #689 writes chunks with no knowledge of domain entities. Chunk → domain entity linkage goes through:

```
kg_chunks → kg_subject_entities (extracted by #690) → kg_subject_resolutions (linked by #691) → kg_entities
```

This is the canonical query path for "chunks about skill X." Any shortcut at ingestion time (frontmatter hints, path heuristics) is rejected: #689 does lexical ingestion, not domain inference. That's an architectural invariant, not a YAGNI punt.

**Guidance to #688 (query tool):** queries like "show me chunks about skill:self-dev" use the multi-hop JOIN described above. All joins are on indexed columns; at agent-scoped cardinality, SQLite handles this efficiently.

### D6. Deletion handling: prune chunks for docs removed from disk

After the per-agent scan-and-ingest pass, #689 prunes chunks whose `source_doc_path` no longer exists on disk:

```sql
-- pseudocode
on_disk = {path for path in glob("docs/solutions/**/*.md")}
in_db = SELECT DISTINCT source_doc_path FROM kg_chunks WHERE agent_id = ?
to_delete = in_db - on_disk
for path in to_delete:
    DELETE FROM kg_chunks WHERE agent_id = ? AND source_doc_path = ?
    (+ unindex_content calls — see D7)
```

Runs in the same transaction as the ingestion writes. Mid-run failure leaves a consistent pre-run state.

Matches #687's prune semantics (entity pruning by source presence). The ingestor owns its namespace (`kg_chunks` rows sourced from the solution-docs tree for its agent), and is responsible for cleaning up within that namespace.

### D7. Delete-composition contract: symmetric to C1.1

C1.1 defines the write contract: `INSERT kg_chunks → index_content()`. The delete contract is symmetric:

```
DELETE kg_chunks → unindex_content(source_type="kg_chunk", source_id=<kg_chunks.id>)
```

`unindex_content()` is a new helper (mirror of `index_content()`) that #689 introduces:

```rust
pub fn unindex_content(&self, source_type: &str, source_id: i64) -> Result<()>;
```

It deletes the `search_content` row for the given `(source_type, source_id)` and removes the corresponding FTS5 entry. The `vec_search` row is keyed by `search_content.id` (rowid), so it's pruned by the FK-less design of FTS5-external-content: the vec_search row remains as an orphan keyed on a now-deleted rowid, but vec_search doesn't reference back so this is benign (the vec row is unreferenced and can be pruned by a periodic GC or left alone — embeddings without a search_content row are never returned by `hybrid_search` because the join filters them).

Both `index_content()` and `unindex_content()` must run within the same transaction as the `kg_chunks` INSERT/DELETE for atomicity.

### D8. Order in startup sequence: after #687 domain rebuild

Startup call order:

```
SkillRegistry::from_dir() → apply_overrides() → validate_loaded() → log_summary()
    → DomainGraphBuilder.rebuild() [from #687]
    → for each agent in agent_configs: LexicalIngestor.ingest_all(agent_id) [this ticket]
    → [future] SubjectExtractor.run(agent_id) [from #690]
```

Hard ordering between #687 and #689 isn't required by FK (per D5 there's no direct FK between chunks and entities), but keeping the order preserves the "each KG layer's startup work completes before the next starts" invariant that downstream tickets assume. Document this as a startup sequence constraint in `server/mod.rs`.

## Open Questions

### Resolved During Planning

- Trigger model (startup scan + compound hook for authoring agent; see D1).
- Chunking strategy (2000/200 overlap with markdown boundaries; see D2).
- Per-agent duplication of shared docs (accept, instrument, defer optimization; see D3).
- Content-change detection (doc-level SHA-256 with normalization; see D4).
- Chunk → domain linkage (deferred to #690/#691 pipeline; see D5).
- Deletion handling (prune chunks for removed docs; see D6).
- Delete-composition contract (`unindex_content()` helper, same transaction; see D7).
- Startup ordering (after #687 domain rebuild; see D8).
- Compound hook failure semantics (fail-silently with warn log; see D1).

### Deferred to Implementation

- Exact scan-path list (today: `docs/solutions/**/*.md`; may extend to `docs/adr/` / `docs/architecture/` in a later ticket if usage shows value).
- Whether `search_content.source_id` can hold the kg_chunks INTEGER id directly without any adapter (should — both are INTEGER). Verify in Unit 2.
- Compound doc detection: does `/ce:compound` always land files in `docs/solutions/`, or are there subdirectory conventions we should trigger on? Probably same tree; confirm during implementation.

## Output Structure

```
crates/mika-agent/src/
├── db.rs                            # ADD: unindex_content() helper (mirror of index_content)
├── db/kg_schema.rs                  # REFERENCE: KG_CHUNK_COLUMNS, KG_CHUNK_SOURCE_TYPE from #686
└── kg/
    ├── mod.rs                       # MODIFY: add `pub mod chunker; pub mod lexical_ingestor;`
    ├── domain_builder.rs            # (from #687 — unchanged)
    ├── chunker.rs                   # NEW: deterministic markdown-aware chunker
    └── lexical_ingestor.rs          # NEW: scan, hash, chunk, write

crates/mika-agent/src/server/
└── mod.rs                           # MODIFY: call LexicalIngestor after DomainGraphBuilder

crates/mika-agent/tests/
└── kg/
    ├── chunker.rs                   # NEW: chunker determinism + boundary tests
    └── lexical_ingestor.rs          # NEW: ingestion integration tests

docs/plans/
└── 2026-04-21-005-feat-lexical-ingestion-plan.md   # this file
```

## Implementation Units

- [ ] **Unit 1: Deterministic markdown-aware chunker**

**Goal:** Produce `fn chunk_doc(text: &str) -> Vec<Chunk>` that splits on `---` and `##` with 2000/200-char window fallback. Deterministic — same input always produces the same output.

**Requirements:** R3.

**Dependencies:** None (pure library code).

**Files:**
- Create: `crates/mika-agent/src/kg/chunker.rs`
- Test: inline `#[cfg(test)] mod tests`

**Approach:**
- `pub struct Chunk { pub seq_id: u32, pub text: String }`.
- Algorithm: split on frontmatter fence (`^---` lines separating YAML from body), then on `## ` section headers within the body. For sections exceeding 2000 chars, slide a 2000-char window with 200-char overlap.
- Pure function: no I/O, no DB, no side effects. Takes `&str`, returns `Vec<Chunk>`.
- Unicode-safe: count chars, not bytes. Overlap boundary respects UTF-8 char boundaries (split at the nearest prior char boundary if byte count exceeds 2000).

**Test scenarios:**
- Happy path: small doc (<2000 chars) with no `##` → one chunk containing the whole body.
- Section split: doc with 3 `## ` sections, each under 2000 chars → 3 chunks.
- Window split: one section exceeds 2000 chars → multiple chunks with 200-char overlap.
- Frontmatter handling: doc with YAML frontmatter → frontmatter is chunk 0, sections start at seq_id 1 (or whatever the spec lands on — document the choice).
- Determinism: same input yields same Vec<Chunk> (important for D4 content-hash idempotency).
- UTF-8 edge: chunk boundary falls mid-multibyte-char → chunker backs off to previous char boundary, no panic.
- Empty doc: returns empty Vec<Chunk>, no error.

**Verification:** `cargo test -p mika-agent kg::chunker` green.

---

- [ ] **Unit 2: `unindex_content` helper**

**Goal:** Add the symmetric delete helper to `index_content()`. `#689`'s delete path and any future ticket that wants to remove indexed content uses it.

**Requirements:** R5, R7 (delete-composition contract).

**Dependencies:** None (uses existing db.rs infrastructure).

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs` (wrap in AsyncDatabase closure, same pattern as `index_content`)

**Approach:**
- `pub fn unindex_content(&self, source_type: &str, source_id: i64) -> Result<()>` in `db.rs`:
  - `DELETE FROM fts_search WHERE rowid = (SELECT id FROM search_content WHERE source_type = ? AND source_id = ?)` — FTS5 external-content model requires explicit delete.
  - `DELETE FROM search_content WHERE source_type = ? AND source_id = ?`.
  - `vec_search` rows are orphaned by design (no FK to delete against) — leave them; `hybrid_search` joins through `search_content` so orphan vec rows never surface.
- All statements in one transaction (caller's responsibility if they wrap multiple unindex calls; single-call use is atomic on its own).
- `AsyncDatabase::unindex_content()` thin wrapper for the async callers.

**Test scenarios:**
- Happy path: index content, then unindex → subsequent `fts_search` MATCH returns zero rows; `search_content` row is gone.
- Idempotency: unindex a non-existent (source_type, source_id) → no error, no-op.
- Isolation: unindex source_type="kg_chunk", source_id=X → person/commitment/preference/event rows with the same numeric source_id are untouched.

**Verification:** `cargo test -p mika-agent db::unindex_content` green.

---

- [ ] **Unit 3: LexicalIngestor core — scan, hash, chunk, write**

**Goal:** Implement `LexicalIngestor::ingest_all(agent_id) -> Result<IngestStats>`. Owns the per-agent ingestion run for the full `docs/solutions/**/*.md` tree.

**Requirements:** R1, R3, R4, R6, R8.

**Dependencies:** Units 1 and 2, #686 schema landed, #687 domain rebuild runs before this.

**Files:**
- Create: `crates/mika-agent/src/kg/lexical_ingestor.rs`
- Modify: `crates/mika-agent/src/kg/mod.rs` (add `pub mod lexical_ingestor;`)

**Approach:**
- Struct `LexicalIngestor { db: &AsyncDatabase, docs_root: PathBuf, trace_id: String }`.
- `fn normalize_content(raw: &[u8]) -> String`: strips BOM, normalizes line endings to LF, strips per-line trailing whitespace, enforces single trailing newline. Return UTF-8 String.
- `fn compute_hash(normalized: &str) -> String`: hex-encoded SHA-256.
- `async fn ingest_all(&self, agent_id: &str) -> Result<IngestStats>`:
  1. Walk `docs_root` for `*.md` files.
  2. For each file: read, normalize, compute hash.
  3. Query DB: is there an existing `kg_chunks` row for `(agent_id, path)` with matching hash? If yes → skip. If different hash or no rows → `delete_doc_chunks` then `insert_doc_chunks`.
  4. After all files processed, `prune_removed_docs(agent_id, on_disk_paths)`.
  5. Return `IngestStats { docs_scanned, docs_skipped_unchanged, docs_reingested, docs_pruned, chunks_added, chunks_deleted, duration_ms }`.
- `async fn insert_doc_chunks(&self, agent_id, path, hash, chunks)`: one transaction — for each chunk, INSERT `kg_chunks` row, then `index_content(source_type="kg_chunk", source_id=<inserted rowid>, text=chunk.text)`. Per C1.1, embeddings are NOT awaited here.
- `async fn delete_doc_chunks(&self, agent_id, path)`: one transaction — SELECT ids, loop calling `unindex_content` for each, then DELETE from kg_chunks.
- `async fn prune_removed_docs(&self, agent_id, on_disk_paths)`: one transaction — find DB-tracked paths not in `on_disk_paths`, delete those docs' chunks via `delete_doc_chunks`.
- `async fn ingest_single_doc(&self, agent_id, path) -> Result<DocStats>`: called by the compound hook — hashes, compares, ingests if changed. Shares the hash-and-write code with `ingest_all`.

**Patterns to follow:**
- AsyncDatabase closure pattern for transactional writes.
- `index_content` call shape from `db.rs:6107`.

**Test scenarios:**
- Happy path: empty DB, 3 docs on disk → all 3 ingested, returned stats match.
- Idempotency: run twice with identical tree → second run scans 3, skips 3, chunks_added=0.
- Change detection: edit one doc, re-run → 1 reingested (delete+reinsert chunks), 2 skipped.
- Normalization: doc with CRLF line endings → hash matches LF-normalized version on subsequent LF save; no false-positive re-ingest.
- Deletion: run ingest, delete a doc file, re-run → doc's chunks pruned, stats report docs_pruned=1.
- Transactional failure: inject DB error mid-ingestion → no partial state; all rollback.
- Cross-agent isolation: ingest for agent A, then agent B, same docs → each agent has their own chunk rows, no cross-contamination.

**Verification:** `cargo test -p mika-agent --test kg` suite green.

---

- [ ] **Unit 4: Startup integration**

**Goal:** Hook `LexicalIngestor::ingest_all` into the server startup sequence, per-agent, after #687's domain rebuild.

**Requirements:** R1, R7, R8.

**Dependencies:** Units 1–3; #687 integration landed.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs`

**Approach:**
- After the existing `DomainGraphBuilder.rebuild()` call (from #687), iterate `agent_configs`. For each agent: construct a `LexicalIngestor` with a fresh trace_id, call `ingest_all(agent_id).await`. On `Ok(stats)`, log `info!(event="lexical_ingest_complete", agent_id=%agent_id, ?stats)`. On `Err(e)`, log `warn!(event="lexical_ingest_failed", agent_id=%agent_id, error=%e)` and continue to the next agent.
- Order is strictly after `DomainGraphBuilder.rebuild()`; document with a code comment.
- Failure policy: per-agent failure does not abort startup; other agents still ingest.

**Test scenarios:**
- Integration: start server in test mode with 2 mock agents and 3 solution docs → post-startup, both agents have 3 docs' worth of chunks.
- Failure isolation: inject DB error for agent A only → agent B still ingests successfully; server startup completes.

**Verification:** `cargo test -p mika-agent --test startup_kg_integration` green.

---

- [ ] **Unit 5: Compound hook**

**Goal:** Add a public entry point the `/ce:compound` skill handler can call to trigger immediate single-doc ingestion for the authoring agent.

**Requirements:** R2.

**Dependencies:** Units 1–3.

**Files:**
- Modify: `crates/mika-agent/src/kg/lexical_ingestor.rs` — expose `ingest_single_doc(agent_id, path)` (from Unit 3) as the entry point.
- Identify the call site in the `/ce:compound` skill handler or the post-write hook and wire up. (Location TBD during implementation — depends on where compound docs land and what mechanism exists for post-write hooks. If no hook mechanism exists, the compound handler explicitly calls into the ingestor after writing the file.)

**Approach:**
- Caller passes `(agent_id, absolute_path_of_compound_doc)`.
- Ingestor does the same hash-check + ingest-if-changed logic as the bulk path.
- Returns `Result<()>`. Caller logs on error but does not fail the compound operation itself.
- Per D1, failure is fail-silently-with-warn-log — the authoring agent gets "your doc is saved; search will pick it up after next restart at latest."

**Test scenarios:**
- Happy path: compound writes a new doc → hook ingests it → doc is queryable via FTS5 immediately.
- Pre-existing doc (content unchanged via this run) → no-op, fast.
- Compound content changed (e.g., retroactive edit) → delete+reinsert chunks.
- Failure path: inject DB error during compound ingestion → hook returns Err, compound skill logs warn, doc remains on disk and will ingest at next startup.

**Verification:** Unit test + manual check via `/ce:compound` smoke test after wiring.

---

- [ ] **Unit 6: Audit event emission (per C3.2)**

**Goal:** Per C3.2 in the conventions doc, emit `tool_name=ingest_document` audit_events at per-document granularity.

**Requirements:** R8.

**Dependencies:** Unit 3.

**Files:**
- Modify: `crates/mika-agent/src/kg/lexical_ingestor.rs`

**Approach:**
- After successfully ingesting (or skipping, or reingesting) each doc, call the audit_events write path with:
  - `tool_name="ingest_document"`
  - `target_key="kg_chunk:<source_doc_path>"`
  - `before_value=json!({"chunks_existed": <pre-count>, "prior_hash": <prior or null>})`
  - `after_value=json!({"chunks_now": <post-count>, "new_hash": <new>})`
  - `reasoning=json!({"source": "startup_scan" | "compound_hook", "outcome": "inserted" | "skipped" | "reingested"})`
  - `trace_id` from the ingestor invocation.
- **No audit_events for prune operations on their own** — deletions happen as part of a reingestion (included in the reingest's audit event) or as part of the end-of-run prune (which emits one audit event per pruned doc with outcome="pruned").

**Test scenarios:**
- New doc ingested → one audit_events row with outcome="inserted", chunks_now > 0.
- Unchanged doc → one row with outcome="skipped", chunks_now == chunks_existed, new_hash == prior_hash.
- Changed doc → one row with outcome="reingested", prior_hash != new_hash.
- Removed doc (pruned) → one row with outcome="pruned", chunks_now=0.
- Row count invariant: ingestion run over N docs produces exactly N audit_events rows.

**Verification:** Unit test inspects audit_events after various ingestion scenarios.

## System-Wide Impact

- **Interaction graph:** New startup hook after #687; new compound-handler hook. No changes to agent loop, tool handlers, or webhook handlers. `search_memory` tool already works with the shared `search_content`/`fts_search`/`vec_search` infrastructure — ingesting `source_type="kg_chunk"` rows makes them searchable automatically.
- **Error propagation:** Ingestion failures are `warn!` per-agent, not fatal. Server startup completes even if some or all agents' ingestion fails.
- **State lifecycle risks:**
  - **Delete composition without unindex_content** (if Unit 2 is skipped or broken): deleting `kg_chunks` rows leaves `search_content` + FTS5 + `vec_search` entries pointing at rowids that no longer exist → `hybrid_search` returns stale snippets with broken backlinks. Unit 2 is load-bearing.
  - **Transactional atomicity** across `kg_chunks` INSERT + `index_content` call: per C1.1, both in one transaction. Partial writes never exposed to queries.
  - **Compound-hook ordering**: the hook must fire AFTER the doc is written to disk, not before or during. Compound handler controls ordering; document the contract.
- **API surface parity:** None changed. No new tools, no endpoints, no CLI subcommands.
- **Integration coverage:** Unit 3's integration suite and Unit 4's startup integration test cover the pipeline end-to-end.
- **Unchanged invariants:** `index_content()`, `hybrid_search()`, `search_memory` tool, `search_content` table shape, embedding backfill — all unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unit 2's `unindex_content` is skipped or broken → orphan FTS/search_content rows | Unit 3's tests verify post-delete queries return zero rows for deleted doc paths. Unit 2 has its own isolation test. |
| Chunker non-determinism breaks content-hash idempotency | Unit 1 has an explicit determinism test (same input → same Vec<Chunk>). Chunker is pure function, no I/O. |
| Hash normalization misses an edge case (e.g., Unicode normalization forms NFC vs NFD) | Start with the documented 4-step normalization (BOM / line endings / trailing ws / trailing newline). If false-positive re-ingestions appear in logs (D3 observability), extend normalization as needed. NFC is a plausible later addition. |
| Per-agent duplication at scale (100+ agents) becomes a real storage problem | Accept per D3, instrument per D3 (duration + chunk count logging), optimize later with real data. Not a #689 concern unless it's observed at implementation time. |
| Compound hook races with other writers (two compounds in quick succession) | `kg_chunks` UNIQUE(agent_id, source_doc_path, seq_id) prevents dup-inserts. Two hooks for the same doc ingest serially via the AsyncDatabase write thread. |
| Very large docs exceed single-transaction limits | SQLite supports multi-MB transactions trivially. A 10MB doc → ~5000 chunks × ~2000 bytes ≈ 10MB write. Well within limits. If this ever becomes an issue, split ingestion into per-doc transactions. |
| Docs tree has non-markdown files that happen to have `.md` extension but aren't conventional | Chunker handles any UTF-8 text; invalid UTF-8 files would fail the normalize step and log a warn. Non-fatal. |

## Documentation / Operational Notes

- Compound skill documentation may need a line about "written docs become searchable via KG after the next handler invocation, or immediately if the compound hook is wired." Defer until #692's doc pass.
- Ingestion duration logs (from Unit 4) are the operational signal for "is KG healthy." If durations exceed 10s per agent on startup, investigate.
- No migration in #689 itself — all schema comes from #686. This ticket is code-only plus docs.

## Sources & References

- **Origin ticket:** [mika#689](https://github.com/senara-solutions/mika/issues/689)
- **Milestone:** [mika milestone#14 "Knowledge Graph"](https://github.com/senara-solutions/mika/milestone/14)
- **Depends on:** [mika#686 (schema, D9/D10 amendments)](https://github.com/senara-solutions/mika/issues/686), [mika#687 (domain rebuild)](https://github.com/senara-solutions/mika/issues/687)
- **Cross-cutting conventions:** [`docs/architecture/kg-implementation-conventions.md`](../architecture/kg-implementation-conventions.md) (C1.1, C3.2 apply)
- **ID convention:** [`docs/architecture/kg-id-convention.md`](../architecture/kg-id-convention.md)
- **Related code:**
  - `crates/mika-agent/src/db.rs:6107` — `index_content()`
  - `crates/mika-agent/src/db.rs:1049-1059` — `search_content` schema
  - `crates/mika-agent/src/kg/domain_builder.rs` — #687, runs before this
  - `crates/mika-common/src/embedding.rs` — embedding pipeline (not called directly)
- **Institutional learnings:**
  - `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`
  - `docs/solutions/database-issues/iso8601-timestamp-migration.md`
  - `docs/solutions/database-issues/trace-id-as-observability-join-key.md`
  - `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`
  - `docs/solutions/logic-errors/startup-backfill-skips-embedding-generation.md`
