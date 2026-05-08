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

### What the script checks (six layers)

For each of the three target docs (`source_doc_path`):

**Layer 1 — Chunking** (`kg_chunks`):
```sql
SELECT source_doc_path, COUNT(*) AS chunks, source_doc_hash, MAX(created_at) AS last_chunked
FROM kg_chunks
WHERE source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR source_doc_path LIKE '%summarizer-factual-assertion%'
   OR source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY source_doc_path;
```
Expected: ≥1 chunk per doc, valid `source_doc_hash`. Already verified pre-investigation (see Reconnaissance § above) — script confirms current state.

**Layer 2 — Subject extraction** (`kg_subject_entities` + `kg_chunk_subjects`):
```sql
SELECT c.source_doc_path,
       COUNT(DISTINCT cs.subject_entity_id) AS entities,
       COUNT(*) AS provenance_rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path;
```
Expected: entity count > 0 per doc. Zero count = extraction layer drop (Path B finding).

**Layer 3 — Subject relationships** (`kg_subject_relationships` + `kg_chunk_subject_relationships`):
```sql
SELECT c.source_doc_path,
       COUNT(DISTINCT csr.subject_relationship_id) AS triples
FROM kg_chunks c
JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path;
```
Expected: triple count ≥ 1 per doc.

**Layer 4 — Resolution** (`kg_resolutions_log` joined back through chunks):
```sql
SELECT c.source_doc_path,
       rl.outcome,
       COUNT(*) AS rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id
JOIN kg_resolutions_log rl ON rl.subject_entity_id = cs.subject_entity_id
WHERE c.source_doc_path LIKE '%silent-mode-summary-budget-cap%'
   OR c.source_doc_path LIKE '%summarizer-factual-assertion%'
   OR c.source_doc_path LIKE '%verify-on-write-citation%'
GROUP BY c.source_doc_path, rl.outcome;
```
Expected per doc: each subject entity has at least one `kg_resolutions_log` row. Outcomes break down into:
- `exact_match`, `matched_llm`, `matched_llm_db_fallback` — successful resolutions
- `no_match` — entity not in domain graph (acceptable for novel subject-only concepts like "Axis 3", "load-omit sentinel")
- `skipped_no_llm` — resolution model not configured (acceptable degraded mode; flag in finding)

Missing rows entirely (no log entry for an entity) = resolution layer drop (Path B finding).

**Layer 5 — Resolver tick health** (server log grep, post-deploy onward):
```bash
grep 'kg_resolver_tick' "$MIKA_SERVER_LOG_FILE" | jq -c 'select(.timestamp > "2026-05-07T16:00:00Z") | {ts: .timestamp, event: .event, agent_id, pending_after, llm_calls, aborted_budget, per_corpus_attempted}'
```
Expected: `kg_resolver_tick.complete` events on a ~30-min cadence per agent; `aborted_budget=false` on every tick; `pending_after` trends to 0 across ticks (per Signal E in `crates/mika-agent/CLAUDE.md`); `per_corpus_attempted` JSON shows non-zero attempts on every tick if pending > 0 per corpus (per Signal F, mika#927).

**Layer 6 — Domain graph health**:
```bash
grep 'domain_rebuild' "$MIKA_SERVER_LOG_FILE" | jq -c 'select(.timestamp > "2026-05-08T08:00:00Z") | {ts: .timestamp, event: .event, entities, edges}'
```
```sql
SELECT COUNT(*) FROM kg_entities;
SELECT COUNT(*) FROM kg_relationships;
SELECT entity_type, COUNT(*) FROM kg_entities GROUP BY entity_type;
```
Expected: `domain_rebuild_complete` event present in server log after each post-deploy server start; `kg_entities` count > 0 with sane breakdown across types (skill/tool/agent/problem_type/concept). Zero count = domain rebuild silently failed (high-severity Path B finding).

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

**File:** `scripts/investigate-kg-1027.sh`

Bash script that:

1. Sources `MIKA_SERVER_LOG_FILE` from environment or argv (default `/var/log/mika/server.log`).
2. Runs the six-layer queries against `~/.mika/data/mika.db` (via `sqlite3`).
3. Greps the server log for `kg_resolver_tick` and `domain_rebuild` events with timestamp filtering.
4. Produces a structured stdout report (markdown table per layer).
5. Captures the session ID + timestamp for the finding doc.

The script is idempotent (read-only) — safe to re-run. Uses `MIKA_SERVER_LOG_FILE` env var resolution: if unset, falls back to `/var/log/mika/server.log` and emits a WARN if read access fails.

### Step 2: Run the script + capture output

Run from the worktree:

```bash
bash scripts/investigate-kg-1027.sh > /tmp/kg-1027-output.md
```

Output goes to `/tmp/`. The finding doc (Step 3) cites this output verbatim where load-bearing.

### Step 3: Author the finding doc

**File:** `mika/docs/audits/2026-05-08-001-investigate-kg-post-axis-deploy-sync.md` (or `mika/docs/solutions/best-practices/post-deploy-kg-verification-2026-05-08.md` if the methodology generalizes).

Path choice rationale: if Outcome A (healthy), the methodology IS the deliverable — file it under `solutions/best-practices/` so the next post-deploy verification can reuse the script + finding shape. If Outcome B (specific bug), the finding is the deliverable — file under `audits/` as a one-shot investigation record. Decide at write-time.

The doc structure:

- **Frontmatter:** `module: agent-core`, `tags: [kg, post-deploy-verification, lexical-ingest, subject-extraction, resolution]`, `problem_type: post-deploy-verification` (Outcome A) or `problem_type: kg-bug-{specific}` (Outcome B).
- **TL;DR:** verdict per doc + outcome path declaration.
- **Reconnaissance:** the L1 chunk counts already established (above).
- **L2/L3/L4 results:** verbatim query outputs.
- **L5/L6 results:** verbatim log greps + counts.
- **Per-doc rollup matrix.**
- **Outcome path verdict + follow-up tickets.**
- **Methodology section** (Outcome A path): how a future operator runs the same verification post-deploy.

### Step 4: If Outcome B, file follow-up ticket(s)

Each named bug in the finding doc gets its own follow-up `mika#` issue. Link from the finding doc + from the follow-up ticket back to mika#1027.

### Step 5: Update CLAUDE.md (Outcome A only)

If Outcome A, add one sentence to `crates/mika-agent/CLAUDE.md` § Knowledge Graph (post-restart safety check section) referencing the verification script + finding doc as the canonical post-deploy KG check. This wires the script into the runbook §3 KG-deploy smoke probe.

If Outcome B, do NOT update CLAUDE.md — wait for the fix ticket(s) to land first.

## Test Strategy

### Unit tests

None directly — this is investigation-only. The deliverable is a finding doc + (optionally) a script + (optionally) a CLAUDE.md update. No production code changes.

### Self-validating script

The investigation script (Step 1) should print clear PASS/FAIL signals per layer. A future operator running the script on a healthy system sees PASS across all six layers; on a broken system sees FAIL at the broken layer. Self-validating in this sense.

### Re-runnability

Running the script on the same DB twice produces identical output (idempotent). Running across server-restart boundaries (with new resolver-tick events accumulated) produces extended log-grep output but the SQL counts may grow (if new docs ingested) — this is correct.

## Acceptance Criteria

Mirroring the ticket body for traceability:

- **AC#1**: Finding doc produced at the appropriate path (`docs/audits/` or `docs/solutions/best-practices/`), naming this ticket and the three compound docs investigated.
- **AC#2**: Per-doc verdict for each of the three compound docs across the four KG SQL layers (chunking, subject extraction, subject relationships, resolution). Each verdict is one of: `present`, `partial-bug-flagged`, `absent-bug-flagged`. Verdicts derived from the queries in §"What the script checks" above.
- **AC#3**: Resolver tick health summary: number of ticks since 2026-05-07T16:00Z (per agent, per corpus), number with `aborted_budget=true`, current `pending_after` value (per agent).
- **AC#4**: Per-corpus fairness summary: any corpus with pending entities and zero attempts on recent ticks (mika#927 violation indicator). Cite the `per_corpus_attempted` JSON field directly.
- **AC#5**: Outcome path declared (A/B/C) in the finding doc's TL;DR; follow-up ticket(s) filed if path B; CLAUDE.md updated if path A.
- **AC#6**: Investigation script committed at `scripts/investigate-kg-1027.sh`. Idempotent. Sources `MIKA_SERVER_LOG_FILE`.

## Risks & Open Questions

- **R1 (low):** `MIKA_SERVER_LOG_FILE` access may be restricted (mode 0640 or owner-only). The script must fall back gracefully — if log file is unreadable, layers 5 + 6 are reported as "log access denied" with a recovery note ("re-run with sudo or update MIKA_SERVER_LOG_FILE perms"); SQL layers 1–4 still produce results.
- **R2 (low):** v27 schema dependency. The investigation queries use `docs_root_hash` joins (post-v27) implicitly — `kg_extractions` and the shared-corpus tables. If the DB has somehow regressed to pre-v27, the queries fail. Pre-flight check: `SELECT MAX(version) FROM schema_version` — assert ≥ 27 before running the per-layer queries; fail loud if not.
- **R3 (low):** mika-arch is the only KG consumer (per mika#800); mika-dev/qa have `[kg].enabled = false`. The script must scope agent-specific queries to mika-arch and not falsely flag missing data for the other agents. Layer 1 (chunks) is shared-corpus and agent-independent. Layers 2–4 are agent-scoped via `agent_kg_corpora` join. Layers 5 + 6 are agent-emitted log events; filter by `agent_id="mika-arch"` for resolver-tick analysis.
- **R4 (low):** Recent-doc skew. The morning's compound docs (#1022, #1025) were committed AFTER server-startup ingest could have indexed them. They depend on the resolver tick (#906) running post-merge. If the most recent server restart was before the doc was committed, the doc would still be missing from extraction layer until the next 30-min tick fires. Pre-flight check: capture the most recent server-startup time + the ingest hook's last-run time; reconcile against doc commit timestamps before declaring an absent-bug-flagged verdict. Time-skew false-positive guard.

**Resolved by reconnaissance (no longer open):**
- Q1 (was: are L1 chunks present?): yes — see Reconnaissance § above. Verified pre-grooming.

**Open questions for architect:**
- OQ1: should the investigation script be a Bash script + sqlite3 + jq, or a Rust binary with structured output? Plan currently says Bash for reproducibility + low ceremony; architect: confirm or counter-propose.
- OQ2: Outcome A path file location — `docs/solutions/best-practices/` (methodology + reusable) vs `docs/audits/` (one-shot). Plan defers to write-time; architect: prefer one over the other?
- OQ3: should AC#6 require the script to live under `scripts/` (workspace-level reusable) or `mika/scripts/` (mika-only)? Plan currently leaves at top-level `scripts/`. Architect: which?

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
