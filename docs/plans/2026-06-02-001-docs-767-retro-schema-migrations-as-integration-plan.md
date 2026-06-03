# Plan: docs(retro) — schema migrations as integration-test events (#767)

## Overview

Write a compound doc at `docs/solutions/best-practices/schema-migrations-as-integration-events-2026-04-23.md` that captures the meta-pattern: schema migrations that change "what needs processing" queries are **integration-test events**, not just migration-test events. Two incidents from 2026-04-23 (mika#757 cost spike + KG UTF-8 panic) are evidence. The doc includes an operational checklist for future schema PRs.

### Overlap with existing docs

Two existing docs already cover the individual incidents in detail:

- `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — deep dive on #757 (cost spike, budget guards, idempotency, fan-out)
- `docs/solutions/runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md` — deep dive on the UTF-8 panic (14 resolver/extractor sites per that doc; #767 body scope is 10 — see §2 Incident 2)

This new doc is the **synthesis layer** — it abstracts the shared pattern from both incidents rather than repeating their details. The two incident docs are evidence; this doc is the lesson. Sections should reference (not duplicate) incident-specific metrics and fixes.

## Requirements trace

From the ticket's acceptance criteria:

1. **Doc written** at the specified path
2. **Checklist included** (verbatim from ticket body, or improved if patterns emerge)
3. **Both incidents cited** with concrete metrics (LLM call counts, dollar estimates, panic counts)
4. **No implementation work** — pure compound documentation
5. **Cross-reference from migration code** so future schema authors find the checklist

## Plan

### Step 1 — Write the compound doc

**File:** `docs/solutions/best-practices/schema-migrations-as-integration-events-2026-04-23.md`

**Frontmatter:**
```yaml
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
```

**Sections:**

1. **Problem** — Pattern statement: migration correctness ≠ downstream correctness. The `migrate_v25_to_v26` test validates the DDL. It does NOT validate that all code paths processing the now-larger pending set survive realistic production input distributions.

2. **Two concrete incidents** — Brief summaries with metrics, linking to the detailed incident docs:
   - **Incident 1: #757 cost spike** — 11 agents × 283 docs → 30,400 LLM calls → ~$40–60, Anthropic credit exhausted. Fix: budget guard + hash idempotency. Detail: see `first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`.
   - **Incident 2: KG UTF-8 panic** — v26 migration invalidated markers → full re-extraction → resolver scheduled ~5K subjects/agent → first multi-byte char (em-dash, byte 2000) panicked tokio-spawn'd resolver tasks → KG resolution fully broken. Site count: #767 body cites "5 resolver/extractor + 5 workspace-wide" (10 total scope); the resolver/extractor-internal site replacement was **14** per `utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md`, which also reports **27 panic events across 11 agents** in server.log. Fix: `safe_truncate` (`floor_char_boundary`) + CI byte-slice lint + spawn-site panic logging. Detail: see `utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md`.

3. **Why migration tests don't catch this** — The schema migration is correct in isolation. The failure is a second-order effect: changing what the pending-work query returns changes the *volume* and *distribution* of inputs to downstream consumers. Migration tests validate DDL + data transformation; they don't exercise the consumer pipeline at the new scale or against the new input distribution.

4. **Operational checklist** — From the ticket body (adapted/improved if warranted during writing):
   - Pending-set size change? → Estimate upper bound, run downstream pipeline against it
   - Processing markers invalidated? → Estimate re-processing cost, verify budget guards cap the spike
   - Realistic input testing? → Not simplified fixtures, actual production-shaped content at scale
   - Deploy plan includes "watch first restart"? → Cost spikes, panics, backlog growth

5. **Implementation hooks** — Options to enforce, without proposing specific implementation:
   - CI check that flags migrations touching tracking/marker tables
   - Plan-template question (first-boot cost) enforced by architect review
   - Staging-env integration test with production-scale pending set
   - Mandatory `Post-deploy verification` section in migration PR description template

6. **Related** — Links to:
   - mika#757, mika#767
   - `first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`
   - `utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md`
   - `kg-schema.rs` idempotency contract documentation
   - `check-byte-slices.sh` CI lint
   - **Implementer note for rescue PR + KG bug ticket:** #767 issue body's Related-section AC asks for "links to #757, the rescue PR, the KG bug ticket." The rescue/KG-bug PRs live in the `#752–#759` series. During implementation, run `gh issue view <N>` for each of `#752 #756 #758 #759` and identify which is the rescue PR (the one that introduced `safe_truncate`) and which is the KG bug ticket (the one that filed the panic). Cite both by number in the compound doc's Related section.

### Step 2 — Add cross-reference in migration code

**File:** `crates/mika-agent/src/db/kg_schema.rs`

Add a doc comment at the module level (or near the existing idempotency contract docs) pointing to the new checklist:

```rust
/// ## Schema migration pre-ship checklist
///
/// Before shipping any migration that changes what "pending work" queries return,
/// consult `docs/solutions/best-practices/schema-migrations-as-integration-events-2026-04-23.md`
/// for the operational checklist (mika#767).
```

This is a comment-only change — no behavior change.

### Step 3 — Verify

- Doc exists at the specified path with correct frontmatter
- Both incidents cited with concrete metrics
- Checklist present and complete
- Cross-reference added in `kg_schema.rs`
- No implementation code changed

## Files touched

| File | Change |
|------|--------|
| `docs/solutions/best-practices/schema-migrations-as-integration-events-2026-04-23.md` | New — compound doc |
| `crates/mika-agent/src/db/kg_schema.rs` | Comment-only — cross-reference to checklist |

## Risks

- **Overlap with existing docs:** Mitigated by making this the synthesis/pattern doc that references (not duplicates) the two incident docs.
- **Checklist goes stale:** Low risk — the checklist is a set of questions, not implementation-specific. Invalidated only if the project moves away from schema migrations entirely.

## Out of scope

- Implementing automated schema-migration integration tests (separate enhancement)
- Refactoring existing migration patterns
- Any code changes beyond the cross-reference comment
