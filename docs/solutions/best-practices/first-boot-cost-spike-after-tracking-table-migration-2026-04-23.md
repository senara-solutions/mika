---
title: "First-boot cost spike after a tracking-table migration"
module: kg
date: 2026-04-23
problem_type: best_practice
component: database
severity: high
applies_when:
  - "A schema migration creates a new tracking table that records 'seen before' / 'already processed' state"
  - "A non-interactive LLM or other expensive-per-unit pipeline reads from that table to decide what work to do"
  - "The consumer runs against already-populated source data on first boot after the migration"
  - "Per-agent or per-tenant fan-out multiplies the cost when the upstream source is shared"
tags:
  - knowledge-graph
  - schema-migration
  - idempotency
  - llm-budget
  - cost-control
  - first-boot
  - fan-out
---

## Context

On 2026-04-23 09:08 UTC, a routine `mika-server` restart burned ~30,400 Haiku LLM calls over 38 minutes (~\$40–60) against `claude-haiku-4-5`, exhausting Anthropic Console API credit. Tracked as mika#757.

The fault was structural, not a bug in any single line of code:

1. Schema v25 landed the Knowledge Graph (KG) tables including `kg_extractions` — a tracking table that records one row per successfully extracted doc to prevent re-extraction on subsequent boots.
2. The first boot after v25 saw an empty `kg_extractions` table — there were no prior "already extracted" rows because the table didn't exist before.
3. The pending-doc query (`docs with chunks but no extraction row`) therefore returned every doc across every agent.
4. 11 agents × 283 shared docs → ~3,113 extractions + per-entity resolution fan-out → ~30K LLM calls.
5. No per-batch cap existed to bound the cost.

**The general pattern:** any schema migration that introduces a tracking or "seen-before" table creates a first-boot cost spike the moment the consumer runs against populated source data. The spike scales with `source-units × consumer-multiplier × per-unit-cost`. If any of those factors is non-trivial — and they usually are for LLM-heavy consumers — the first boot after such a migration is a financial event waiting to happen.

This pattern generalizes well beyond KG. Think: a new `email_processed` table added to an already-populated mailbox. A new `vector_embedded` column added after embeddings already exist. A new `consent_recorded` flag added after every user has already implicitly opted in. Any time "we'll track it going forward" meets "we have a backlog" on first boot, the cost is an open question that must be answered at planning time, not discovered at runtime.

## Guidance

### Ask the first-boot question during planning

When a plan adds a tracking table for a consumer that does non-trivial per-unit work (LLM calls, external API calls, heavy compute), add this checklist item to the requirements trace:

> **Q: On the first boot after this migration, how many units of work does the pending query return, and what does that cost?**

Acceptable answers include:

- "Zero, because the consumer doesn't run at startup."
- "Bounded at N, because we cap the batch."
- "All of them, but each unit costs \$X and total is acceptable."

What is **not** an acceptable answer: "We don't know" or silence. If the answer is "we don't know," derive it — count source rows, model per-unit cost, multiply by consumer multipliers, and if the result is uncomfortable, build a safeguard before shipping.

**Multiplier awareness** (both `docs_root` fan-out and resolver fan-out hit this):

- Per-agent × per-tenant × per-customer → multiply every cost by the worst-case container count.
- Per-entity downstream consumers (like resolution after extraction) compound further — one extracted entity may produce one disambiguation call.
- Retry loops multiply again. A transient failure category that retries 3× with backoff scales by 3 on top of everything else.

### Ship tracking-table migrations with a structural budget cap

A migration that might trigger a first-boot spike should ship with a **caller-side structural budget** — not a prompt-level "be mindful" — capped on the consumer's batch. In Mika's case:

```rust
pub async fn extract_pending(&self, budget: u32) -> Result<BatchStats> {
    for doc in pending_docs {
        if stats.llm_calls >= budget {
            warn!(event = "kg_budget_exhausted", scope = "extraction", ...);
            stats.aborted_budget = true;
            break;
        }
        extract_document(doc).await?;
        stats.llm_calls += 1;
    }
}
```

Three properties matter:

1. **Structural, not prompted.** LLMs rationalize crossing prompt-level budgets ("this one is important"); a caller-side counter that refuses to issue a request past the cap cannot be rationalized around. See `feedback_prompt_enforcement_fragile.md`.
2. **Leaves remaining work pending.** A budget abort is not an error — it writes `aborted_budget = true` and logs `kg_budget_exhausted` with enough structured fields for operators to grep (scope, calls_made, budget, remaining). Remaining work drains over subsequent restarts.
3. **Exempts free paths from the debit.** Stage-1 exact matches in the KG resolver cost no LLM calls; debiting the budget for them would starve free work. The resolver **continues** past budget-skipped entities rather than breaking, so later free-path entities still make progress.

Default: pick a budget that is an order of magnitude below the worst-case steady-state volume. Mika defaults `MIKA_KG_BATCH_BUDGET` to 500 — high enough for healthy operation, low enough that a misconfiguration caps loss at roughly \$0.50 per restart at cheap-tier pricing instead of \$60.

### Pair hash-based idempotency with atomic marker writes

A tracking row is useless if the consumer can pay for the work and then fail to write the marker. On the next restart the same work repeats and the cost is paid again — unbounded if the failure is persistent.

**Two properties are required**:

1. **Content-aware idempotency.** Row-exists idempotency is fragile: if the source changes, the consumer still skips. Hash-based idempotency (`kg_extractions.source_doc_hash = kg_chunks.source_doc_hash`) compares the marker's recorded hash against the current source hash, so content drift triggers re-work and content-equivalence skips it. The pending query becomes a direct equality predicate:

   ```sql
   WHERE NOT EXISTS (
     SELECT 1 FROM kg_extractions e
     WHERE e.agent_id = c.agent_id
       AND e.source_doc_path = c.source_doc_path
       AND e.source_doc_hash = c.source_doc_hash
   )
   ```

   This only works if the upstream writes one identical hash across all chunk rows of a source doc (which the lexical ingestor does — confirmed at `crates/mika-agent/src/kg/lexical_ingestor.rs:260`). If the upstream ever changed to per-chunk hashes, a `GROUP BY ... HAVING` aggregation would be needed instead.

2. **Atomic marker write.** The marker must be written **inside the same transaction** as the extracted rows, not after them:

   ```rust
   tx.execute("INSERT ... kg_subject_entities ...", ...)?;
   tx.execute("INSERT ... kg_subject_relationships ...", ...)?;
   tx.execute("INSERT ... kg_extractions (..., source_doc_hash, ...) ...", ...)?;
   tx.commit()?;
   ```

   A crash, SIGKILL, disk-full, or FK-violation between the entity write and the marker write would otherwise leave "LLM paid but no marker" — the doc re-extracts on every restart, burning budget on the same item forever. Wrap them, or accept unbounded-on-failure cost.

### NULL-hash backfill is a one-shot: accept it and bound it

When the migration is `ALTER TABLE ... ADD COLUMN source_doc_hash TEXT` (nullable), every pre-existing row gets `NULL`. The pending query rejects `NULL = anything`, so each pre-existing row is "pending" once. The first post-deploy boot re-extracts them — **once** — and populates the hash. From the second boot onward the rows compare equal and stay skipped.

Two implications:

- **Document this explicitly.** Operators reading the logs on the first post-deploy boot will see activity and need to know it's the one-shot backfill, not a regression.
- **Bound it with the budget.** Don't assume "it's just a one-shot." If the backlog is large and the budget is tight, the one-shot becomes a multi-restart drain. That is still fine — it's deterministic and structural — but it needs to be named in the plan so operators don't panic-tune when they see `kg_budget_exhausted` WARN on the first few restarts.

### Provider choice is a structural cost lever — surface it in code

The same model family routed through different providers can be an order of magnitude apart in price. Anthropic direct vs. OpenRouter for `claude-haiku-4-5` is ~10× for bulk NER. An operator mis-setting `MIKA_KG_EXTRACTION_MODEL=anthropic/...` when `openrouter/anthropic/...` would work is a single-line choice that becomes a monthly bill difference.

Pattern: when the code resolves to an expensive provider for a high-volume internal path, emit a one-shot startup advisory naming both the current choice and the cheaper alternative:

```rust
if llm.provider_name().eq_ignore_ascii_case("anthropic") {
    warn!(
        event = "kg_anthropic_provider",
        scope = "extraction",
        model = %llm.model_name(),
        "KG extraction is using Anthropic — typically ~10× more expensive than \
         OpenRouter equivalents for bulk NER. Consider MIKA_KG_EXTRACTION_MODEL=openrouter/<model>."
    );
}
```

One-shot, not per-call — a log flood is worse than a silent cost. The advisory should fire once per role (extraction / resolution / ingestion) per startup.

Keep the check narrow and specific (`provider_name == "anthropic"`). A generic `kg_expensive_provider` event that tries to classify every provider's pricing becomes a maintenance tax — when OpenAI pricing changes, when a new provider is added, when OpenRouter's routing changes. Name the specific provider the specific incident involved, and add new providers case-by-case as new incidents surface.

### Fan-out on shared source data: document it or share the extraction

Per-tenant isolation is a deliberate schema decision — Mika's subject/chunk tables are agent-scoped so each agent has its own view of the subject graph. The cost consequence: when N agents point at the same `docs_root`, every doc is processed N times.

Two options, pick one at planning time and commit:

- **Option A (share extraction):** subject tables become per-docs_root with per-agent views. Larger redesign; schema-level.
- **Option B (document the cost):** per-agent tables stay, and the cost multiplier is explicitly documented. Add a startup INFO log when multiple agents share a `docs_root` so operators see the multiplier before it bites.

Mika picked Option B in the rescue fix because Option A is a schema redesign that doesn't belong in an incident response. The INFO log (`kg_shared_docs_root`) surfaces the fan-out at boot:

```
{"event":"kg_shared_docs_root","agents":["mika","mika-dev","mika-qa",...],"agent_count":11}
```

If operators running 10+ agents on one `docs_root` become common, that's the signal to revisit Option A.

### Migration immutability — historical migrations are frozen

An early version of this fix retroactively edited `migrate_v24_to_v25` to include the new column. This seems harmless (both paths converge to the same schema) but breaks two invariants:

1. **Audit trail.** `migrate_v24_to_v25` should record what actually shipped at v25, not what we later wish had shipped. Editing historical migrations makes them unreliable as a rollback/forensic reference.
2. **Test coverage.** The convergence test (fresh-install vs incremental-migrate) now skips the `ALTER TABLE` code path in `migrate_v25_to_v26`, because the retro-edited `migrate_v24_to_v25` already creates the column. The production v25→v26 migration path runs unexercised by tests.

Rule: once a migration has shipped, it is frozen. New changes go in new migrations. The fresh-install path (`migrate_v1` clean-slate) gets updated to include the latest schema, but per-version migrations stay as they were.

### AC interpretation discipline

The original AC for #757 said "skip any `kg_chunks` row that already has rows in `kg_chunk_subjects`." That wording assumed chunk-level extraction. The actual implementation is whole-doc extraction, making chunk-level markers structurally redundant — the same information lives on the parent doc.

The planner's job isn't to follow AC wording literally when the wording contradicts the code's shape. It's to call out the mismatch, propose an interpretation, and require explicit reviewer sign-off before `/ce:work` starts. Don't silently reinterpret; don't refuse to ship. Elevate the interpretation to the plan's Overview so approval is explicit.

## Why This Matters

Mika burned \$40–60 on one restart. The rescue fix caps the blast radius to ~\$0.50 per restart at equivalent pricing. But the deeper win is pattern recognition: a similar incident will happen the next time any tracking-table migration ships without someone asking the first-boot question.

The budget guard is a 50-line safeguard. The hash-idempotency plumbing is a 100-line addition. Each of those, shipped as part of the original #690 feature, would have prevented the incident entirely — and cost less than the incident response.

The expensive lesson is the **structural awareness**. The cheap lesson is the ~150 lines of code. Document the structural awareness first; the code pattern second.

## When to Apply

Invoke this pattern during planning for any feature that ships:

- A new tracking table (`*_processed`, `*_ingested`, `*_seen`, `*_logged`)
- A hash column added to an existing table for idempotency
- A consumer that runs non-trivial per-unit work at startup
- Per-tenant or per-agent fan-out that multiplies the work
- Any combination of LLM calls, external API calls, or heavy compute tied to a schema change

Skip the full pattern review for:

- Purely cosmetic migrations (renaming a column, adding a comment)
- Tracking tables that feed only interactive-request-path work (no startup scan)
- Consumers with already-bounded cost (<\$1/month worst case)

## Examples

### The failing shape (what #757 was before the fix)

```rust
// Pending-doc query, row-exists only
"SELECT DISTINCT source_doc_path FROM kg_chunks c
 WHERE c.agent_id = ?1
   AND NOT EXISTS (
     SELECT 1 FROM kg_extractions e
     WHERE e.agent_id = c.agent_id AND e.source_doc_path = c.source_doc_path
   )"

// Per-agent spawn, no budget
for agent in agents {
    tokio::spawn(async move {
        let extractor = SubjectExtractor::new(...);
        extractor.extract_pending().await  // loops through every pending doc
    });
}

// Non-atomic marker write
extract_document(doc).await?;         // LLM call #1
write_extraction_results(doc).await?; // DB write #1
record_extraction(doc).await;         // DB write #2 — separate transaction
```

Three amplifiers: row-exists idempotency invalidated by the v25 migration, no budget, non-atomic marker. The intersection produces the \$60 incident.

### The safe shape

```rust
// Pending-doc query, hash-aware
"SELECT DISTINCT c.source_doc_path FROM kg_chunks c
 WHERE c.agent_id = ?1
   AND NOT EXISTS (
     SELECT 1 FROM kg_extractions e
     WHERE e.agent_id = c.agent_id
       AND e.source_doc_path = c.source_doc_path
       AND e.source_doc_hash = c.source_doc_hash  // content-aware
   )"

// Per-agent spawn, budget-capped
let budget = settings.effective_kg_batch_budget();  // default 500
for agent in agents {
    tokio::spawn(async move {
        let extractor = SubjectExtractor::new(...);
        extractor.extract_pending(budget).await  // aborts at cap, leaves work pending
    });
}

// Atomic marker write — same transaction as entity rows
let tx = db.conn.unchecked_transaction()?;
tx.execute("INSERT ... kg_subject_entities ...", ...)?;
tx.execute("INSERT ... kg_subject_relationships ...", ...)?;
tx.execute("INSERT ... kg_extractions (..., source_doc_hash, ...) ...", ...)?;
tx.commit()?;
```

### Post-deploy verification signals

The rescue fix added four observable signals in `CLAUDE.md` so operators can confirm the fix is working after a restart:

- **Signal A** (extraction not re-running): `grep subject_extraction_start server.log | jq 'select(.pending_docs == 0)'` should list every agent by the second post-deploy restart.
- **Signal B** (budget not exhausted): `grep kg_budget_exhausted server.log` returns zero lines on a healthy restart.
- **Signal C** (resolver backlog drains over time): `SELECT agent_id, COUNT(*) FROM kg_subject_entities WHERE NOT EXISTS (...)` trends to 0 across restarts.
- **Signal D** (concrete cost prediction): with OpenRouter, expect ~\$0.05–\$0.50 per restart until the backlog drains.

Signal D is the one most often missed. Giving operators a concrete cost prediction lets them verify the fix in the first minute post-deploy, instead of waiting for a monthly bill to surface a regression.

## Related

- **Incident ticket:** [mika#757](https://github.com/senara-solutions/mika/issues/757)
- **Implementation plan:** `docs/plans/757-kg-extraction-idempotency-fanout.md`
- **Prior idempotency precedent:** `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md` (#689) — the hash-based idempotency pattern that #690 should have inherited but didn't.
- **Constrained NER lineage:** `docs/solutions/best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md` (#690) — the feature whose first-boot spike caused the incident.
- **KG conventions:** `docs/architecture/kg-implementation-conventions.md` — C2.5 (per-batch budget guard) added in this fix; pairs with pre-existing C2.1–C2.4.
- **Related memory entries:** `feedback_prompt_enforcement_fragile.md` (structural vs. prompt constraints), `feedback_transport_vs_workflow.md` (async decomposition over timeout tuning), `project_knowledge_graph.md` (KG milestone overview).

## Follow-ups (captured during review, deferred out of rescue scope)

These are real risks identified by the multi-reviewer `/ce:review` pass. They didn't block the rescue fix but warrant dedicated tickets:

- **Cross-agent global semaphore** — per-agent budget caps volume but not concurrent rate. 11 agents × `tokio::spawn` hammers one provider simultaneously.
- **N+1 on `get_doc_hash`** — the marker write now reads the chunk hash before the LLM call; partially mitigated by colocating both inside `extract_document`, but the query could be folded into `get_pending_docs`.
- **Resolver correlated-subquery indexing** — `get_pending_entities` does a per-entity `ORDER BY created_at DESC LIMIT 1` subquery with no `created_at` index.
- **Agent-native observability** — `get_config` reads only customer_config; agents can't see effective `kg_batch_budget` or correlate `kg_budget_exhausted` events (`tracing::warn!` only, not `audit_events`).
- **Graceful-shutdown `JoinHandle`s** — background spawns have no handles stored, so a SIGTERM mid-batch cancels at the next `.await` and may lose the in-flight doc's marker.
- **Invalid env-value fail-closed test** — `MIKA_KG_BATCH_BUDGET=-1` fails server startup (correct); no test locks this contract.
