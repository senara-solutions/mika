---
title: "Schema migrations as integration-test events"
module: kg
date: 2026-04-23
problem_type: best_practice
component: database
severity: high
tags:
  - knowledge-graph
  - schema-migrations
  - integration-testing
  - operational-readiness
related_issues: [757, 767]
---

# Schema migrations as integration-test events

## Problem

Migration correctness does not imply downstream correctness. A schema migration that changes what a "pending work" query returns is an **integration-test event**, not just a migration-test event.

The `migrate_v25_to_v26` test validates the DDL — it confirms the column is added, the data is transformed, and the schema version is incremented. It does NOT validate that all code paths processing the now-larger pending set survive realistic production input distributions. The migration is correct in isolation. The failures are second-order effects: changing what the pending-work query returns changes the *volume* and *distribution* of inputs to downstream consumers.

This pattern generalizes beyond KG. Any migration that introduces a tracking table, invalidates processing markers, or alters a "what needs doing" query creates a first-boot event whose cost and failure modes must be evaluated before shipping — not discovered at runtime.

## Two concrete incidents (2026-04-23)

Both incidents occurred on the same deploy and share the same root cause: schema v25→v26 changed the pending-work query surface without integration-level verification of the downstream consumers.

### Incident 1: Cost spike (#757)

The v26 migration introduced `kg_extractions` — a tracking table that records which docs have been extracted. On first boot, the table was empty (no prior "already extracted" rows), so the pending-doc query returned every doc across every agent.

- **Scale:** 11 agents × 283 shared docs → ~3,113 extractions + per-entity resolution fan-out → ~30,400 LLM calls over 38 minutes
- **Cost:** ~$40–60 against `claude-haiku-4-5`, exhausting Anthropic Console API credit
- **Fix:** Per-batch budget guard (`MIKA_KG_BATCH_BUDGET`, default 500), hash-based content idempotency (`kg_extractions.source_doc_hash`), atomic marker writes within the extraction transaction, provider-choice advisory (`kg_anthropic_provider` startup WARN)
- **Detail:** See [`first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`](first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md)

### Incident 2: KG UTF-8 panic (#764)

The same v26 migration invalidated extraction markers, triggering full re-extraction. The resolver then scheduled ~5,000 subject entities per agent. The first multi-byte UTF-8 character (an em-dash at byte offset 2000) panicked the `tokio::spawn`'d resolver tasks via `&str[..N]` byte slicing.

- **Scale:** 14 unsafe byte-slicing sites across resolver, extractor, and workspace-wide code; 27 panic events across all 11 agents in `server.log`
- **Impact:** KG resolution fully broken — zero `kg_resolutions_log` writes for 5+ hours. Panics invisible to tracing because `tokio::spawn` JoinHandles were discarded
- **Fix:** Shared `safe_truncate()` helper using `str::floor_char_boundary()`, spawn-site panic containment (outer spawn + JoinHandle await), CI `byte-slice-lint` regression guard
- **Detail:** See [`../runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md`](../runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md)

## Why migration tests don't catch this

Schema migration tests validate three things: (1) DDL correctness (table/column exists after migration), (2) data transformation correctness (existing rows are converted properly), and (3) version increment (schema version advances). All three passed for v25→v26.

The failures are second-order effects that migration tests are structurally unable to catch:

- **Volume amplification.** The migration changed what the pending-work query returns from "incrementally new docs" to "every doc ever ingested." The migration test doesn't run the consumer pipeline, so it can't observe the volume spike.
- **Input distribution shift.** Pre-migration, the consumer saw small batches of newly-ingested docs. Post-migration, it saw the entire corpus — including docs with multi-byte UTF-8 characters that the consumer had never encountered at scale. The migration test uses simplified fixtures, not production-shaped content.
- **Fan-out multiplication.** Per-agent isolation means N agents × M docs. The migration test runs against a single-agent test database with a handful of rows.

The gap is structural: migration tests validate the schema change; integration tests validate the system's behavior after the schema change. These are different concerns, and treating the first as sufficient for the second is the failure pattern.

## Operational checklist for schema-change PRs

Before shipping any migration that touches "what needs processing" queries, answer these questions:

1. **Pending-set size change?**
   Estimate the upper bound of the pending set on first boot after this migration. If the consumer does non-trivial per-unit work (LLM calls, external API calls, heavy compute), multiply by the per-unit cost and by the consumer fan-out multiplier (agents × tenants). Document the estimate in the PR description.

2. **Processing markers invalidated?**
   Does this migration invalidate existing "already processed" markers (by adding a new tracking table, changing a hash column, or altering the pending-set query)? If yes, estimate the re-processing cost and verify that structural budget guards cap the spike. A prompt-level "be mindful of cost" is not a budget guard.

3. **Realistic input testing?**
   Run the downstream pipeline against production-shaped content at the estimated scale — not simplified fixtures. The #764 panic only manifested on multi-byte UTF-8 characters that appeared in real compound docs but not in test fixtures.

4. **Deploy plan includes "watch first restart"?**
   The first restart after a tracking-table migration is a financial and operational event. The deploy plan should include concrete signals to watch: cost spikes, panics, backlog growth rates. See the post-deploy verification signals (Signals A–J) documented in `CLAUDE.md` for the KG-specific instance of this pattern.

## Implementation hooks

These are options to enforce the checklist — not proposals for immediate implementation:

- **CI check** that flags migrations touching tracking or marker tables (e.g., tables matching `*_extractions`, `*_processed`, `*_seen`) and requires a `## First-boot impact` section in the PR description
- **Plan-template question** ("What is the first-boot cost of this migration?") enforced by architect review — the mika-arch groom-ticket skill could include this as a required consideration for migration-bearing plans
- **Staging-env integration test** that runs the consumer pipeline against a production-scale pending set after the migration, measuring cost and checking for panics
- **Mandatory `Post-deploy verification` section** in migration PR description template, with operator-facing signals to watch on the first restart

## Related

- **Incident tickets:** [mika#757](https://github.com/senara-solutions/mika/issues/757), [mika#767](https://github.com/senara-solutions/mika/issues/767)
- **KG bug ticket (UTF-8 panic):** [mika#764](https://github.com/senara-solutions/mika/issues/764)
- **Rescue PR (idempotency + budget guard):** [mika#759](https://github.com/senara-solutions/mika/pull/759)
- **Incident deep-dives:**
  - [`first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`](first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md) — #757 cost spike, budget guards, idempotency, fan-out
  - [`../runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md`](../runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md) — #764 UTF-8 panic, safe_truncate, spawn containment, CI lint
- **Code cross-reference:** `crates/mika-agent/src/db/kg_schema.rs` — idempotency contract documentation + pointer to this checklist
- **CI regression guard:** `scripts/check-byte-slices.sh` — byte-slice lint added after #764
