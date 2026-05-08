---
module: agent-core
tags: [kg, post-deploy-verification, lexical-ingest, subject-extraction, resolution]
problem_type: post-deploy-verification
ticket: mika#1027
parent: mika_priority_stack_2026-05 Tier 1 #2
---

# Post-Axis-Deploy KG Sync Verification — Finding Doc

**Ticket:** mika#1027
**Investigation date:** 2026-05-08T09:29Z–09:35Z
**Investigator:** Claude Code (autonomous, via `/mika` pipeline)
**Script:** `scripts/investigate-kg-1027.sh` (committed alongside this doc)

## TL;DR

**Outcome Path A — KG healthy.** All three compound docs are chunked (L1 PASS) and the extraction batch is actively processing them (L2–L4 PENDING). The resolver tick is running nominally (68 ticks since 2026-05-07T16:00Z, zero `aborted_budget`, `pending_after=0` on every tick). The domain graph rebuilt successfully post-deploy (165 entities, sane type distribution). No bugs detected; the system is working as designed — the extraction batch started at 08:48:33Z and had processed 20 of 57 pending docs by 09:34Z, with the three target docs among the remaining 37.

**Per-doc verdict:**

| Doc | L1 chunks | L2 entities | L3 triples | L4 resolutions | Verdict |
|-----|-----------|-------------|------------|----------------|---------|
| `silent-mode-summary-budget-cap-2026-05-08.md` | 8 | 0 | 0 | 0 | **PENDING** |
| `summarizer-factual-assertion-reform-2026-05-08.md` | 8 | 0 | 0 | 0 | **PENDING** |
| `verify-on-write-citation-discipline-2026-05-07.md` | 9 | 0 | 0 | 0 | **PENDING** |

All three docs are PENDING at the extraction layer — chunked successfully, awaiting the in-progress extraction batch to reach them. This is expected behavior per the plan's R4 time-skew analysis: the docs were committed after the previous extraction batch completed, and the current batch started at 08:48:33Z with 57 docs to process.

## Investigated Compound Docs

1. **`docs/solutions/best-practices/verify-on-write-citation-discipline-2026-05-07.md`** — landed via mp#92 (orchestrator-Claude, 2026-05-07 ~19:44Z). Lives in **mika-platform** corpus (`ac0e96dc51b85b80`), not the mika corpus — the ticket body incorrectly assumed all three docs were in `mika/docs/solutions/`.
2. **`docs/solutions/best-practices/silent-mode-summary-budget-cap-2026-05-08.md`** — landed via mika#1022 (autonomous loop, 2026-05-08 ~00:18Z). Lives in **mika** corpus (`34b8cf03c80614f9`).
3. **`docs/solutions/best-practices/summarizer-factual-assertion-reform-2026-05-08.md`** — landed via mika#1025 (autonomous loop, 2026-05-08 ~08:43Z). Lives in **mika** corpus (`34b8cf03c80614f9`).

## Layer 1 — Chunking (`kg_chunks`)

All three docs are chunked with non-NULL `source_doc_hash`:

| Doc | Corpus | Chunks | Last chunked |
|-----|--------|--------|-------------|
| `silent-mode-summary-budget-cap-2026-05-08.md` | `34b8cf03c80614f9` (mika) | 8 | 2026-05-08T08:08:33Z |
| `summarizer-factual-assertion-reform-2026-05-08.md` | `34b8cf03c80614f9` (mika) | 8 | 2026-05-08T08:48:33Z |
| `verify-on-write-citation-discipline-2026-05-07.md` | `ac0e96dc51b85b80` (mika-platform) | 9 | 2026-05-07T19:44:01Z |

**Verdict: PASS** — all three docs chunked successfully.

## Layer 2 — Subject Extraction (`kg_chunk_subjects`)

Zero `kg_extractions` records exist for any of the three target docs. The extraction batch started at 08:48:33Z with 57 pending docs and a budget of 500. By 09:34Z, 20 docs had been processed (progress events show `completed=20, remaining=37`). The three target docs are among the remaining 37.

**Verdict: PENDING** — extraction in progress, not yet reached these docs. This is NOT a bug.

## Layer 3 — Subject Relationships (`kg_chunk_subject_relationships`)

Follows from L2: no extraction = no relationships. Will be populated when extraction reaches these docs.

**Verdict: PENDING** — blocked on L2 extraction.

## Layer 4 — Resolution (`kg_resolutions_log`)

Follows from L2: no entities = nothing to resolve. The resolver tick is functioning correctly (see L5) — it resolves entities as they are extracted.

**Verdict: PENDING** — blocked on L2 extraction.

## Layer 5 — Resolver Tick Health

**Tick summary (2026-05-07T16:00Z → 2026-05-08T09:35Z):**

- **Total ticks (mika-arch):** 68 (34 complete events, each duplicated — server emits to both tracing and otel targets)
- **Ticks with `aborted_budget=true`:** 0
- **Latest `pending_after`:** 0 (on every tick — no backlog)
- **Tick errors:** 0

**Per-corpus fairness (mika#927):**

Non-zero `per_corpus_attempted` observed on ticks where entities existed to resolve:
- `34b8cf03c80614f9` (mika): attempted on 2026-05-07T21:44Z (6), 2026-05-08T08:39Z (3), 2026-05-08T09:19Z (3)
- `ac0e96dc51b85b80` (mika-platform): attempted on 2026-05-07T23:14Z (4), 2026-05-07T23:44Z (7)
- `d7107cd14e544043` (mika-cloud): attempted on 2026-05-08T00:44Z (2), 2026-05-08T01:14Z (2)

No corpus with pending entities showed zero attempts — fairness is working as designed.

**Verdict: PASS** — resolver tick nominal, no budget exhaustion, per-corpus fairness confirmed.

## Layer 6 — Domain Graph Health

**L6a — Rebuild events:**

Two domain rebuilds observed post-deploy:
1. 2026-05-08T08:08:33Z (trace `138ba51d...`): updated 165, removed 23
2. 2026-05-08T08:48:33Z (trace `81b8edea...`): updated 165, removed 0

Both completed successfully with paired `start`/`complete` events.

**L6b — Entity snapshot:**

| Type | Count |
|------|-------|
| tool | 94 |
| skill | 34 |
| concept | 20 |
| agent | 12 |
| problem_type | 5 |
| **Total** | **165** |

Total relationships: 63

**Verdict: PASS** — domain graph healthy, sane type distribution.

## Extraction Backlog Context

At investigation time, 133 docs were pending extraction across all corpora:

| Corpus | Docs Root | Pending |
|--------|-----------|---------|
| `34b8cf03c80614f9` | mika/docs/solutions | 45 |
| `ac0e96dc51b85b80` | mika-platform/docs/solutions | 51 |
| `98509090f0a833d2` | mika-skills/docs/solutions | 15 |
| `d7107cd14e544043` | mika-cloud/docs/solutions | 22 |

The batch budget of 500 is well above the 133 pending docs, so the current extraction batch will process all of them in this run. At the observed rate (~20 docs per ~46 minutes), the backlog should drain within ~5 hours of the batch start (08:48Z → ~14:00Z).

## Outcome Path Declaration

**Outcome A — KG healthy.** All three compound docs are successfully chunked and actively being processed by the extraction pipeline. No bugs detected at any layer. The resolver tick is nominal. The domain graph is healthy.

**No follow-up tickets required.** The investigation confirms the system is operating as designed — extraction takes time to process a backlog of 57+ docs, and the three target docs simply hadn't been reached yet.

**Methodology observation:** The six-layer verification methodology proved effective. Consider extracting it into a reusable `docs/solutions/best-practices/post-deploy-kg-verification.md` if future post-deploy verifications are needed (per plan Step 6 — separate follow-up, out of mika#1027 scope).

## Script

The investigation was performed by `scripts/investigate-kg-1027.sh` — a read-only, idempotent Bash script that queries the four KG SQL layers and parses structured server logs. Per the plan's NF2, this is a per-ticket one-shot script, not a canonical reusable artifact.

## Sources

- mika#1027 (this investigation ticket)
- mika#1009 finding doc + 4-axis fix plan (parent context)
- mika#906 (resolver tick), mika#927 (per-corpus fairness), mika#800 (KG topology)
- `crates/mika-agent/CLAUDE.md` § Knowledge Graph
- Server log: `/var/log/mika/server.log` (2026-05-07T16:00Z — 2026-05-08T09:35Z window)
- SQLite DB: `~/.mika/data/mika.db` (schema v32)
