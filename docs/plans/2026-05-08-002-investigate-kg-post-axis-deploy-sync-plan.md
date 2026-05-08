---
ticket: mika#1027
type: investigate
module: mika-agent
tags: [kg, lexical-ingest, subject-extraction, resolution, post-deploy-verification]
parent: mika_priority_stack_2026-05 Tier 1 #2
---

# Plan: Post-Axis-Deploy KG Sync Verification (Tier 1 #2)

## Problem

Three compound docs landed in `mika/docs/solutions/best-practices/` over 2026-05-07 + 2026-05-08:

1. `verify-on-write-citation-discipline-2026-05-07.md` — landed via mp#92 (orchestrator-Claude, 2026-05-07 ~19:44Z)
2. `silent-mode-summary-budget-cap-2026-05-08.md` — landed via mika#1022 (autonomous loop, 2026-05-08 ~00:18Z)
3. `summarizer-factual-assertion-reform-2026-05-08.md` — landed via mika#1025 (autonomous loop, 2026-05-08 ~08:43Z)

The KG ingests `docs/solutions/**/*.md` per agent at startup and via the resolver tick (mika#906) every 30 min. **Question this investigation answers:** are these three docs (a) chunked, (b) extracted into subject entities + relationships, (c) resolved into the domain graph (or correctly classified `no_match` for novel subject-only concepts), and (d) reflected in resolver-tick health metrics without `aborted_budget=true`?

This is investigation-only. Output: a finding doc with verdict + outcome path declaration. **Outcome paths:** A (healthy, close ticket); B (specific bug, file follow-up fix ticket); C (signals unclear, escalate to operator).

## Reconnaissance already done (treat as starting context, verify in /ce:work)

**On-disk:** all three files present at expected paths (operator pre-fetched 2026-05-08T08:50Z).

**Chunked:**

| Doc | chunks | last_chunked |
|---|---|---|
| `silent-mode-summary-budget-cap-2026-05-08.md` | 8 | 2026-05-08T08:08:33Z |
| `summarizer-factual-assertion-reform-2026-05-08.md` | 8 | 2026-05-08T08:48:33Z |
| `verify-on-write-citation-discipline-2026-05-07.md` | 9 | 2026-05-07T19:44:01Z |

Layer 1 (chunking) is **PRESENT** for all three. Investigation focuses on layers 2 (extraction), 3 (resolution), and resolver-tick health.

**Schema note (per v27 migration, mika#786/#787):** `kg_extractions`, `kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships` are now keyed by `docs_root_hash` (16-hex SHA-256 prefix of canonical docs root path), NOT `agent_id`. `kg_subject_resolutions` and `kg_resolutions_log` remain per-agent. Per-agent corpora are linked via `agent_kg_corpora(agent_id, docs_root_hash)`. **Plan-time observation:** this changes the query shape vs the original ticket body — `kg_extractions.agent_id` does not exist post-v27. The investigation must use `docs_root_hash` joins.

## Design

### Investigation script approach

A single Bash script at `scripts/investigate-kg-1027.sh` that runs the canonical SQL + log queries against `~/.mika/data/mika.db` and `MIKA_SERVER_LOG_FILE`, prints structured output, and writes the finding doc inline. Reasoning: the queries are concrete and uncached — the investigation IS the queries. Embedding them in a script (rather than running ad-hoc) makes the work reproducible: the next post-deploy KG verification (which the runbook §3 anticipates) reuses the same script.

Alternatives rejected:
- **Single audit doc with copy-paste queries:** less reproducible; future runbook hooks would need to re-derive the queries.
- **Rust integration test:** overkill for a one-shot investigation; would also miss the log-grep portion.
- **Skip the script, write the finding doc directly:** loses the reusability win for the runbook.

### What the script checks (six layers — verbatim queries)

Per architect first-pass F1: the SQL/jq queries below are the implementation. Each is shown with its full table set, join path, output columns, expected shape, and PASS/FAIL criterion. Per architect NF3: L5/L6 use **jq selectors against structured log fields**, not text grep — the server emits JSON with `.event` field as canonical event name.

For each of the three target docs (`source_doc_path` LIKE %target-substring%):

#### Layer 1 — Chunking (`kg_chunks`)

```sql
-- L1: chunk presence per doc
SELECT
  source_doc_path,
  COUNT(*)              AS chunks,
  source_doc_hash,
  MAX(created_at)       AS last_chunked
FROM kg_chunks
WHERE source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR source_doc_path LIKE '%summarizer-factual-assertion%'
   OR source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY source_doc_path;
```

| Column | Meaning |
|---|---|
| `chunks` | count of `kg_chunks` rows per doc |
| `source_doc_hash` | content-hash for #757 idempotency |
| `last_chunked` | most recent ingest timestamp |

**PASS:** all 3 docs return ≥1 chunk row, non-NULL `source_doc_hash`.
**FAIL:** any doc missing or `chunks=0` → Path B (chunking layer drop).
**Reconnaissance result:** PASS — verified 2026-05-08T08:50Z (see § Reconnaissance above).

#### Layer 2 — Subject extraction (`kg_chunk_subjects` × `kg_subject_entities`)

```sql
-- L2: entity count per doc via chunk → subject_entity provenance
SELECT
  c.source_doc_path,
  COUNT(DISTINCT cs.subject_entity_id) AS entities,
  COUNT(*)                              AS provenance_rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.source_doc_hash  -- v27 shared-corpus join
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path;
```

| Column | Meaning |
|---|---|
| `entities` | distinct `subject_entity_id` count per doc |
| `provenance_rows` | row count in `kg_chunk_subjects` (>= entities; one entity may appear across multiple chunks) |

**PASS:** each doc has `entities > 0`. Typical: 4–15 entities per doc depending on length.
**FAIL:** any doc with `entities = 0` while L1 chunks present → Path B (subject extractor drop). Possible causes: `kg_extractions` row exists without write to provenance tables (idempotency-marker bug); LLM extraction returned empty result (parse-tolerance regression — see mika#876); `MIKA_KG_EXTRACTION_MODEL` unset.

#### Layer 3 — Subject relationships (`kg_chunk_subject_relationships`)

```sql
-- L3: fact-triple count per doc via chunk → subject_relationship provenance
SELECT
  c.source_doc_path,
  COUNT(DISTINCT csr.subject_relationship_id) AS triples,
  COUNT(*)                                     AS provenance_rows
FROM kg_chunks c
JOIN kg_chunk_subject_relationships csr
  ON csr.chunk_id = c.id
 AND csr.docs_root_hash = c.source_doc_hash  -- v27 shared-corpus join
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path;
```

**PASS:** each doc has `triples > 0`. Best-practices docs typically yield 3–12 triples (USES, SOLVED_BY, MENTIONS, etc.).
**FAIL:** any doc with `triples = 0` while `entities > 0` → Path B (relationship extractor drop while entity extractor succeeded — possible LLM partial-output bug or schema-validation reject).

#### Layer 4 — Resolution (`kg_resolutions_log`, agent-scoped)

```sql
-- L4: resolution outcomes per doc, scoped to mika-arch (sole KG consumer per mika#800)
SELECT
  c.source_doc_path,
  rl.outcome,
  COUNT(*) AS rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.source_doc_hash
JOIN kg_resolutions_log rl
  ON rl.subject_entity_id = cs.subject_entity_id
 AND rl.agent_id = 'mika-arch'  -- per-agent scoping (kg_resolutions_log is agent-keyed)
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path, rl.outcome;
```

| Outcome | Meaning | Verdict |
|---|---|---|
| `exact_match` | Subject ↔ domain entity, case-insensitive name match | PASS |
| `matched_llm` | LLM disambiguator picked a domain candidate | PASS |
| `matched_llm_db_fallback` | LLM match outside in-prompt window, accepted via DB fallback (mika#874) | PASS |
| `no_match` | No domain candidate (acceptable for novel subject-only concepts: "Axis 3", "load-omit sentinel", "Mode 5") | PASS (acceptable degraded) |
| `skipped_no_llm` | `MIKA_KG_RESOLUTION_MODEL` unset | FAIL with note (degraded mode; flag in finding) |
| (no row at all) | Subject entity has zero `kg_resolutions_log` rows | FAIL → Path B (resolution layer drop or pending-resolution stuck in queue) |

**PASS:** every entity from L2 has at least one row in L4 (count ≥ L2 entities) AND `no_match` rate is below 80% of resolved entities per doc. Outcome distribution is informational beyond those two thresholds.
**FAIL (resolution drop):** L2 entities > L4 row count → unresolved entities pending or dropped.
**FAIL (false-PASS guard, per architect NF5):** `no_match` rate > 80% of resolved entities for a given doc → either the domain graph is sparse (cross-check L6 entity count + per-type breakdown) or the resolver is misconfigured. Some `no_match` is expected for novel subject-only concepts; a majority-`no_match` rate is structural, not novelty. The 80% threshold is heuristic — operator can calibrate per future doc-class data, but having an explicit threshold is better than leaving the judgment to the implementer.

Cross-check query (count subject entities WITHOUT any resolution log row):

```sql
-- L4 cross-check: subject entities with zero resolution log rows (per agent)
SELECT
  c.source_doc_path,
  COUNT(DISTINCT cs.subject_entity_id) AS unresolved_pending
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.source_doc_hash
WHERE NOT EXISTS (
  SELECT 1 FROM kg_resolutions_log rl
  WHERE rl.subject_entity_id = cs.subject_entity_id
    AND rl.agent_id = 'mika-arch'
)
  AND (c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
    OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
    OR c.source_doc_path LIKE '%verify-on-write-citation%')
GROUP BY c.source_doc_path;
```

PASS: zero `unresolved_pending` per doc OR pending count is monotonically draining (compare against the resolver-tick `pending_after` from L5).

#### Layer 5 — Resolver tick health (jq on structured server log)

Per architect NF3: server logs are structured JSON; use jq selectors against `.event` field.

```bash
# L5: resolver tick events from 2026-05-07T16:00Z onward, mika-arch only
jq -c '
  select(.event == "kg_resolver_tick.complete" or .event == "kg_resolver_tick.error" or .event == "kg_resolver_tick.start")
  | select(.timestamp > "2026-05-07T16:00:00Z")
  | select(.agent_id == "mika-arch")
  | {ts: .timestamp, event, agent_id, pending_after, llm_calls, aborted_budget, per_corpus_attempted}
' "$MIKA_SERVER_LOG_FILE"
```

| Field | Source | PASS | FAIL |
|---|---|---|---|
| `aborted_budget` | tick.complete | `false` on every tick | `true` on any tick → resolver drowning (Signal D check; raise `MIKA_KG_BATCH_BUDGET` or investigate) |
| `pending_after` | tick.complete | trending to 0 across ticks (Signal E) | flat or growing across multiple ticks → resolver stalled |
| `per_corpus_attempted` JSON | tick.complete (mika#927) | non-zero attempts on every tick for every corpus that has pending entities | corpus with pending > 0 and zero attempts → fairness violation |
| `event == "kg_resolver_tick.error"` | error event | absent | present → log error message, classify as Path B |

#### Layer 6 — Domain graph health

Two probes — the rebuild log event and the entity-count snapshot:

```bash
# L6a: domain rebuild events post-deploy (2026-05-08T08:00Z onward)
jq -c '
  select(.event == "domain_rebuild_start" or .event == "domain_rebuild_complete" or .event == "domain_rebuild_entities" or .event == "domain_rebuild_edges")
  | select(.timestamp > "2026-05-08T08:00:00Z")
  | {ts: .timestamp, event, trace_id}
' "$MIKA_SERVER_LOG_FILE"
```

```sql
-- L6b: entity-count snapshot
SELECT COUNT(*) AS total_entities FROM kg_entities;
SELECT COUNT(*) AS total_relationships FROM kg_relationships;
SELECT entity_type, COUNT(*) AS n
FROM kg_entities
GROUP BY entity_type
ORDER BY n DESC;
```

| Probe | PASS | FAIL |
|---|---|---|
| L6a: `domain_rebuild_complete` log event present after each post-deploy server start | event present, paired with `domain_rebuild_start` (matching `trace_id`) | event missing → rebuild silently failed (the module's failure policy is log-and-continue per `domain_builder.rs`) |
| L6b: `total_entities > 0` | sane breakdown — `skill: ~30+`, `tool: ~10+`, `agent: ~5`, `problem_type: 5`, `concept: 20` | zero total entities → catastrophic rebuild failure (high-severity Path B) |

### Per-doc rollup

After running the six-layer queries, the script produces a per-doc verdict matrix:

| Doc | L1 chunks | L2 entities | L3 triples | L4 resolutions | Verdict |
|---|---|---|---|---|---|
| `silent-mode-summary-budget-cap-2026-05-08.md` | (count) | (count) | (count) | (success / no_match / missing) | `present`/`partial`/`absent` |
| `summarizer-factual-assertion-reform-2026-05-08.md` | (count) | (count) | (count) | (success / no_match / missing) | `present`/`partial`/`absent` |
| `verify-on-write-citation-discipline-2026-05-07.md` | (count) | (count) | (count) | (success / no_match / missing) | `present`/`partial`/`absent` |

A `present` verdict requires non-zero counts at L1, L2, L3 AND non-empty resolution log (any outcome class). A `partial` verdict means L1 is fine but L2 or L3 is empty. An `absent` verdict means L1 is empty (already disproven by reconnaissance).

### Outcome path declaration

The finding doc's TL;DR ends with one of:

- **Outcome A — KG healthy:** all three docs `present` at all layers; resolver tick + domain rebuild metrics nominal. Close ticket; no follow-up.
- **Outcome B — specific bug:** at least one doc has `partial`/`absent` verdict OR resolver tick shows persistent `aborted_budget=true` OR domain graph has zero/empty rebuild. Finding doc names the bug with a concrete fix shape; follow-up fix ticket(s) filed inline.
- **Outcome C — signals unclear:** mixed or insufficient signals (e.g., one doc has zero entities but the LLM extraction model legitimately found nothing because the doc is short). Finding doc records what was checked, names the ambiguity, and proposes either (i) deeper investigation or (ii) operator-judgment call.

## Implementation Steps

### Step 1: Write the investigation script

**File:** `mika/scripts/investigate-kg-1027.sh` (per architect F2 — repo-local, not workspace-level; the queries cite mika-internal schema and event names, so the script lives with the repo whose state it inspects).

Bash script that:

1. Sources `MIKA_SERVER_LOG_FILE` from environment or argv (default `/var/log/mika/server.log`).
2. Runs the six-layer queries from §"What the script checks" verbatim against `~/.mika/data/mika.db` (via `sqlite3`).
3. Filters the server log for KG events using **jq selectors against `.event` field** (per architect NF3), not text grep — the server emits structured JSON.
4. Produces a structured stdout report (markdown table per layer).
5. Captures a timestamp for the finding doc.

The script is idempotent (read-only) — safe to re-run. Uses `MIKA_SERVER_LOG_FILE` env var resolution: if unset, falls back to `/var/log/mika/server.log` and emits a WARN if read access fails.

**Per architect NF2: scope of reusability.** The script `investigate-kg-1027.sh` is a **per-ticket one-shot** — its filename is bound to mika#1027 and it queries this specific moment in the KG state. The **methodology** it embodies (six-layer post-deploy KG verification) IS reusable, but the script itself is not the reusable artifact. Future post-deploy verifications either (a) copy this script and rename per-ticket, or (b) consume the methodology distilled into a `solutions/best-practices/` doc per Step 6 below. Do not market this script as "the canonical post-deploy KG verification script" in CLAUDE.md.

### Step 2: Run the script + capture output

Run from the mika worktree:

```bash
bash mika/scripts/investigate-kg-1027.sh > /tmp/kg-1027-output.md
```

Output goes to `/tmp/`. The finding doc (Step 3) cites this output verbatim where load-bearing.

### Step 3: Author the finding doc

**File:** `mika/docs/audits/2026-05-08-001-investigate-kg-post-axis-deploy-sync.md` (per architect NF1 — `audits/` **unconditionally**, regardless of outcome path).

Rationale: an investigation produces an audit record. The audit is one-shot by definition — it documents what was checked at this moment in time. If the methodology turns out to be reusable, that's a separate artifact (Step 6, methodology follow-up) extracted to `solutions/best-practices/`. Writing the audit speculatively under `solutions/` (in case it's "reusable") collapses the audit-vs-pattern distinction the docs structure deliberately preserves.

The doc structure:

- **Frontmatter:** `module: agent-core`, `tags: [kg, post-deploy-verification, lexical-ingest, subject-extraction, resolution]`, `problem_type: post-deploy-verification` (Outcome A) or `problem_type: kg-bug-{specific}` (Outcome B).
- **TL;DR:** verdict per doc + outcome path declaration (A/B/C).
- **Reconnaissance:** the L1 chunk counts already established (above).
- **L2/L3/L4 results:** verbatim query outputs.
- **L5/L6 results:** verbatim log queries + counts.
- **Per-doc rollup matrix.**
- **Outcome path verdict + follow-up tickets** (named inline if Path B).

### Step 4: If Outcome B, file follow-up ticket(s)

Each named bug in the finding doc gets its own follow-up `mika#` issue. Link from the finding doc + from the follow-up ticket back to mika#1027.

### Step 5: Close mika#1027

Path A: finding doc declares healthy; close ticket; no further follow-up.
Path B: finding doc declares specific bugs; close ticket once follow-up tickets are filed (the investigation IS done).
Path C: finding doc declares unclear; close ticket once escalation is delivered to operator (the investigation IS done; subsequent action is operator-driven).

### Step 6: Methodology extraction (separate follow-up — out of mika#1027 scope)

Per architect NF1 + NF2: if the methodology turns out to be useful, file a **separate** follow-up ticket "feat(docs): post-deploy KG verification methodology" that extracts the reusable pattern from this audit into `mika/docs/solutions/best-practices/post-deploy-kg-verification.md` (no per-ticket suffix). That doc would (a) describe the six-layer check, (b) provide a template script (without ticket-numbered filename), and (c) wire into the deploy-protocol runbook §3 KG-deploy smoke probe.

This step is **out of mika#1027 scope**. mika#1027 produces the audit record. The methodology extraction is a separate decision after the audit's results inform whether the methodology is sound enough to canonize.

If Outcome A (healthy + methodology proven): file the follow-up ticket as a normal Tier 1 work item.
If Outcome B/C: file the follow-up ticket only if the methodology itself was sound (i.e., the bugs surfaced were specific to data, not to the method).

## Test Strategy

### Unit tests

None directly — this is investigation-only. The deliverable is a finding doc + (optionally) a script + (optionally) a CLAUDE.md update. No production code changes.

### Self-validating script

The investigation script (Step 1) should print clear PASS/FAIL signals per layer. A future operator running the script on a healthy system sees PASS across all six layers; on a broken system sees FAIL at the broken layer. Self-validating in this sense.

### Re-runnability

Running the script on the same DB twice produces identical output (idempotent). Running across server-restart boundaries (with new resolver-tick events accumulated) produces extended log-grep output but the SQL counts may grow (if new docs ingested) — this is correct.

## Acceptance Criteria

Mirroring the ticket body for traceability:

- **AC#1**: Finding doc produced at `mika/docs/audits/2026-05-08-001-investigate-kg-post-axis-deploy-sync.md` (per architect NF1 — `audits/` unconditionally), naming this ticket and the three compound docs investigated.
- **AC#2**: Per-doc verdict for each of the three compound docs across the four KG SQL layers (L1 chunking, L2 extraction, L3 relationships, L4 resolution). Each verdict is one of: `present`, `partial-bug-flagged`, `absent-bug-flagged`. Verdicts derived from the **verbatim queries** in §"What the script checks" above (per architect F1).
- **AC#3**: Resolver tick health summary: number of ticks since 2026-05-07T16:00Z, number with `aborted_budget=true`, current `pending_after` value (per agent — mika-arch is the only KG consumer per mika#800). Derived from the L5 jq queries (per architect NF3 — jq selectors, not grep).
- **AC#4**: Per-corpus fairness summary: any corpus with pending entities and zero attempts on recent ticks (mika#927 violation indicator). Cite the `per_corpus_attempted` JSON field from `kg_resolver_tick.complete` events directly.
- **AC#5**: Outcome path declared (A/B/C) in the finding doc's TL;DR; follow-up fix ticket(s) filed if Path B (separate from the methodology-extraction follow-up).
- **AC#6**: Investigation script committed at `mika/scripts/investigate-kg-1027.sh` (per architect F2 — repo-local, not workspace-level). Idempotent (read-only). Sources `MIKA_SERVER_LOG_FILE` from env or argv. Per architect NF2: this is a **per-ticket one-shot**, not a canonical reusable script; methodology extraction (if pursued) is a separate follow-up ticket per Step 6.

## Risks & Open Questions

- **R1 (low):** `MIKA_SERVER_LOG_FILE` access may be restricted (mode 0640 or owner-only). The script must fall back gracefully — if log file is unreadable, layers 5 + 6 are reported as "log access denied" with a recovery note ("re-run with sudo or update MIKA_SERVER_LOG_FILE perms"); SQL layers 1–4 still produce results.
- **R2 (low):** v27 schema dependency. The investigation queries use `docs_root_hash` joins (post-v27) implicitly — `kg_extractions` and the shared-corpus tables. If the DB has somehow regressed to pre-v27, the queries fail. Pre-flight check: `SELECT MAX(version) FROM schema_version` — assert ≥ 27 before running the per-layer queries; fail loud if not.
- **R3 (low):** mika-arch is the only KG consumer (per mika#800); mika-dev/qa have `[kg].enabled = false`. The script must scope agent-specific queries to mika-arch and not falsely flag missing data for the other agents. Layer 1 (chunks) is shared-corpus and agent-independent. Layers 2–4 are agent-scoped via `agent_kg_corpora` join. Layers 5 + 6 are agent-emitted log events; filter by `agent_id="mika-arch"` for resolver-tick analysis.
- **R4 (low):** Recent-doc skew. The morning's compound docs (#1022, #1025) were committed AFTER server-startup ingest could have indexed them. They depend on the resolver tick (#906) running post-merge. If the most recent server restart was before the doc was committed, the doc would still be missing from extraction layer until the next 30-min tick fires. Pre-flight check: capture the most recent server-startup time + the ingest hook's last-run time; reconcile against doc commit timestamps before declaring an absent-bug-flagged verdict. Time-skew false-positive guard. **Per architect second-pass NF4:** the script must emit a third verdict class — `PENDING` — when `last_chunked > last_tick_complete` (i.e., the chunk landed after the most recent resolver-tick fired). A `PENDING` verdict is NOT an `absent-bug-flagged` outcome; it's "the system hasn't had a chance to process this yet." The finding doc records `PENDING` per doc and re-runs the investigation after the next tick fires (or the operator manually triggers a tick if urgent). Without this, the audit could file a false Outcome B against a system that's working correctly.

**Resolved by reconnaissance (no longer open):**
- Q1 (was: are L1 chunks present?): yes — see Reconnaissance § above. Verified pre-grooming.

**Resolved by architect first-pass:**
- OQ1 (Bash vs Rust): Bash + sqlite3 + jq confirmed. Rust binary is overkill for read-only one-shot work.
- OQ2 (finding doc path): `audits/` **unconditionally**, regardless of outcome. Methodology extraction is a separate follow-up if pursued.
- OQ3 (script location): `mika/scripts/` (repo-local), not workspace-level — the queries cite mika-internal schema and event names.

No open questions remain.

## Sources

- mika#1009 finding doc + 4-axis fix plan (parent context)
- `mika/docs/solutions/best-practices/silent-mode-summary-budget-cap-2026-05-08.md`
- `mika/docs/solutions/best-practices/summarizer-factual-assertion-reform-2026-05-08.md`
- `mika/docs/solutions/best-practices/verify-on-write-citation-discipline-2026-05-07.md`
- `crates/mika-agent/CLAUDE.md` § Knowledge Graph (extractor, resolver, query, post-restart safety check Signals A–F)
- `crates/mika-agent/src/kg/` — extractor, resolver, query, config modules
- mika#906 (resolver tick), mika#927 (per-corpus fairness), mika#800 (KG topology), mika#876 (parse tolerance)
- mika#786, mika#787 (v27 shared-corpus migration)
- `mika_priority_stack_2026-05` Tier 1 #2

## Out of scope

- Fixing any bug surfaced (file follow-up ticket(s) per Outcome B).
- KG performance/latency tuning.
- KG schema redesign.
- Adding new compound docs (these tests check existing ones; the resolver tick will pick up new ones automatically).
