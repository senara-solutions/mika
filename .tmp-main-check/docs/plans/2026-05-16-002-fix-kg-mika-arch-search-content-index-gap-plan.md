---
issue: 1155
type: fix
title: KG lexical-ingest skip path bypasses per-agent search_content writes
status: GROOMED
---

# Plan: fix per-agent `search_content` parity on shared-corpus lexical ingest

## Phase 0 — Pin

Base commit: `31c1b0a5c12db57e6898c341fdeb3d7adad2c4d9` (main @
`docs(solutions/kg-investigations): resolver sonnet-baseline experiment
ratifies Decision A`).

### Pinned sites (verbatim slices)

**Site A — bug site:**
`crates/mika-agent/src/kg/lexical_ingestor.rs:269-286`:

```rust
// 1. Hash check: is the doc unchanged?
let existing_hashes: Vec<String> = {
    let mut stmt = db.conn.prepare(
        "SELECT DISTINCT source_doc_hash FROM kg_chunks
         WHERE docs_root_hash = ?1 AND source_doc_path = ?2",
    )?;
    stmt.query_map(
        rusqlite::params![docs_root_hash, rel_path_owned],
        |row| row.get(0),
    )?
    .filter_map(|r| r.ok())
    .collect()
};

// Single matching hash → unchanged, skip.
if existing_hashes.len() == 1 && existing_hashes[0] == new_hash_owned {
    return Ok((0usize, 0usize, true));
}
```

**Site B — per-agent search index API + schema:**
`crates/mika-agent/src/db.rs:1411-1421`:

```rust
CREATE TABLE search_content (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER,
    content TEXT NOT NULL,
    embedding_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);
```

`crates/mika-agent/src/db.rs:8132-8175` (`pub fn index_content`):
UPSERT keyed on `(agent_id, source_type, source_id)`. Match path UPDATEs
content + bumps FTS row; no-match path INSERTs new row and indexes FTS.
The function is idempotent at the row level.

**Site C — resolver join (downstream symptom):**
`crates/mika-agent/src/kg/entity_resolver.rs:1196-1197`:

```rust
"SELECT sc.content
 FROM kg_chunk_subjects cs
 JOIN search_content sc ON sc.source_type = 'kg_chunk'
     AND sc.source_id = cs.chunk_id AND sc.agent_id = ?
 WHERE cs.docs_root_hash IN ({}) AND cs.subject_entity_id = ?
 LIMIT 3"
```

The `sc.agent_id = ?` predicate is the failure point — empty result when
agent's `search_content` row is missing for the chunk.

**Site D — chunker contract:**
`crates/mika-agent/src/kg/chunker.rs:1-16, 38`:

```rust
//! # Markdown-Aware Document Chunker
//!
//! Deterministic, pure-function chunker for the lexical graph layer (#689).
//! Splits markdown documents into [`Chunk`]s suitable for entity extraction
//! and relationship linking.
//!
//! ## Algorithm
//!
//! 1. Strip YAML frontmatter (delimited by `---`) into its own chunk.
//! 2. Split the body on `## ` section headers.
//! 3. Window-split any section exceeding [`MAX_CHUNK_CHARS`] into overlapping
//!    windows of [`MAX_CHUNK_CHARS`] with [`OVERLAP_CHARS`] overlap.
//! 4. Assign monotonic [`Chunk::seq_id`] starting at 0.
//!
//! All size arithmetic uses **char counts**, not byte counts, so multibyte
//! UTF-8 sequences are handled correctly.

pub fn chunk_doc(text: &str) -> Vec<Chunk>
```

Documented as pure-and-deterministic. **But** `MAX_CHUNK_CHARS = 2000` and
`OVERLAP_CHARS = 200` are tunable constants — a future tuning change would
silently desync our backfill output from already-stored DB text (see F2
resolution under §Solution).

**Site E — normalization contract:**
`crates/mika-agent/src/kg/lexical_ingestor.rs:558-581` (`pub fn normalize_content`):

```rust
pub fn normalize_content(raw: &[u8]) -> String {
    // 1. Strip UTF-8 BOM if present.
    let bytes = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &raw[3..]
    } else {
        raw
    };
    // 2. Decode as UTF-8 (lossy — invalid sequences become U+FFFD).
    let text = String::from_utf8_lossy(bytes);
    // 3. Normalize line endings: \r\n -> \n, then lone \r -> \n.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    // 4. Strip trailing whitespace from each line.
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    let mut result = lines.join("\n");
    // 5. Enforce single trailing newline.
    let trimmed = result.trim_end_matches('\n');
    result = format!("{trimmed}\n");
    result
}
```

Pure, deterministic on byte input. SHA-256 of this output is the
`source_doc_hash` stored in `kg_chunks`.

**Site F — embedding backfill site (transitive cascade — see §Transitive
effects):**
`crates/mika-agent/src/db.rs:8223-8231`:

```rust
/// Returns search content rows that have no embedding yet (for backfill).
pub fn get_unembedded_content(&self, agent_id: &str) -> Result<Vec<(i64, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, content FROM search_content
         WHERE agent_id = ?1 AND embedding_json IS NULL",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
```

Per-agent query. New `search_content` rows from backfill arrive with
`embedding_json = NULL` (no embedding column written by `index_content`).
They are automatically picked up by this query — no separate trigger
needed.

## Problem (one paragraph)

After the v27 shared-corpus migration (#786/#787), `kg_chunks` became keyed by
`docs_root_hash` and the lexical ingestor uses a content-hash idempotency check
to skip re-chunking. The hash-check skip path in
`LexicalIngestor::ingest_single_doc_inner`
(`crates/mika-agent/src/kg/lexical_ingestor.rs:269-286`) returns
`(0, 0, was_skipped=true)` whenever another agent has already ingested a doc
with the matching content hash for the shared `docs_root_hash` — **without
performing the per-agent `db.index_content(agent_id, …)` writes for that
agent**. `search_content` is still a per-agent index (`agent_id` PK column,
`db.rs:1411`), so the agent that lost the race ends up with shared `kg_chunks`
rows but no corresponding per-agent `search_content` rows. The resolver's
`get_chunk_context()` join filters on `sc.agent_id = ?`
(`entity_resolver.rs:1196-1197`) and returns empty for ~76% of mika-arch's
subjects, producing name-only LLM disambiguation prompts and elevating the
miss rate.

This is a v27 migration-shape regression: the writer became corpus-scoped but
the per-agent index it feeds did not, and the skip path was not updated to
treat the two scopes orthogonally.

## Root cause (precise)

`ingest_single_doc_inner`:

```
SELECT DISTINCT source_doc_hash FROM kg_chunks
 WHERE docs_root_hash = ?1 AND source_doc_path = ?2

if existing_hashes.len() == 1 && existing_hashes[0] == new_hash {
    return Ok((0, 0, true));   // Skipped.
}
```

When `was_skipped == true`, the function returns before reaching the
transaction that calls `db.index_content(agent_id, KG_CHUNK_SOURCE_TYPE,
Some(chunk_rowid), …)`. The chunk rows live in the shared table and are
correctly idempotent across agents; the per-agent search index does not get
written for any agent that wasn't the first to ingest the doc.

mika-arch is hit hardest because it has six `docs_roots` and the shared
`mika/docs/solutions` corpus is also ingested by `mika`/`mika-relay`. Whichever
agent's startup task wins the race for a given doc writes its own
`search_content` rows; the others skip. mika-arch's exclusive corpora
(mika-platform, mika-cloud, mika-skills, claude-pilot-py, openclaw, lettabot)
are unaffected because no other agent indexes them — they account for
mika-arch's existing 1,722 rows.

## Solution (shape)

Split the idempotency check into two orthogonal axes:

| Axis | Scope | Source of truth |
|------|-------|------------------|
| Shared chunk content | `(docs_root_hash, source_doc_path)` | `kg_chunks` rows + `source_doc_hash` |
| Per-agent search index | `(agent_id, source_type='kg_chunk', source_id)` | `search_content` rows |

The hash-check optimization is correct for the first axis. The skip path must
not collapse the second axis into it.

Concrete change: when `existing_hashes.len() == 1 && existing_hashes[0] ==
new_hash`, do NOT early-return. Instead, fall into a backfill branch that:

1. Reads the existing `kg_chunks.id` values for `(docs_root_hash,
   source_doc_path)`.
2. Reads the agent's existing `search_content` `source_id` values for the
   same chunks under `(agent_id, source_type='kg_chunk')` to compute the
   missing set.
3. For each missing chunk, fetches `kg_chunks` text (re-derive from the
   normalized doc + chunker — already in memory) and calls
   `db.index_content(agent_id, KG_CHUNK_SOURCE_TYPE, Some(chunk_id), text)`.
4. Returns a new `DocOutcome::IndexBackfilled { chunks_indexed: N }` (or
   reuse `Ingested` with a flag — see §Decision points) so stats and audit
   events reflect the work done.

`index_content` is already an UPSERT keyed on `(agent_id, source_type,
source_id)` (`db.rs:8139-8175`), so calling it for chunks the agent already
has is a no-op UPDATE — safe but wasteful, hence the membership check before
the writes.

Self-healing: each startup `ingest_all` for a previously-affected agent will
discover the missing rows on the skip path and backfill them. No explicit
one-shot migration needed; no schema change needed.

## Decision points (settle these in pass 1 if architect requests)

### D1 — outcome variant

**Option A (recommended): introduce `DocOutcome::IndexBackfilled { chunks: N }`**
and surface it in `IngestStats` as a new `chunks_indexed_backfill` counter.
Audit event payload uses `outcome="index_backfilled"`. Clean separation from
`Ingested` (which means chunk rows changed) and `Skipped` (which means
nothing happened).

**Option B: reuse `Ingested` with `chunks_added=0`.** Cheaper change, but
loses the signal that the chunks themselves were unchanged — operators
looking at the existing `lexical_ingest_complete` log line wouldn't be able
to distinguish "fresh chunk write" from "agent index sync."

Pick A — observability matters during the rollout. The Lexical-Ingest signal
list in the root `CLAUDE.md` is the natural home; we'll add a sentence under
the existing lexical signals.

### D2 — placement of the backfill query

The membership-check + backfill loop runs inside the same `with_db` closure
as the existing hash check, so it shares the transaction boundary with the
read. Two SQL queries instead of one (the hash check + a `WHERE NOT EXISTS`
or `EXCEPT` on `search_content`). Both are indexed (`kg_chunks` has
`UNIQUE(docs_root_hash, source_doc_path, seq_id)`; `search_content` has
`idx_search_agent` on `(agent_id, source_type)`). At ~3,000 chunks per
docs_root, the membership check is a single sub-millisecond scan per doc.

### D3 — chunk text source (decided)

**Decision: `SELECT text FROM kg_chunks` is the authoritative source for
the backfill path.** Architect F2 (pass 1) ratified — committing to (b)
over (a).

Reasoning: the chunker is documented as pure and deterministic
(`chunker.rs:33-37`), BUT `MAX_CHUNK_CHARS` and `OVERLAP_CHARS` are tunable
module constants (`chunker.rs:19, 22`). A future tuning change between the
deploy that wrote chunks-version-N to the DB and the deploy running the
backfill would silently produce chunks-version-N+1 in memory while reusing
the version-N `chunk_id` values from the DB — yielding a per-chunk text
disagreement under a matching `source_doc_hash`. The on-disk doc would be
unchanged; only the chunker's slicing rule would have moved.

Cost: one extra indexed read per chunk per missing-row on the first
post-deploy restart per affected agent. The query is
`SELECT id, text FROM kg_chunks WHERE docs_root_hash = ? AND
source_doc_path = ? ORDER BY seq_id` — covered by the existing
`UNIQUE(docs_root_hash, source_doc_path, seq_id)` index. Negligible vs the
self-healing one-shot semantics.

The in-memory `chunks` vector remains in scope but is now only used by the
content-changed delete+insert path (which already trusts the chunker —
unchanged from current behavior, because that path *rewrites* the chunks).

### D3a — fail-loud on row-count mismatch

When the backfill path reads `kg_chunks` rows for `(docs_root_hash,
source_doc_path)` and the count differs from the local `chunks.len()`,
log a `warn!` with `event = "lexical_backfill_chunk_count_mismatch"` and
fall through to the delete+insert path (treat as content-changed). This
defends against the chunker-version-drift case Phase 0 Site D flags: if
in-memory and DB disagree on chunk count for the same `source_doc_hash`,
something is wrong and re-chunking is the safe move.

### D4 — observability

Add two log fields to `lexical_ingest_complete`:
`docs_index_backfilled` (count of docs where backfill ran) and
`chunks_indexed_backfill` (total chunks written via backfill path).
Add a per-doc audit event with `outcome="index_backfilled"`. This makes the
rollout visible in `server.log` without DB inspection.

### D5 — order in `with_db` closure

The existing code has a tx that handles delete+insert. The skip path doesn't
need a tx (UPSERT semantics in `index_content` are per-row atomic — failure
of one row doesn't corrupt others; partial backfill on the next startup will
finish what crashed). Keep the backfill outside the `unchecked_transaction`
to avoid widening the lock scope.

Actually — `index_content` writes to `search_content` AND `fts_search` as
separate statements. For atomicity per chunk we should wrap the backfill in
a small tx so a crash mid-doc leaves either-both-or-neither. Use
`unchecked_transaction` matching the existing pattern at lines 289-352.

## Affected files (concrete)

1. `crates/mika-agent/src/kg/lexical_ingestor.rs`
   - Add `DocOutcome::IndexBackfilled { chunks_indexed: usize }` variant.
   - Extend `DocStats` with `chunks_indexed_backfill: usize` field.
   - Extend `IngestStats` with `docs_index_backfilled: usize` and
     `chunks_indexed_backfill: usize` fields.
   - Rewrite the `was_skipped` early-return path
     (lines 269-286, 357-363) to drop into the backfill branch when the
     hash matches.
   - Update `ingest_all`'s match arms (lines 155-167) to account for the
     new outcome.
   - Update `emit_audit_event` (lines 473-507) to map `IndexBackfilled` to
     `outcome="index_backfilled"` and include `chunks_indexed_backfill`
     in the `after_value` JSON.
   - Add two new fields to the `lexical_ingest_complete` `info!` call.
   - Add the seven new unit tests (see §Test plan).

2. `crates/mika-agent/CLAUDE.md`
   - Update the Lexical Ingestor section (line locations TBD — search for
     `Knowledge Graph — Lexical` or similar) to document the two-axis
     idempotency model and the new outcome variant.

3. `mika/CLAUDE.md` (root)
   - Add a brief signal entry to the "Post-restart safety check" section
     describing the new `docs_index_backfilled` log field, alongside
     Signal G (extraction fairness) and Signal H (extraction tick drain).

## Test plan

### Unit tests (`lexical_ingestor.rs#tests`)

Use an in-memory `AsyncDatabase` with `Database::open_in_memory_full` (or
the test-utils helper if available) and two distinct `agent_id` values
sharing the same `docs_root_hash`.

1. **`backfill_when_second_agent_finds_chunks_already_indexed`** — agent A
   ingests a 3-chunk doc; agent B runs `ingest_all` on the same docs_root.
   Assert: agent B has 3 `search_content` rows after; `IngestStats` shows
   `docs_index_backfilled=1`, `chunks_indexed_backfill=3`,
   `docs_ingested=0`, `docs_skipped_unchanged=0`.

2. **`no_backfill_when_agent_already_has_search_rows`** — agent A ingests;
   agent A runs `ingest_all` again. Assert: `docs_skipped_unchanged=1`,
   `chunks_indexed_backfill=0` (true no-op, the original optimization
   still fires).

3. **`partial_backfill_when_agent_has_subset_of_chunks`** — seed agent B
   with 1 of 3 chunks manually (`db.index_content` for one chunk_id), then
   run ingest. Assert: `chunks_indexed_backfill=2`, agent B's
   `count_search_content` returns 3 after.

4. **`backfill_preserves_idempotency_on_repeat`** — run ingest twice for
   agent B against the same shared corpus. Assert second run reports
   `docs_skipped_unchanged=1`, `chunks_indexed_backfill=0`.

5. **`fts_search_rows_present_after_backfill`** — after backfill, call
   `db.fts_search(agent_b, "needle", 10, Some(KG_CHUNK_SOURCE_TYPE))` for
   a term in the doc; assert non-empty result. (Guards against an FTS-only
   regression — the bug surfaces in `get_chunk_context` precisely because
   FTS row absence joins to nothing.)

6. **`get_chunk_context_join_succeeds_post_backfill`** — replicate the
   ticket reproduction shape: seed a `kg_chunk_subjects` row for
   `(docs_root_hash, chunk_id, subject_entity_id)`, run agent B's ingest,
   then query the `get_chunk_context`-equivalent join. Assert agent B's
   join returns the chunk content. (End-to-end shape test, not just unit.)

7. **`changed_content_takes_normal_ingest_path_not_backfill`** — agent A
   ingests; doc changes on disk; agent A re-ingests. Assert `docs_ingested=1`
   and `docs_index_backfilled=0` — the hash mismatch keeps the existing
   delete+insert path (which already does its own per-agent index_content
   writes).

### Integration test (regression guard — ticket AC requirement)

`crates/mika-agent/tests/eval/kg_fixtures/` already exists for KG schema
fixtures. Add `tests/eval/kg_multi_agent_corpus_parity.rs`:

- Spin two agents with `[kg].enabled = true` and overlapping `docs_roots`
  (one shared dir, one unique-per-agent dir).
- Run the server startup hook (`lexical_ingest_all_agents` or equivalent
  call from `server/mod.rs:806-907`) sequentially for both agents.
- Assert: `db.count_search_content(agent_a) ==
  db.count_search_content(agent_b)` for the shared `docs_root_hash`
  (computed via filter on `search_content.source_id IN (SELECT id FROM
  kg_chunks WHERE docs_root_hash = ?)`).
- Assert the random ordering case: shuffle agent invocation order across
  multiple test iterations; parity must hold regardless of order.

The query for the parity assertion is the same shape as the ticket's
reproduction step 3, which doubles as a manual smoke check.

### Manual smoke (post-deploy on the gentoo host)

```sql
-- Pre-deploy snapshot
SELECT agent_id, COUNT(*) FROM search_content
 WHERE source_type='kg_chunk' GROUP BY agent_id;

-- After restart, expect mika-arch to converge to peer parity for shared corpus
SELECT agent_id, COUNT(*) FROM search_content sc
 WHERE source_type='kg_chunk'
   AND sc.source_id IN (
     SELECT id FROM kg_chunks
      WHERE docs_root_hash = (SELECT docs_root_hash FROM agent_kg_corpora
                               WHERE agent_id='mika' LIMIT 1)
   )
 GROUP BY agent_id;
```

Expect: mika-arch row count for the shared `mika/docs/solutions`
`docs_root_hash` reaches parity with `mika`/`mika-relay` (2,700+).

## Acceptance criteria mapping (ticket §AC)

| Ticket AC | Plan element |
|-----------|--------------|
| mika-arch `search_content` parity for shared corpus | Unit test 1 + integration test + manual smoke |
| `get_chunk_context()` join-success rate >80% | Unit test 6 (join succeeds post-backfill) + the structural fix itself (the join can succeed iff `search_content` has the row) |
| Regression guard: parity integration test | `tests/eval/kg_multi_agent_corpus_parity.rs` |

## Rollout

1. Land the PR. CI runs the new tests.
2. Deploy to the dev host (gentoo). On startup, mika-arch's ingest_all
   sees ~3,000 docs in the shared corpus on the skip path → backfills
   per-doc. `lexical_ingest_complete` log line shows `docs_index_backfilled`
   ≈ shared-corpus doc count for that one boot.
3. Subsequent restarts show `docs_index_backfilled=0` for the same corpus
   — self-healing complete.
4. Re-run the resolver-baseline experiment (mika#1152 surface) to confirm
   mika-arch's miss rate drops independently of the fragmentation finding
   (the sister ticket).

## Transitive effects (no separate triggers needed)

Three downstream subsystems consume `search_content` rows. None require
explicit new triggers — the backfill is naturally picked up by their
existing per-agent queries. Documented here so future maintainers don't
add redundant invocations.

### Embeddings (Layer 3 vector index)

`get_unembedded_content(agent_id)` (Phase 0 Site F,
`crates/mika-agent/src/db.rs:8224-8231`) selects on
`agent_id = ?1 AND embedding_json IS NULL`. New rows written by
`index_content` during backfill have `embedding_json = NULL` by default
(the column is set later by `index_embedding`). They enter the embedding
backfill set on the next embedding cycle automatically. **No explicit
embedding-backfill trigger is needed in this PR.**

The embedding backfill is driven by `embedding_backfill_loop`
(server-level periodic) and by the initial `OptionalEmbedder` walk at
startup — both already iterate `get_unembedded_content` per agent.

### FTS5 (Layer 3 lexical index)

`fts_search` table is co-written by `index_content` in the same call that
inserts `search_content` (`db.rs:8170-8173`). The backfill path calls
`index_content`, which writes both rows. **No FTS-specific work needed.**
Unit test 5 (`fts_search_rows_present_after_backfill`) is the regression
guard.

### Resolver `get_chunk_context()` (the original user-visible symptom)

After backfill, the join at `entity_resolver.rs:1196-1197` finds rows for
the affected agent. **No resolver-side code change in this PR.**

**Important non-trigger:** prior `kg_resolutions_log.outcome='no_match'`
rows for the affected agent are NOT invalidated by this PR. The resolver
tick (#906) re-evaluates only entities without a log row; entities that
got `no_match` under a name-only prompt stay logged-as-`no_match` until a
separate invalidation. That invalidation is filed as a follow-up
candidate (§Open uncertainties #1) — out of scope here.

## Risk / non-goals

- **Sister ticket scope:** subject-extractor type-overreach (referenced in
  the ticket) is OUT of scope. The fix here addresses chunk-context
  availability, not fragmentation. The two are independent and were
  separated deliberately in #1152's investigation.
- **Schema:** no DDL; no migration; no `schema_meta` marker; no
  `agent_kg_corpora` change.
- **Performance:** the skip path now does one extra SQL query
  (`COUNT/EXCEPT` to find missing chunks) per doc per agent per startup.
  At 3,000 docs × N_agents, that's 9,000 ms of extra startup work for the
  worst-case full-cohort backfill, then 0 ms thereafter. Acceptable —
  startup is already O(seconds) for the lexical phase per agent.
- **Concurrency:** two agents ingesting the same shared corpus in parallel
  could race. The existing v27 ingest path already handles this for
  `kg_chunks` (idempotent insert). The added backfill path is read-then-
  write per-agent under `unchecked_transaction`; cross-agent there is no
  conflict because `search_content` rows differ by `agent_id`. No new
  locks needed.
- **Resolution invalidation:** this fix does NOT invalidate prior
  `kg_resolutions_log.outcome='no_match'` rows for mika-arch. Those rows
  were written when `get_chunk_context()` returned empty — name-only
  prompts. After backfill, the chunk context will be available, so future
  resolution attempts will see richer context. The domain-rebuild
  invalidation path (#960) covers entity-type expansion, not
  chunk-context expansion. **Follow-up consideration**: a one-shot
  invalidation of `no_match` rows for mika-arch (gated by a `schema_meta`
  marker) would let the resolver tick re-attempt those subjects with the
  newly-available chunk context. I'm flagging this as a follow-up issue
  candidate, not folding it in — it's a separable scope and the architect
  should weigh whether to bundle it or keep this PR tight.

## Open uncertainties (call out for architect)

1. **Should the resolution-invalidation follow-up be folded into this PR or
   filed separately?** My instinct is separate — this PR fixes the index;
   that PR re-uses the index. Independent commits, independent revert
   blast radius. But I want pass-1 to ratify.

2. **Should the backfill log a one-time INFO at first detection per agent
   instead of WARN?** Current plan: INFO inside the per-doc audit event,
   no top-level WARN. The first deploy will produce a single
   `lexical_ingest_complete` line per agent with non-zero
   `docs_index_backfilled`; subsequent restarts converge to zero. WARN
   would be noisy. Confirm.

3. **Sister-ticket interaction:** the fragmentation fix (subject-extractor
   type-overreach) will change which subjects exist. If it lands first,
   mika-arch's `kg_subject_entities` set shifts; our backfill is keyed on
   `kg_chunk_subjects` join → `chunk_id`, which is upstream of subject
   identity. No coupling. Order-of-land: independent.
