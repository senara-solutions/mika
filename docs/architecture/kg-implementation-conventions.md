# KG Implementation Conventions

**Status:** Active
**Created:** 2026-04-21
**Applies to:** milestone mika#14 (Knowledge Graph) — tickets #687–#692

This document captures cross-cutting decisions that apply to multiple KG tickets. Each ticket plan cites this doc rather than relitigating these decisions individually. If you are planning a KG ticket and find yourself reasoning about embedding budgets, non-interactive LLM calls, or audit/observability conventions, read this first.

Companion document: [`kg-id-convention.md`](./kg-id-convention.md) covers the typed-prefix ID scheme.

---

## C1. Embedding policy — piggyback, never block

KG chunks and any future embeddable content flow through the existing Layer 3 infrastructure:

- Model: OpenAI `text-embedding-3-small`, 512 dimensions.
- Storage: `search_content.embedding_json` + `vec_search` virtual table (`crates/mika-agent/src/db.rs`).
- Backfill: existing path with batch size 100, idempotent by `embedding_json IS NULL`.
- Indexing is best-effort — a failed embedding call never fails the tool response or the ingestion operation.

There is **no KG-specific embedding quota or concurrency cap**. Economics do not justify one: `text-embedding-3-small` costs ~$0.02 per 1M tokens; a 10,000-chunk ingestion run (~500 tokens per chunk) costs ~$0.10. The KG would need to be ingesting at a scale of millions of chunks before embedding cost becomes a material concern, at which point quota design should be informed by real usage data rather than speculation.

### C1.1 The async-embedding contract (mandatory for #689 and #690)

**Ingestion writes commit without waiting for embeddings.** The transactional boundary is:

1. Insert `kg_chunks` row (or equivalent subject-layer row).
2. Insert corresponding `search_content` row with `embedding_json = NULL`.
3. Insert corresponding `fts_search` row (FTS5 indexing).
4. **COMMIT.**

The actual embedding call is **not** part of the transaction. It runs later via the existing backfill path, which is idempotent by `embedding_json IS NULL`.

**Three consequences ingestion code must handle:**

1. **Ingestion latency is bounded.** A user-facing "ingest this doc" operation returns in chunk+write time, not chunk+write+embed time. A large doc's ingestion completes in seconds, not tens of seconds.
2. **Ingestion failures are decoupled from embedding failures.** If OpenAI is down, rate-limited, or the agent has no embedding API key: ingestion still succeeds. Chunks are immediately queryable via FTS5 and become queryable via vector search once the backfill catches up.
3. **Freshly ingested content may not be in `vec_search` yet.** Callers that read immediately after writing must either accept FTS5-only results in that window, or poll — but no waiter may block a user-facing operation. Document this in every ingestor's contract.

Every KG ingestion ticket should include this paragraph verbatim or in equivalent language in its Key Technical Decisions section:

> Ingestion writes are synchronous and transactional for rows and FTS5 indexing. Embeddings are generated asynchronously by the existing backfill path and may not be available immediately after ingestion returns. Callers that need vector search results on freshly ingested content must either accept FTS5-only results until backfill catches up, or explicitly wait/poll — but no such waiter blocks user-facing operations.

---

## C2. Non-interactive LLM call policy

Non-interactive LLM calls are calls that happen during ingestion or background processing, not in response to a user turn. Current scope: #690 subject extraction, #691 entity resolution. Future tickets that add similar calls inherit this policy.

### C2.1 Model selection — two env vars, not one

Two environment variables cover the two distinct ingestion call types:

- `MIKA_KG_EXTRACTION_MODEL` — used by #690 for NER + fact-triple extraction from prose. Default: a cheap/fast tier (e.g., `deepseek-v3` or `haiku-4.5`). Task is mechanical — identify spans, classify, emit JSON.
- `MIKA_KG_RESOLUTION_MODEL` — used by #691 for ambiguous-case entity resolution arbitration. Default: mid-tier (better judgment than extraction requires, because errors produce persistent wrong edges that downstream queries depend on). May initially share the extraction model's default and escalate later if quality data suggests it should.

Both fall back to a shared `MIKA_KG_INGESTION_MODEL` if only that is set, so operators who want one knob can set one and move on.

**Do not** route ingestion calls through the agent's primary conversational model (`MIKA_PROVIDER` / `MIKA_MODEL`). Interactive reasoning and bulk extraction have different quality, latency, and cost profiles; coupling them means every model upgrade for conversational quality drags ingestion cost along, and every ingestion cost optimization risks degrading conversational quality.

**Do not** use per-skill `skill_overrides` as the default for ingestion category selection. Per-skill overrides are the right escape hatch if one specific skill needs a different model, but they scatter global ingestion policy across skill manifests — the "which model does ingestion use in this container" question should be answerable by reading one env var, not N files.

### C2.2 Retry taxonomy — four failure categories

All non-interactive LLM calls must distinguish four failure modes and handle each differently:

| Failure category | Detection | Handling |
|------------------|-----------|----------|
| **Transport** | Network error, timeout, 5xx from provider | Retry with exponential backoff up to N attempts (N recommended: 3). |
| **Rate limit** | HTTP 429 | Retry with backoff. Respect `Retry-After` header when present; otherwise use exponential backoff. |
| **Semantic (malformed output)** | Valid HTTP response but model returned non-parseable JSON or JSON that doesn't match the declared schema | One retry with a prompt reinforcement (e.g., "return valid JSON matching schema X"). If second attempt also fails, log-and-skip. Retrying more than once rarely helps — the model's failure mode on malformed output is usually consistent. |
| **Configuration** | HTTP 401/403, model-not-found, unsupported operation | Do not retry. Halt ingestion and surface an error. These indicate a configuration problem, not a transient failure; retrying spams the provider and logs without fixing anything. |

### C2.3 Log-and-skip preserves lexical state

When a subject extraction or entity resolution call hits a log-and-skip outcome (semantic failure after retry, or all transient retries exhausted), the already-committed **lexical chunks remain**. Only the subject-graph rows or resolution edges for that specific chunk/entity are dropped. Never let a downstream failure take down upstream committed state.

The ingestion pipeline is layered:

```
[1] Chunk write → [2] Index → [3] Extract subjects → [4] Resolve subjects to domain
```

Failures at stage 3 or 4 must not affect stages 1 or 2 that already committed. This is the orthogonality invariant — each stage owns its own success/failure, and later-stage failure cannot trigger earlier-stage rollback.

### C2.4 Non-interactive LLM calls emit observability rows

Every non-interactive LLM call emits a row to `llm_calls` with the invocation's `trace_id` (same as any other LLM call). This gives "how many extraction calls did agent X run this week against which model" as a one-query answer rather than requiring after-the-fact instrumentation.

---

## C3. Observability granularity — hybrid, per operation type

audit_events today is broader than "agent-facing user state." Existing entries include:
- Pure agent actions: `update_core_memory`, `store_fact`, `create_task`, `update_task_status`, `create_work_item`, etc.
- Server-side infrastructure: `ci_success_merge`, `webhook_replayed`, `webhook_deferred`, `verdict_handled`.
- Direct interventions: `manual_db_update`.

All existing rows carry a concrete `agent_id` (schema constraint: `NOT NULL REFERENCES agents(id)`). KG ingestion events that are naturally agent-scoped fit this pattern; KG operations that are container-wide (deterministic rebuild from manifests) do not.

### C3.1 Domain rebuild (#687): structured logs only, no audit_events

Domain graph rebuild is container-wide infrastructure with no agent attribution. It runs from deterministic startup code over manifests and registries. Emitting audit_events rows would require a sentinel `agent_id` or a schema change — neither is worth the cost.

Observability for domain rebuild lives in structured logs at INFO level with `trace_id`:

```
INFO trace_id=<id> event=domain_rebuild_start
INFO trace_id=<id> event=domain_rebuild_entities added=12 updated=3 removed=1
INFO trace_id=<id> event=domain_rebuild_edges added=47 removed=2
INFO trace_id=<id> event=domain_rebuild_complete duration_ms=234
```

### C3.2 Lexical ingestion (#689): per-document audit_events

Lexical ingestion is per-agent. Audit cadence is **per document**, not per chunk. A 500-chunk document produces one audit_events row, not 500.

- `tool_name`: `ingest_document`
- `target_key`: `kg_chunk:<source_doc_path>`
- `before_value`/`after_value`: summary counts (e.g., `{"chunks": 47, "total_chars": 23100}`)
- `reasoning`: source of the ingestion (e.g., `"solution doc ingestion"`)

The natural batch boundary is "one `ingest_document` call" regardless of how many chunks it fans out to.

### C3.3 Subject extraction (#690) and entity resolution (#691): per-operation audit_events

These are per-agent, per-document (extraction) or per-entity (resolution) operations. Audit cadence matches the natural batch:

- `tool_name`: `extract_subject_entities` (extraction, per document)
- `tool_name`: `resolve_subject_entity` (resolution, per subject entity)
- `target_key`: `kg_subject:<subject_entity_key>` or equivalent
- `before_value`/`after_value`: summary counts (e.g., `{"entities_extracted": 12, "triples": 8}`)

Extraction's `tool_name` firing per-document avoids the Option 2 trap (one audit row per extracted entity would be hundreds of rows for a single doc). Resolution's per-entity is defensible because resolution is an atomic decision per entity — one row per decision, not per candidate match.

### C3.4 Not a catch-all — no per-row mutations

Do not emit audit_events for individual `kg_entities` / `kg_relationships` / `kg_chunks` row insertions or deletions. That would re-create the god-log anti-pattern at the audit layer. The "what rows changed in this operation" detail belongs in structured logs (DEBUG level) and the created_at / updated_at timestamps on the KG tables themselves; audit_events captures meaningful-transition granularity, not per-row deltas.

---

## C4. Document lifecycle

This document is the authoritative source for C1–C3. Ticket plans (#687–#692) cite this doc by section (e.g., "per C2.2 retry taxonomy") rather than restating its content. Updates to this doc require:

1. Updating the relevant section with the new decision and rationale.
2. Noting the change in the ticket plan that drove it.
3. If the change is backwards-incompatible for earlier-planned tickets, flagging those tickets for re-review.

If a KG ticket needs a decision that isn't covered here, either (a) the decision is ticket-specific — handle it in the plan, or (b) it's cross-cutting — add a new section here and cite it from the ticket plan.
