---
title: KG post-deploy verification methodology — six-layer audit
date: 2026-05-08
category: best-practices
module: agent-core
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - After deploying KG-related changes (extractor, resolver, domain builder)
  - When compound docs are added and need to confirm KG ingestion
  - When resolver tick health needs verification post-restart
tags: [kg, post-deploy-verification, lexical-ingest, subject-extraction, resolution, audit]
---

# KG Post-Deploy Verification Methodology — Six-Layer Audit

## Context

After deploying KG-related changes or adding new compound docs to `docs/solutions/`, operators need to verify the full KG pipeline is functioning: chunking, subject extraction, resolution, and resolver tick health. The KG pipeline has four async stages (chunk → extract → resolve → query) that run on different schedules — chunking at startup, extraction as a background batch, resolution via 30-min resolver ticks. A doc that's chunked but not yet extracted is not a bug — it's the system working through its queue.

This methodology was developed during mika#1027 (post-axis-deploy KG sync verification) and proven against a live system with 133 pending docs across 5 corpora.

## Guidance

### Six verification layers

| Layer | Table(s) | What it checks | PASS | FAIL |
|-------|----------|---------------|------|------|
| L1 — Chunking | `kg_chunks` | Doc was split into chunks | `chunks > 0`, non-NULL `source_doc_hash` | Doc missing from chunks |
| L2 — Extraction | `kg_chunk_subjects`, `kg_extractions` | Subject entities extracted from chunks | `entities > 0` per doc | 0 entities when `kg_extractions` row exists |
| L3 — Relationships | `kg_chunk_subject_relationships` | Fact triples extracted | `triples > 0` per doc | 0 triples when entities exist |
| L4 — Resolution | `kg_resolutions_log` | Entities resolved against domain graph | Resolution log rows exist | Entities with zero log rows |
| L5 — Resolver tick | Server log (`kg_resolver_tick.complete`) | Tick health metrics | `aborted_budget=false`, `pending_after` trending to 0 | Persistent `aborted_budget=true` |
| L6 — Domain graph | `kg_entities`, server log (`domain_rebuild_complete`) | Domain graph rebuilt | `total_entities > 0`, rebuild event present | Zero entities or missing rebuild |

### Critical schema awareness

Post-v27, the shared-corpus tables (`kg_chunks`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`) are keyed by `docs_root_hash`, NOT `agent_id`. The correct join between `kg_chunks` and `kg_chunk_subjects` is:

```sql
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.docs_root_hash  -- corpus hash, NOT source_doc_hash
```

`source_doc_hash` is a per-document content hash for idempotency. `docs_root_hash` is a per-corpus identifier (16-hex SHA-256 prefix of the canonical docs root path). Confusing these produces zero-row joins.

### PENDING verdict (not a bug)

When L1 shows chunks but L2 shows zero entities, check `kg_extractions` for the doc. If no extraction record exists, the verdict is **PENDING** — the extraction batch hasn't reached this doc yet. This is expected when:
- The doc was committed after the last extraction batch started
- The extraction backlog exceeds what one batch processes (budget-limited at `MIKA_KG_BATCH_BUDGET`)

### Log parsing for large files

The server log may be multi-GB with non-JSON lines mixed in. Use `grep -a` to pre-filter before `jq`:

```bash
TICK_LINES=$(grep -a "kg_resolver_tick" "$LOG_FILE" | grep "$AGENT_ID" || true)
echo "$TICK_LINES" | jq -c 'select(.event == "kg_resolver_tick.complete") | ...'
```

Direct `jq` on the full file fails on non-JSON lines and is extremely slow on large files.

## Why This Matters

KG verification catches two classes of issues:
1. **Silent drops** — a doc is chunked but never extracted (extraction model unset, parse tolerance regression, idempotency-marker bug)
2. **Resolver stalls** — entities are extracted but never resolved (`aborted_budget=true` on every tick, fairness violation per mika#927)

Without a structured verification methodology, operators rely on `pending_after` trending to 0 over multiple restarts — which doesn't distinguish "working through backlog" from "silently stuck."

## When to Apply

- After any deploy that touches `crates/mika-agent/src/kg/` (extractor, resolver, domain builder)
- After adding new compound docs that should appear in the KG
- When `kg_budget_exhausted` WARN appears in server logs
- When investigating KG query_knowledge_graph results that seem stale

## Examples

The reference implementation is `scripts/investigate-kg-1027.sh` — a per-ticket one-shot script for mika#1027. For future verifications, adapt the target doc patterns and cutoff timestamps.

Key query for checking extraction status of specific docs:

```sql
-- Are target docs pending extraction?
SELECT DISTINCT c.source_doc_path
FROM kg_chunks c
WHERE c.docs_root_hash = '<corpus_hash>'
  AND NOT EXISTS (
    SELECT 1 FROM kg_extractions e
    WHERE e.docs_root_hash = c.docs_root_hash
      AND e.source_doc_path = c.source_doc_path
  )
  AND c.source_doc_path LIKE '%<target-pattern>%';
```

## Related

- mika#1027 — investigation ticket that developed this methodology
- `docs/audits/2026-05-08-001-investigate-kg-post-axis-deploy-sync.md` — finding doc
- `scripts/investigate-kg-1027.sh` — reference implementation
- mika#906 (resolver tick), mika#927 (per-corpus fairness), mika#800 (KG topology)
- CLAUDE.md § Post-restart safety check (Signals A–F)
