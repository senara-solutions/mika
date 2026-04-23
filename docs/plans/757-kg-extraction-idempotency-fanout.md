---
title: "fix: KG extraction idempotency + startup budget guard + per-agent fan-out"
type: fix
status: active
date: 2026-04-23
origin: senara-solutions/mika#757
---

# fix: KG extraction idempotency + startup budget guard + per-agent fan-out

## Overview

On a normal server restart (2026-04-23 09:08 UTC), KG startup extraction + resolution fired ~30,400 LLM calls over 38 minutes against `claude-haiku-4-5`, exhausting Anthropic credit. Root cause is two-pronged: schema v25 landed the KG tracking tables, so the *first* restart after v25 saw every agent's doc set as "pending" (no `kg_extractions` rows); and 11 agents share the same `docs_root` (`.../mika/docs/solutions`), giving an 11× multiplier on every doc. The resolver then amplifies further because every newly-extracted entity drives a disambiguation call.

This plan adds three layered safeguards — structural idempotency hardening, a hard budget cap per batch, and documented / warned-about fan-out — plus safer provider defaults and a post-deploy verification signal. The budget guard is the primary defense: even if every prior mitigation fails, a single batch cannot exceed its configured cap without aborting and surfacing a loud WARN.

### AC #1 interpretation — **explicit approval needed**

AC #1 as written says *"skip any `kg_chunks` row that already has rows in `kg_chunk_subjects`."* That framing assumes **chunk-level** extraction. Current implementation is **whole-doc** extraction (per #690 D1 — one LLM call per doc with `[CHUNK N]` markers), so chunk-level idempotency markers are structurally redundant: the doc is the unit of work, not the chunk.

**This plan interprets AC #1 as "harden the doc-level `kg_extractions` marker with a content-hash check"** and documents the distinction explicitly in `crates/mika-agent/src/db/kg_schema.rs`. Moving to chunk-level extraction would be a larger refactor and is out of scope for this rescue fix. Reviewers must approve this interpretation before `/ce:work` starts — the ACs themselves are the contract, and quietly reinterpreting them is the drift class this milestone has been disciplined about avoiding elsewhere.

Explicit non-goal: per-chunk refactoring of the extractor. In scope: a schema v26 bump that adds `kg_extractions.source_doc_hash TEXT NULL` so idempotency can compare against the per-doc hash already stored on `kg_chunks` rows.

## Problem Frame

**Incident (2026-04-23 09:08 UTC):** `sudo rc-service mika-server restart` triggered 30,400 LLM calls in 38 min (~800/min) across 11 agents on `claude-haiku-4-5`. 1,044 `credit balance too low` 400s logged. Default `mika` chat responses silently degraded for ~3h until Vincent noticed. Estimated cost: $40–60. KG resolution state left half-drained (50–92% pending per agent).

**Structural issues:**

1. **Idempotency fragility.** Extraction has a doc-level marker (`kg_extractions`), but after schema v25 landed, every agent's entire doc set was "pending" on first boot — no prior tracking rows existed. Subsequent restarts are guarded, but there is no second-line defense if the marker is ever invalidated (schema migration, agent rename, docs_root change, etc.).
2. **No batch budget.** `extract_pending()` and `resolve_pending()` iterate until exhaustion with no maximum-call guard. A single misconfiguration silently burns an order of magnitude more than expected before anyone notices.
3. **Undocumented 1:N cost.** Subject/chunk/resolution tables are agent-scoped by design. When N agents share `docs_root`, the cost is N×. This is intentional at the schema level but was never surfaced in `kg_schema.rs` or operator docs, so it caught the operator by surprise.
4. **Expensive provider by default.** `MIKA_KG_INGESTION_MODEL=anthropic/claude-haiku-4-5` was in the live `.env`. There is no code-level signal that OpenRouter or another cheap provider is the recommended default for bulk NER.
5. **No post-deploy verification.** After this fix merges, there is no one-line signal Vincent can read to confirm that the next restart was safe.

**Constraints carried in from the prompt:**

- No live env edits (`~/.mika/.env` is operator-owned).
- No `make deploy`, no `rc-service`, no server restart during implementation.
- No unguarded live-provider integration tests (`#[ignore]` + env gate required).
- GitHub App webhooks are DISABLED for this rescue window; PR will not round-trip through mika-dev/mika-qa. Vincent merges manually.
- Schema migration **is** in scope: v25 → v26 adds one nullable column to `kg_extractions`. Reviewer pushback on "avoid the migration" was accepted — the migration is ~30 minutes of code, convergence-tested (forward harness from #686), and avoids both the anti-pattern of out-of-band schema drift and the cost of perpetual query-time aggregation on every startup.

## Requirements Trace

- **R1 (AC #1).** Extraction idempotency hardened at the **doc level** (see AC interpretation in Overview): schema v26 adds `kg_extractions.source_doc_hash TEXT NULL`; pending query compares against the per-doc hash already stored on `kg_chunks` rows (which the lexical ingestor writes identically across all chunks of a single doc — verified in `lexical_ingestor.rs:318`). Idempotency key + AC interpretation documented in `crates/mika-agent/src/db/kg_schema.rs`.
- **R2 (AC #2).** Startup budget guard: configurable cap (`MIKA_KG_BATCH_BUDGET`, default 500) on LLM calls per extraction batch and per resolution batch. Overflow logs WARN with exact count + remaining work and aborts the batch cleanly (pending work stays for next run).
- **R3 (AC #3).** Per-agent fan-out decision: option (b) — **document** the 1:N cost in `kg_schema.rs` with operator guidance, and emit a startup INFO log when multiple agents share a `docs_root` so the cost is visible.
- **R4 (AC #4).** Default provider safety: `.env.example` recommends an OpenRouter model for KG, `CLAUDE.md` KG section updated, and a startup WARN fires when extraction or resolution resolves to an `anthropic/*` model (non-fatal advisory).
- **R5 (AC #5).** Resolver drain validation: plan doc + PR body describe the exact observable signals (log events, DB query) Vincent can read to confirm a post-deploy restart is safe.

## Scope Boundaries

- **In scope:** Code changes in `crates/mika-agent/src/kg/`, `crates/mika-agent/src/db.rs` (schema v26), `crates/mika-agent/src/db/kg_schema.rs`, `crates/mika-common/src/config.rs`, `crates/mika-agent/src/server/mod.rs`; documentation in `kg_schema.rs` doc comments, `CLAUDE.md`, `.env.example`; unit + migration tests using `#[cfg(test)]` with `MockLlmProvider`.
- **Out of scope:** Chunk-level extraction refactor (whole-doc extraction stays); live-provider integration tests; rebalancing or backfilling the half-drained resolver state (Vincent handles post-deploy via a controlled restart); changing the Anthropic KG settings in `~/.mika/.env` (operator action, not code); schema changes beyond the one additive-nullable column on `kg_extractions`.

### Deferred to Separate Tasks

- **Chunk-level extraction.** Moving `extract_document` from whole-doc to per-chunk LLM calls would be a larger rewrite and is not required to meet AC #1 (see interpretation in Overview). Track as a follow-up if the observed cost profile warrants it.
- **Shared-extraction short-circuit across agents.** Option (a) from AC #3 is a schema-touching redesign (subject/chunk tables are agent-scoped); opt for option (b) here and capture the redesign as a separate milestone item if operators want to run 10+ agents off one docs_root.
- **Resolver drain rebalancing.** The 50–92% backlog per agent will drain on the next safe restarts with a cheap provider under budget; no code change needed here. May take multiple restart cycles for agents starting >3,000 pending — see Signal D in Unit 5.
- **Milestone retrospective note (reviewer observation).** "First-boot cost spike after a tracking-table migration" is a pattern worth naming for future milestone planning reviews. This plan flags it here so the retrospective for milestone #14 + this rescue can capture it; actually writing the retrospective doc belongs to the retrospective pass, not to this fix PR.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/kg/subject_extractor.rs` — `extract_pending()` (line 507), `extract_document()` (line 404), `get_pending_docs()` (line 1090), `record_extraction()` (line 1115). Extraction is doc-level: one LLM call per doc with `[CHUNK N]` markers, then a single transaction writes entities/relationships/provenance.
- `crates/mika-agent/src/kg/entity_resolver.rs` — `resolve_pending()` (line 199). Per-entity resolution via two-stage pipeline (exact match → LLM disambiguation).
- `crates/mika-agent/src/kg/ingestion_orchestrator.rs` — compound hook (sync inline) and re-extraction on doc change. Does not need to honor the batch budget in the same way — single-doc path, bounded by construction.
- `crates/mika-agent/src/server/mod.rs` — startup dispatch at lines 806–916 spawns per-agent extraction and resolution tasks after lexical ingestion. This is where fan-out multiplies: one `tokio::spawn` per agent × N agents.
- `crates/mika-common/src/config.rs` — `kg_ingestion_model`, `kg_extraction_model`, `kg_resolution_model` fields (lines 754–765); `make_kg_extraction_provider` / `make_kg_resolution_provider` factories (lines 1014–1039). All currently default to `None` (feature disabled when unset), so "safer default" means operator guidance + startup advisory, not a hardcoded fallback.
- `crates/mika-agent/src/db/kg_schema.rs` — central place for KG schema doc comments + column lists. The idempotency contract doc belongs here.
- `crates/mika-agent/src/async_db.rs` + `crates/mika-agent/src/db.rs` — `AsyncDatabase` pattern for DB queries.
- `.env.example` — referenced by `CLAUDE.md` as the source of truth for operator-facing env config.

### Institutional Learnings

- `docs/solutions/kg/` (if present) — check for KG-specific lessons from #689–#692.
- Memory `feedback_transport_vs_workflow.md` — "Transport config shouldn't compensate for workflow concerns; prefer async decomposition." The batch budget is a workflow concern (per-batch ceiling), not a transport concern (per-request timeout). Aligns with that guidance.
- Memory `feedback_prompt_enforcement_fragile.md` — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints." The budget guard is structural (caller-side counter + early return), not prompted — correct pattern.
- Memory `project_knowledge_graph.md` — milestone #14 layer overview; schema v25 landed the KG tracking tables.

### External References

None required — the fix is entirely within the existing KG module boundaries and uses established Rust/SQLite patterns already in the repo.

## Key Technical Decisions

- **Doc-level idempotency with content-hash verification (AC #1).** Schema v26 adds `kg_extractions.source_doc_hash TEXT NULL`. The pending query compares the stored hash against `kg_chunks.source_doc_hash` directly — no aggregation, no concatenation — because the lexical ingestor already stores one identical per-doc hash across all of a doc's chunk rows (confirmed by inspection of `lexical_ingestor.rs:260` which uses `SELECT DISTINCT source_doc_hash` and treats `len == 1` as "unchanged," and `lexical_ingestor.rs:318` which inserts the same `new_hash_owned` for every chunk). Rationale for v26 over out-of-band ALTER or query-time aggregation: (a) repo convention is versioned migrations with forward-test convergence (`migrate_v24_to_v25` pattern, convergence test at `db.rs:10992`), (b) one-shot additive nullable column is exactly the class of migration that pattern exists for, (c) query-time aggregation would pay SHA over N chunks per doc per startup forever.
- **Budget guard is per-batch, not per-agent-process, and not shared across extraction/resolution.** Each call to `extract_pending()` or `resolve_pending()` receives its own budget (default 500). *Rationale:* simplest structural cap, composable, no shared counter between parallel per-agent tasks. *Tradeoff named honestly:* worst-case **per-startup cap** is `2 × N × budget` (extraction batch **plus** resolution batch). With N=11 agents and budget=500, that is up to ~11,000 LLM calls per startup — still roughly three × lower than the 30,400 incident, and in practice far lower once idempotency skips extraction entirely on the second restart. A shared pool would cap the total more tightly but would couple the two phases and is deferred unless the observed cost justifies it.
- **Option (b) for AC #3 — document the fan-out.** Per-agent subject tables are agent-scoped by schema design; moving to shared extraction would require a substantial redesign (subject tables → cross-agent with per-agent views) and is out of scope for a rescue fix. *Rationale:* ship-before-next-restart mandate. Add a startup INFO log listing agents that share `docs_root` so operators see the multiplier. Document the 1:N cost in `kg_schema.rs` with guidance on when to share vs. separate agents.
- **Safer provider default = guidance, not hardcoded default.** The code already defaults to `None` (disabled). The fix is (i) `.env.example` now shows an OpenRouter model, (ii) `CLAUDE.md` KG section recommends OpenRouter with cost rationale, (iii) a one-line startup WARN fires when extraction or resolution resolves to `anthropic/*`. *Rationale:* we cannot change the live operator `.env` per constraint; this is the safest in-code change set.
- **Budget config lives alongside KG model config in `Settings`.** New field `kg_batch_budget: Option<u32>` with env key `MIKA_KG_BATCH_BUDGET`. Default when unset: `500`. Shared by extraction and resolution to keep the mental model simple.
- **Tests use `MockLlmProvider` only.** No live-provider tests added. Integration-style tests stay under `#[cfg(test)]` in-module.

## Open Questions

### Resolved During Planning

- *Should the budget guard be global (shared across all per-agent tasks) or per-batch?* **Per-batch.** Simpler, composable, and 11 × 500 = 5,500 is still safe by two orders of magnitude compared to the incident.
- *Should we attempt to drain the current 50–92% resolver backlog in this PR?* **No.** Vincent will do a controlled restart with a cheap provider after merge. The fix is about preventing recurrence, not retroactively fixing state.
- *Should we hard-default the KG model to openrouter if unset?* **No.** `None` means disabled, which is the safest default. A hardcoded fallback could silently turn on LLM calls for operators who deliberately left it off.
- *Is the AC #1 literal "skip chunks with `kg_chunk_subjects`" incompatible with the existing whole-doc extractor?* **Yes — interpreted as doc-level idempotency hardening.** Elevated to Overview; documented in `kg_schema.rs`.
- *Path A (v26 migration) vs Path B (query-time aggregation)?* **Path A.** Accepted reviewer pushback — leaving the choice to the implementer was the worst option. Schema bump is additive-nullable, convergence-tested, ~30 minutes.
- *Should the anthropic warning be a general "expensive provider" warning or anthropic-specific?* **Anthropic-specific.** Event is `kg_anthropic_provider`; message explicitly names Anthropic. Future incidents can add other providers case by case; keeping the list general requires ongoing maintenance for no current benefit.
- *Can the hash check query be simplified (reviewer's hypothesis)?* **Yes.** `kg_chunks.source_doc_hash` is per-doc, not per-chunk. Query becomes a direct `ke.source_doc_hash = kc.source_doc_hash` comparison.

### Deferred to Implementation

- Exact field name and default-constant location for the budget config — choose during implementation to match existing `config.rs` conventions.
- Whether the WARN on `anthropic/*` should be info-level — tune during implementation to avoid noise in normal dev workflows.
- Whether to include the hash in the `kg_extractions` row at migration time or populate it lazily on next successful extraction. Lazy is simpler; verify no regression.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```text
Startup dispatch (server/mod.rs):
    group agents by docs_root; info!(kg_shared_docs_root) when group size >= 2
    warn once per role if resolved provider is anthropic (kg_anthropic_provider)
    for each agent:
        tokio::spawn(extract_pending(agent, budget=N))
    for each agent:
        tokio::spawn(resolve_pending(agent, budget=N))   // N = effective_kg_batch_budget()

extract_pending(budget):
    // Pending query (simplified — hash is per-doc in kg_chunks):
    //   SELECT DISTINCT c.source_doc_path FROM kg_chunks c
    //   WHERE c.agent_id = ?1
    //     AND NOT EXISTS (
    //       SELECT 1 FROM kg_extractions e
    //       WHERE e.agent_id=c.agent_id AND e.source_doc_path=c.source_doc_path
    //         AND e.source_doc_hash = c.source_doc_hash
    //     )
    pending_docs = query(above)
    calls_made = 0
    for doc in pending_docs:
        if calls_made >= budget:
            warn!(event="kg_budget_exhausted", scope="extraction",
                  calls_made, remaining=pending_docs.len()-i, budget)
            return Ok(BatchStats { aborted: true, ... })
        extract_document(doc)     // one LLM call per doc
        calls_made += 1
        record_extraction(doc, source_doc_hash=hash_from_chunks)

resolve_pending(budget):
    pending_entities = query: subject entities missing or stale resolution log row
    llm_calls = 0
    for entity in pending_entities:
        if exact_match_hits(entity):
            resolve via exact     // no LLM call, no budget debit
            continue
        if llm_calls >= budget:
            warn!(event="kg_budget_exhausted", scope="resolution",
                  llm_calls, remaining=..., budget)
            return Ok(ResolutionStats { aborted: true, ... })
        disambiguate_via_llm(entity)
        llm_calls += 1
```

Per-startup cap (worst case, honest number):

```text
    extraction_cap  = N_agents × budget           (e.g., 11 × 500 = 5,500)
    resolution_cap  = N_agents × budget           (e.g., 11 × 500 = 5,500)
    total_per_boot  = extraction_cap + resolution_cap = 2 × N × budget
                    = 11,000 calls per startup under worst-case fan-out

    Expected steady state (after first successful boot):
    extraction_calls = 0   (idempotency skips everything)
    resolution_calls ≈ (new subject entities drained this boot), bounded by budget
```

Fan-out advisory (server/mod.rs startup):

```text
After collecting `agents` map, group by docs_root:
    docs_root_groups = HashMap<PathBuf, Vec<agent_id>>
    for group with len > 1:
        info!(event="kg_shared_docs_root",
              docs_root, agents=group,
              note="each agent extracts independently; cost scales N×")
```

Anthropic provider advisory (one-shot, per role, anthropic-specific event):

```text
After make_kg_extraction_provider() returns Some(Ok(llm)):
    if llm.provider_name() == "anthropic":
        warn!(event="kg_anthropic_provider",
              role="extraction", provider="anthropic",
              note="Anthropic pricing is ~10× typical OpenRouter equivalents \
                    for these models; consider MIKA_KG_EXTRACTION_MODEL=openrouter/...")
// Same one-shot pattern for the resolution provider with role="resolution".
```

## Implementation Units

- [ ] **Unit 1: Add budget config + startup logging for shared docs_root / expensive provider**

**Goal:** Wire the new `kg_batch_budget` config field, surface fan-out cost at startup, and warn when KG resolves to an expensive provider. All user-visible observability for R2/R3/R4 lands here first so the later units have structured log events to reference.

**Requirements:** R2, R3, R4.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-common/src/config.rs` (add `kg_batch_budget: Option<u32>` field, default 500, documented alongside the other KG fields; expose via `effective_kg_batch_budget(&self) -> u32`).
- Modify: `crates/mika-agent/src/server/mod.rs` (after `agents` collection and before per-agent extraction/resolution spawns: (a) group agents by `docs_root` and emit `kg_shared_docs_root` INFO when a group has ≥ 2 agents, (b) after each `make_kg_*_provider` call, check resolved provider and emit `kg_anthropic_provider` WARN — at most once per role per startup — when it resolves to an `anthropic/*` model).
- Modify: `.env.example` — add `MIKA_KG_BATCH_BUDGET=500` with a comment; change commented example `MIKA_KG_*_MODEL` lines to reference an `openrouter/*` model rather than `anthropic/*`.
- Test: `crates/mika-common/src/config.rs` (in-module `#[cfg(test)] mod tests`) — round-trip test for the new field default + env override.

**Approach:**
- Follow the existing `kg_ingestion_model` / `kg_extraction_model` pattern (serde default, `#[serde(default)]`, `Option<u32>`).
- Provide `effective_kg_batch_budget()` helper returning `self.kg_batch_budget.unwrap_or(500)` so the callsite is explicit about the default.
- For the shared-docs_root detection, docs_root is currently computed once per server (`std::env::current_dir().join("docs/solutions")`) — so the shared case is "all agents with extraction enabled share the same root." Emit once at INFO with the full agent list only when the group size is ≥ 2. Single-agent case stays silent.
- For the anthropic-specific warning, use the `provider_name()` method on the resolved `LlmProvider`. Event name is `kg_anthropic_provider` (not a generic `kg_expensive_provider`). Message explicitly names Anthropic and points at the env var to change (`MIKA_KG_EXTRACTION_MODEL=openrouter/...`). Do NOT widen the check to other providers in this PR — that's a separate call when and if new incidents surface.
- Do NOT add runtime providers or fallbacks — provider selection stays as-is.

**Patterns to follow:**
- `crates/mika-common/src/config.rs` field declarations + `Default::default()` overrides.
- `crates/mika-agent/src/server/mod.rs` existing KG dispatch logging style (`event="..."`, `agent_id = %...`).

**Test scenarios:**
- Happy path: `Settings::default()` produces `kg_batch_budget = None` and `effective_budget()` returns 500.
- Edge case: `MIKA_KG_BATCH_BUDGET=100` in env overrides to 100.
- Edge case: `MIKA_KG_BATCH_BUDGET=0` is accepted and treated as "disable entirely" by the extractor/resolver (no calls made, immediate return — verified in Units 2 and 3).
- Error path: `MIKA_KG_BATCH_BUDGET=-1` or non-numeric value fails to deserialize — captured as a config-load error, server logs it.

**Verification:**
- `cargo test -p mika-common config::` passes.
- Manual: starting mika-server with multiple agents configured + `MIKA_KG_INGESTION_MODEL=openrouter/...` emits exactly one `kg_shared_docs_root` INFO with the full agent list; no `kg_anthropic_provider` WARN.
- Manual: swapping to `anthropic/*` emits one `kg_anthropic_provider` WARN per role (extraction + resolution).

- [ ] **Unit 2: Schema v26 migration + budget-guarded `extract_pending` with doc-hash idempotency marker**

**Goal:** Schema v26 adds `kg_extractions.source_doc_hash TEXT NULL`. `extract_pending()` honors the budget cap and its pending query does a direct per-doc hash comparison against `kg_chunks.source_doc_hash`, so stale markers only re-trigger extraction when chunk content actually changed.

**Requirements:** R1, R2.

**Dependencies:** Unit 1 (needs `kg_batch_budget` config and `effective_kg_batch_budget()`).

**Files:**
- Modify: `crates/mika-agent/src/db.rs` — bump `CURRENT_SCHEMA_VERSION` to 26; add `migrate_v25_to_v26()` following the established pattern (see `migrate_v24_to_v25` at line 2783). Migration is a single `ALTER TABLE kg_extractions ADD COLUMN source_doc_hash TEXT` + `INSERT INTO schema_version (version) VALUES (26)`. Additive-nullable, fully idempotent on re-run (SQLite's `ALTER` is not, so guard with `pragma_table_info` check or rely on the version gate at line 622 which already prevents re-run). Update the `fresh schema` branch (line 877 area) to create the column on v26 installs. Extend the v24→v25 convergence test (`db.rs:10992`) to also assert v26 convergence from a fresh v26 install vs. a migrated v25→v26 install.
- Modify: `crates/mika-agent/src/db/kg_schema.rs` — add `source_doc_hash` to `KG_EXTRACTION_COLUMNS`. Extend the module doc with a new **Idempotency key (extraction)** section documenting: (a) primary key is `(agent_id, source_doc_path)` in `kg_extractions`; (b) staleness triggers re-extraction when `kg_extractions.source_doc_hash != kg_chunks.source_doc_hash` for any of the doc's chunk rows (the hash is per-doc, so any chunk row for the doc has the same value — see `lexical_ingestor.rs:260,318`); (c) per-chunk `kg_chunk_subjects` rows are provenance, not idempotency markers — AC #1's literal "skip chunks with `kg_chunk_subjects`" is interpreted as doc-level marker hardening given whole-doc extraction.
- Modify: `crates/mika-agent/src/kg/subject_extractor.rs` — `extract_pending(budget: u32)`; inner loop tracks `calls_made`, emits `kg_budget_exhausted` WARN and returns early with `BatchStats { aborted: true, ... }` when reached. `get_pending_docs()` rewritten (see Approach). `record_extraction()` persists the `source_doc_hash` it just consumed by reading one chunk row per doc before extraction (the hash is already in memory from the ingestor; we just need it back here).
- Modify: `crates/mika-agent/src/server/mod.rs` — spawn site passes `settings.effective_kg_batch_budget()` into the constructor call chain.
- Test: `crates/mika-agent/src/kg/subject_extractor.rs` `#[cfg(test)] mod tests` — scenarios below, using `MockLlmProvider`.
- Test: `crates/mika-agent/src/db.rs` — convergence test covers v26.

**Approach:**
- **Migration.** Single nullable column, no backfill. Pre-existing `kg_extractions` rows from the first-run-after-v25 boot will have `source_doc_hash IS NULL`. The pending query treats NULL as stale (see SQL below), which correctly forces one re-extraction of those rows on the next boot — but only if chunks still exist and only within budget, so total cost is bounded. On the second post-deploy boot, all rows have a hash and the query returns empty.
- **Pending query (simplified per reviewer).** The lexical ingestor stores one identical hash across all chunk rows of a doc (`lexical_ingestor.rs:318` writes `new_hash_owned` on every chunk). So a direct comparison suffices — no aggregation, no concatenation:
  ```sql
  SELECT DISTINCT c.source_doc_path
  FROM kg_chunks c
  WHERE c.agent_id = ?1
    AND NOT EXISTS (
      SELECT 1 FROM kg_extractions e
      WHERE e.agent_id      = c.agent_id
        AND e.source_doc_path = c.source_doc_path
        AND e.source_doc_hash = c.source_doc_hash
    )
  ORDER BY c.source_doc_path
  ```
  `source_doc_hash IS NULL` on an old row fails the equality predicate, matching the NOT EXISTS → the doc is pending, as desired. On the next boot after a successful run, the row has a non-NULL hash matching current chunks → NOT EXISTS is false → doc is not pending.
- **Budget.** `extract_pending(budget: u32)` — `budget == 0` returns immediately with zero calls. For every doc, check `calls_made >= budget` before `extract_document`; on trigger, emit `warn!(event="kg_budget_exhausted", scope="extraction", calls_made, remaining=pending.len()-i, budget)` and return `Ok(BatchStats { aborted: true, ..stats })`.
- **Hash provenance on write.** Before extracting, read one `kg_chunks.source_doc_hash` for the doc; on success, pass that hash to `record_extraction()` which writes it into `kg_extractions.source_doc_hash` via the existing ON CONFLICT UPSERT.
- **Mock LLM** from `mika-common::llm::mock` (already `test-utils` gated and used in eval tests).

**Patterns to follow:**
- `migrate_v24_to_v25` at `crates/mika-agent/src/db.rs:2783` for migration shape.
- Convergence test at `crates/mika-agent/src/db.rs:10992` for fresh-vs-migrated parity.
- Existing `for (i, doc_path) in pending_docs.iter().enumerate()` loop at `subject_extractor.rs:534`.
- Existing `warn!` event style with `trace_id`, `agent_id`, `event=...`.

**Test scenarios:**
- Happy path: 3 pending docs (no `kg_extractions` rows), budget=10, mock LLM returns valid output → all 3 extracted, `calls_made=3`, `aborted=false`, 3 rows in `kg_extractions` each with a non-NULL `source_doc_hash` matching the corresponding `kg_chunks.source_doc_hash`.
- Edge case: 3 pending docs, budget=2 → first 2 extracted, `aborted=true`, third doc has no `kg_extractions` row.
- Edge case: `budget=0` → immediate return with `calls_made=0`, no LLM calls issued (`MockLlmProvider` verifies).
- Integration (primary idempotency check): doc extracted once → `kg_extractions.source_doc_hash` populated → second `extract_pending` run with identical chunks sees **zero** pending → no LLM calls.
- Integration (hash drift): doc extracted → chunks mutated (simulate re-ingestion writing a new hash on all chunk rows) → next `extract_pending` sees the doc pending again → one LLM call → `kg_extractions` updated to new hash.
- Integration (NULL-hash reprocessing): pre-existing `kg_extractions` row with `source_doc_hash IS NULL` (simulating the post-v26-migration state) → first `extract_pending` treats doc as pending, re-extracts, writes non-NULL hash → second run is a no-op.
- Error path: LLM returns malformed JSON after retries → C2.3 log-and-skip; budget is consumed for the attempt; remaining docs continue within the remaining budget.
- Migration: v25 DB → run v25→v26 migration → `kg_extractions.source_doc_hash` column exists and is NULL for all pre-existing rows. Fresh v26 install + v25-then-migrated install converge schema.

**Verification:**
- `cargo test -p mika-agent --lib kg::subject_extractor` passes.
- `cargo test -p mika-agent --lib db::` passes (includes migration + convergence).
- `cargo clippy -p mika-agent` clean.
- `kg_schema.rs` doc section covers idempotency key, hash-equality check, AC #1 interpretation, and non-idempotency role of `kg_chunk_subjects`.

- [ ] **Unit 3: Budget-guarded `resolve_pending`**

**Goal:** `resolve_pending()` honors the same budget cap. LLM-disambiguation calls count against the budget; exact-match resolutions do not (zero-cost). On exhaustion, remaining entities stay pending for the next run.

**Requirements:** R2.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` — `resolve_pending(budget: u32)`; count LLM calls made in Stage 2 (disambiguation) against the budget; Stage 1 exact-match resolutions are free. Emit `kg_budget_exhausted` WARN with `scope = "resolution"`, return cleanly.
- Modify: `crates/mika-agent/src/server/mod.rs` — pass budget into the resolver spawn at line 884.
- Modify: `crates/mika-agent/src/kg/ingestion_orchestrator.rs` — `spawn_resolution()` also honors the budget (pass from `IngestionOrchestrator` constructor — thread through a `budget: u32` field).
- Test: `crates/mika-agent/src/kg/entity_resolver.rs` `#[cfg(test)] mod tests` — add scenarios below.

**Approach:**
- Thread `budget: u32` through the resolver constructor and `resolve_pending`. Keep the existing mode-split (exact-match-only when no LLM) — `budget = 0` in exact-match-only mode is a no-op as before.
- Emit budget-exhaustion WARN with structured fields matching Unit 2 for easy log correlation.
- Ingestion orchestrator also needs the budget; default it through `Settings::effective_kg_batch_budget()` at the call site in `server/mod.rs`.

**Patterns to follow:**
- Existing resolver two-stage dispatch.
- Same log event shape as Unit 2.

**Test scenarios:**
- Happy path: 5 pending entities, 3 exact-match, 2 LLM-disambiguated, budget = 10 → all resolved, `llm_calls = 2`, not aborted.
- Edge case: 5 pending, all require LLM disambiguation, budget = 2 → first 2 resolved, remaining 3 stay pending (`kg_resolutions_log` has 2 rows, not 5).
- Edge case: exact-match-only mode (`resolution_llm = None`) with budget = 0 → no-op; exact matches still happen for Stage 1 entities.
- Edge case: 5 pending entities all exact-match with budget = 0 → all resolved via exact match, no LLM calls, no budget debit.
- Error path: LLM disambiguation fails mid-batch → C2.3 log-and-skip; budget consumed for the failed attempt; remaining entities continue under remaining budget.

**Verification:**
- `cargo test -p mika-agent --lib kg::entity_resolver` passes.
- `kg_resolutions_log` shows the cut-off cleanly (`outcome = 'matched_exact' | 'matched_llm' | 'skipped_no_llm' | 'no_match'`).

- [ ] **Unit 4: Documentation + operator guidance (AC #3, AC #4)**

**Goal:** Operators understand the per-agent fan-out cost, the recommended provider, and the new budget knob without reading the source.

**Requirements:** R3, R4.

**Dependencies:** Units 1–3 (references their log event names and new env var).

**Files:**
- Modify: `crates/mika-agent/src/db/kg_schema.rs` — extend the top-of-file module doc with a new **Fan-out cost model** section: documents the per-agent isolation decision, the N× multiplier when `docs_root` is shared, guidance on when to share vs. separate agents (examples: research agents that should diverge = separate; workspace tooling agents that should see the same library = shared with understood cost), and a forward pointer to `MIKA_KG_BATCH_BUDGET` as the safety net.
- Modify: `CLAUDE.md` (project root) — KG section update: recommend OpenRouter for `MIKA_KG_*_MODEL`, mention `MIKA_KG_BATCH_BUDGET` default + override, reference `kg_shared_docs_root` and `kg_budget_exhausted` log events for observability.
- Modify: `crates/mika-agent/CLAUDE.md` — subject-extractor / entity-resolver subsections: note budget guard, idempotency hash check, `kg_extractions` marker semantics, and pointer to `kg_schema.rs` for the full contract.
- Modify: `.env.example` — replace any commented `MIKA_KG_*_MODEL=anthropic/...` examples with `openrouter/...` examples; add `MIKA_KG_BATCH_BUDGET=500` with a one-line comment on intent.

**Approach:**
- Keep `CLAUDE.md` edits concise — one paragraph per change, not full specs.
- `kg_schema.rs` is the canonical place; other docs forward-point to it.
- The fan-out section should include an example cost calc so operators can predict the bill (e.g., "11 agents × 283 docs × 1 LLM call ≈ 3,113 calls per full extraction").

**Test scenarios:**
- Test expectation: none — documentation-only unit. Covered indirectly by the `/mika-doc-audit` pipeline step, which verifies docs match code behavior.

**Verification:**
- `/mika-doc-audit` passes (no stale KG claims).
- `rg 'anthropic/claude' .env.example` returns zero hits outside of "was" / historical mentions.
- `kg_schema.rs` top-of-file doc visibly covers: write contract (existing), idempotency key (Unit 2), fan-out cost model (this unit).

- [ ] **Unit 5: PR-body verification signal (AC #5)**

**Goal:** The PR body for this change explicitly tells Vincent the three observable signals that prove the fix works, so post-deploy verification is a 30-second check, not an investigation.

**Requirements:** R5.

**Dependencies:** Units 1–4 merged in PR body references.

**Files:**
- Create: (none — signal description is in the PR body and plan, not a committed file).
- Modify: `crates/mika-agent/CLAUDE.md` Knowledge Graph section — a short **Post-restart safety check** subsection that operators can re-read later, describing the expected log events and a SQL query.

**Approach:**
The PR body must include a **Post-deploy verification** section with four signals. Framing is deliberately tight — reviewer flagged that Signal A alone could be misread as "fully caught up" while Signal C still shows backlog.

1. **Log signal A — extraction not re-running (narrowly scoped):** On the **second** post-deploy restart (the first one backfills NULL hashes per the v26 migration note), `event="subject_extraction_start"` should show `pending_docs=0` for every agent that previously had `kg_extractions` rows. Does **not** prove resolver is caught up — that's Signal C. `grep 'subject_extraction_start' server.log | jq -c 'select(.pending_docs == 0) | {agent_id}'` should list every agent.
2. **Log signal B — budget not exhausted:** `grep 'kg_budget_exhausted' server.log` returns zero lines on a healthy second+ restart. A hit indicates either the budget is too tight, an unexpected re-extraction cascade, or the resolver needs more budget windows to catch up (in which case it'll drain over a few restarts — see Signal D).
3. **SQL signal C — resolver backlog drains over time:** `SELECT agent_id, COUNT(*) FROM kg_subject_entities se WHERE NOT EXISTS (SELECT 1 FROM kg_resolutions_log rl WHERE rl.agent_id = se.agent_id AND rl.subject_entity_id = se.id) GROUP BY agent_id;` — count trends toward 0 across restarts (bounded by the per-restart budget). May take multiple restart cycles to reach 0 for agents starting at >3,000 pending. **This is the resolver-caught-up signal; Signal A does not imply it.**
4. **Cost signal D — concrete prediction for first post-fix restart:** With `openrouter/...` configured and the current 50–92% resolver backlog, expect roughly `N_agents × budget = ~5,500` LLM calls for resolution on the first restart, plus ~0 for extraction (idempotency skips once hashes are backfilled). At typical OpenRouter cheap-tier pricing (~\$0.0001 per call average), total cost ≈ **\$0.05–\$0.50 per restart** until the backlog drains. If the first post-fix restart shows substantially more than `N × budget` calls or substantially more than \$1 of spend, something is wrong — budget guard failure, stale idempotency, or provider routing regression.

The plan captures these signals; the PR body restates them concretely with copy-pasteable commands.

**Test scenarios:**
- Test expectation: none — verification signal is a documentation artifact. Its correctness is verified manually by Vincent post-deploy.

**Verification:**
- PR body contains all three signals with concrete commands/queries.
- `CLAUDE.md` KG section has the matching **Post-restart safety check** subsection.

## System-Wide Impact

- **Interaction graph:** Startup spawn sites in `crates/mika-agent/src/server/mod.rs` are the only callers affected. Compound hook in `ingestion_orchestrator.rs` is per-doc and naturally bounded; threading budget through is defensive, not load-bearing.
- **Error propagation:** Budget exhaustion is not an error — it returns `Ok(BatchStats { aborted: true, ... })` with a WARN log. Callers in `server/mod.rs` already log the returned stats; no behavior change downstream.
- **State lifecycle risks:** A budget abort partway through extraction leaves some docs marked `kg_extractions` and others pending. This is correct behavior — the next run picks up where it left off. No partial-row state (extraction writes are transactional per-doc already).
- **API surface parity:** `ingest_and_extract` and `reingest_and_reextract` in `IngestionOrchestrator` are compound-hook paths that extract one doc at a time — threading `budget` through them is a small refactor. For correctness, these paths should either honor the budget or explicitly skip the check because they're bounded-by-construction; pick "skip with comment" to avoid accidental compound-hook failures.
- **Integration coverage:** Startup-triggered extraction + resolution is exercised by the server's init path. Unit tests cover extract/resolve in isolation using `MockLlmProvider`. A full end-to-end startup test would require instantiating the server with multiple agents — deliberately out of scope; Vincent's post-deploy verification signal covers the integration side.
- **Unchanged invariants:** Sole-writer contracts (extractor, resolver, chunks), schema version (stays at v25), compound hook semantics, exact-match-only resolution mode, and the `kg_subject_resolutions` / `kg_resolutions_log` tables are untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Pre-existing `kg_extractions` rows have `source_doc_hash=NULL` after v26 migration; first post-deploy boot treats every one of those docs as pending and re-extracts. | Accepted and bounded: the first post-deploy boot runs one extraction batch per agent capped at `budget`. Worst case ~N × budget extraction calls, which is exactly what the budget guard exists for. From the second boot onward, all rows have a hash and extraction is a no-op. This is the same first-boot behavior as the original v25 landing — only this time it's bounded and loud. Post-deploy verification Signal A is explicitly worded to test on the *second* restart for that reason. |
| Convergence-test drift: fresh v26 install vs. migrated v25→v26 install produce different schemas. | Extend the existing v24→v25 convergence test at `db.rs:10992` to also cover v26. This is the forward-test harness pattern from #686 doing the job it was designed for. |
| Budget default (500) is too tight for a fresh install with many docs_root-sharing agents. | Per-startup worst case is `2 × N × budget` (extraction + resolution). With N=11, default=500 → ~11,000 calls, still ~3× below the incident. Operators who legitimately need a bigger bound override via `MIKA_KG_BATCH_BUDGET`. Document default + tradeoff in `CLAUDE.md`. |
| Budget exhaustion silently accrues partial state if operators never notice the WARN. | Mitigation is structural: partial state is valid (pending work resumes next run, per-doc transactions). Log signal B in Unit 5 makes "budget exhausted" a first-class observability event operators check post-deploy. |
| Anthropic provider WARN fires on every restart, creating log noise. | Emit at most once per startup per role (extraction / resolution) via a `OnceLock` / `AtomicBool` at the call site in `server/mod.rs`. |
| Fan-out INFO log is noisy when run with 1 agent (no multiplier). | Only emit `kg_shared_docs_root` when the group size is ≥ 2. Single-agent case is silent. |
| Test coverage using `MockLlmProvider` could drift from real provider behavior. | Real-provider tests are gated behind `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` per repo convention — not added in this PR. Vincent's post-deploy verification covers the real-world integration path. |
| Per-startup cap under pathological fan-out (20+ agents sharing docs_root) could still exceed operator tolerance. | Out of scope for this rescue. Cap is `2 × N × budget`; if N grows that much, operators should either share extraction across agents (AC #3 option (a), deferred) or lower the budget. Documented in `CLAUDE.md` alongside the per-agent fan-out guidance. |

## Documentation / Operational Notes

- **CLAUDE.md (root)** — KG section gets the new env var + provider guidance.
- **CLAUDE.md (`crates/mika-agent`)** — subject-extractor / entity-resolver sections reference the budget and idempotency hash.
- **`kg_schema.rs`** — canonical idempotency key doc + fan-out cost model doc.
- **`.env.example`** — new `MIKA_KG_BATCH_BUDGET`, reworded provider example.
- **PR body** — must include the three post-deploy verification signals (Unit 5).
- **Post-deploy ops (Vincent, not this PR):** swap `MIKA_KG_*_MODEL` to an openrouter/* model before the next restart to drain the half-done resolver backlog cheaply, then confirm the three signals.

## Sources & References

- **Origin document:** GitHub issue senara-solutions/mika#757.
- **Related code:** `crates/mika-agent/src/kg/{subject_extractor.rs,entity_resolver.rs,ingestion_orchestrator.rs}`, `crates/mika-agent/src/db/kg_schema.rs`, `crates/mika-agent/src/server/mod.rs`, `crates/mika-common/src/config.rs`.
- **Related PRs/issues:** #689 (lexical ingestor), #690 (subject extractor), #691 (entity resolver), #692 (self-knowledge upgrade), #740 (KG self-knowledge eval, merged today), #758 (companion eval PR).
- **Memory references:** `project_knowledge_graph.md`, `feedback_transport_vs_workflow.md`, `feedback_prompt_enforcement_fragile.md`.
