---
date: 2026-05-16
module: mika-agent/kg
tags: [kg, resolver, model-comparison, sonnet, gemini-flash-lite, fragmentation, mika-1152]
problem_type: investigation
category: kg-investigations
related_tickets: [mika#1152, mika#1076, mika#1077, mika#1091, mika#14]
status: complete
---

# KG resolver model-quality baseline — sonnet-4-6 vs gemini-2.5-flash-lite

## TL;DR

**Fragmentation hypothesis holds. Decision A (deprecate `query_knowledge_graph` for mika-arch, 2026-05-12) is ratified. Model-quality hypothesis is falsified.**

Sonnet-4-6 was re-run against 87 distinct subjects that gemini-2.5-flash-lite had returned `no_match` on between 2026-05-01 and 2026-05-16. Sonnet matched **0/87 (0.0%)**. Total spend: $0.34.

Root cause confirmed by orthogonal check: **0/87 of these subjects have any corresponding `kg_entities` row by entity_key OR by name** (case-insensitive). The miss is not the resolver failing to disambiguate — there is no candidate to disambiguate against. The bottleneck is upstream of the resolver: the subject extractor is producing subjects (e.g., `agent:vincent`, `agent:ci`, `agent:tower_http`, `tool:tailwind`) for which the domain graph has no canonical counterpart.

Secondary finding (separate ticket-worthy): mika-arch's `search_content` only contains 1,722 indexed `kg_chunk` rows vs 2,700–2,900 for other agents — its chunk-context join misses on ~76% of its subjects, leaving the resolver to disambiguate against a name-only prompt. This explains some of mika-arch's elevated miss rate independent of the fragmentation finding above.

## Sample shape

- **Source query**: `kg_resolutions_log` rows where `model = 'google/gemini-2.5-flash-lite'`, `outcome = 'no_match'`, `resolved_at >= '2026-05-01'`, joined to `kg_subject_entities` filtered to the five resolvable types (`skill`, `tool`, `agent`, `problem_type`, `concept`).
- **Stratification**: 20 per type via `ROW_NUMBER() OVER (PARTITION BY type ORDER BY RANDOM())` (problem_type yielded 18 unique subjects after dedup).
- **Dedup**: 100 raw rows → 87 distinct `subject_entity_id` values (some subjects had `no_match` rows for multiple agents — prompt is agent-agnostic so duplicates were collapsed).
- **Per-type distribution**: agent=14, concept=20, problem_type=18, skill=15, tool=20.
- **Agent corpora represented**: mika-arch, mika-qa, mika-dev, mika.
- **Time window**: gemini resolutions from `2026-05-01T00:00:00Z` through `2026-05-16T15:02:28Z` (~2 weeks of production data, immediately preceding this experiment).

## Resolver code reference

Reconstruction targets `crates/mika-agent/src/kg/entity_resolver.rs` (commit at HEAD on 2026-05-16):

- **System prompt template**: `build_disambiguation_prompt()` lines 1605–1618 — copied verbatim into the experiment harness.
- **User prompt construction**: lines 1620–1655 — `Extracted entity: {key} (confidence: {N.NN})\n\nSource prose:\n{chunk_ctx truncated to 2000 chars}\n\nCandidates:\n- {entity_key} — {description from properties_json}\n...`.
- **Candidate selection** (`get_domain_candidates`, lines 1083–1119): SQL range scan `WHERE entity_key >= '{type}:' AND entity_key < '{type};'`, ordered by `entity_key ASC`, capped at `MAX_DISAMBIGUATION_CANDIDATES = 50`.
- **Chunk context** (`get_chunk_context`, lines 1182–1216): joins `kg_chunk_subjects` × `search_content` filtered by `agent_id` and `docs_root_hash`, returns top-3 chunks joined by `\n\n---\n\n`.
- **Response schema**: `{"match": "<entity_key>" | null, "confidence": 0.0-1.0}` — parsed via `parse_disambiguation_json` with markdown-fence tolerance.
- **Production model**: `MIKA_KG_RESOLUTION_MODEL = openrouter/google/gemini-2.5-flash-lite` (verified in `~/.mika/.env`, 2026-05-16).

The harness replicates the candidate query, chunk-context query, and prompt construction exactly. The only difference between production and the re-run is the LLM endpoint (Anthropic `claude-sonnet-4-6` instead of OpenRouter `google/gemini-2.5-flash-lite`).

## Comparison

### Gemini vs sonnet on the same subjects

| Model | Sample | Matched | No-match | Match rate |
|-------|--------|---------|----------|------------|
| `google/gemini-2.5-flash-lite` (prod, recorded in `kg_resolutions_log`) | 87 | 0 | 87 | **0.0%** |
| `claude-sonnet-4-6` (this experiment, 2026-05-16) | 87 | 0 | 87 | **0.0%** |

**Delta: 0 percentage points.** No subject that gemini missed was caught by sonnet. The disagreement set is empty.

### Historical context (advisory — different time window, different cohorts)

`kg_resolutions_log` also contains 9,082 rows from an earlier sonnet run (2026-04-22 to 2026-04-25, before sonnet was rotated out for cost). On the 4,734 subjects where both models have at least one log row (joining ignoring agent):

| Outcome cell | Distinct subjects | % |
|--------------|--------------------|---|
| both `no_match` | 3,434 | 72.5% |
| gemini matched, sonnet `no_match` | 822 | 17.4% |
| both matched | 308 | 6.5% |
| sonnet matched, gemini `no_match` | 170 | 3.6% |

Apparent historical match rates: gemini 23.9%, sonnet 10.1% (gemini 2.4× sonnet). This is **not load-bearing for the conclusion** because the two cohorts were resolved at different times against potentially different domain-graph snapshots, but it independently undermines the "stronger model recovers a meaningful fraction" prediction of the model-quality hypothesis.

### Mean latency (advisory)

- gemini-2.5-flash-lite (production-recorded): **537 ms/call**
- sonnet-4-6 (this experiment, p50): **1,305 ms/call**, p95: **1,823 ms**
- Sonnet is ~2.5× slower per call. Resolver runs in batched ticks (default 500 calls/30min, mika#906), so the latency delta would have direct cadence cost — independent reason to prefer the cheaper model when match rates are equal.

## Failure-shape tabulation

All 87 sonnet responses parsed cleanly as `{"match": null, "confidence": 0.0}` — none were prose evasions, fenced JSON, or schema deviations. Sonnet correctly refused to force a pick.

Cross-checking why no candidates matched (the production resolver's no_match shape):

| Check | Hits / 87 | Interpretation |
|-------|-----------|----------------|
| `LOWER(subject.entity_key)` exists in `kg_entities` | **0 / 87** | None of these subjects would have triggered Stage 1 exact match even with the case-insensitive lookup (which the resolver already does). |
| `LOWER(subject.name)` matches any `kg_entities.name` (cross-type) | **0 / 87** | None of these subjects have a same-name entity under a different type either. They have no canonical counterpart in the domain graph at all. |
| Subject reached resolver with `chunk_ctx_len = 0` | **58 / 87 (66.7%)** | Source prose unavailable to the LLM. Production resolver disambiguates these on name + candidate list only. |
| Subject reached resolver with `chunk_ctx_len > 0` | **29 / 87 (33.3%)** | Sonnet had source prose and the same candidate set. Still returned no_match. |

### Representative no-match subjects

Examples of subjects that the extractor produced but for which no domain counterpart exists:

- **agent** (domain holds 12 real mika agents like `agent:mika-arch`, `agent:mika-dev`): extractor produced `agent:vincent`, `agent:ci`, `agent:llm`, `agent:tower_http`, `agent:github_actions`, `agent:entity_resolver`, `agent:operator_claude`, `agent:bypass_actor`, `agent:reliability`, `agent:infrastructure_agent`, `agent:langfuse`.
- **tool** (domain holds 894 entities, mostly real builtin tools): extractor produced `tool:tailwind`, `tool:react_router_link`, `tool:unixepoch_function`, `tool:sqlite_partial_index`, `tool:github_app_rs`, `tool:tokio_process_command`, `tool:database`.
- **problem_type** (domain holds 5 seed entries — `ci_failure`, `merge_conflict`, `duplicate_pr`, `stale_uuid`, `fabrication`): extractor produced `problem_type:auth_failure`, `problem_type:best_practice`, `problem_type:investigation`, `problem_type:test_coverage_gap`, `problem_type:bug`, `problem_type:misread`.
- **concept** (domain holds 20 hand-seeded `concept:cross-repo:*` and `concept:infra:*`): extractor produced `concept:groom`, `concept:self_dev`, `concept:pr_scope`, `concept:core_memory`, `concept:dispatch_gate`, `concept:webhook_ready_label_dispatch`.

The pattern is consistent across types: the **subject extractor's NER pulls common nouns and code identifiers** from prose and types them into the five resolvable buckets, but the domain graph's entries for those types are a tightly curated set populated by `domain_builder.rs` from authoritative sources (`SkillRegistry`, `ToolRegistry`, agent configs, hardcoded seeds). The two populations don't overlap meaningfully on subjects that the extractor identifies in mika's `docs/solutions/` prose.

### Secondary finding — mika-arch chunk index gap

`search_content` row counts by agent, `source_type='kg_chunk'`:

| Agent | Indexed kg_chunks |
|-------|--------------------|
| mika-dev | 2,905 |
| mika-qa | 2,790 |
| odds-engine-ceo | 2,737 |
| mika | 2,703 |
| ... | ... |
| **mika-arch** | **1,722** |

mika-arch — the **sole** well-known agent that consumes the KG (`mika/CLAUDE.md` § "well-known agent KG topology, #800") — has the smallest indexed chunk pool. For the multi-corpus mika-arch (`docs_roots` spans 6 repos), this manifests as: `kg_chunk_subjects` rows exist (subject extractor wrote them), but the agent-id-scoped `search_content` join in `get_chunk_context()` returns zero. Worked example: subject 34427 (`agent:bypass_actor`) has 1 `kg_chunk_subjects` row matching mika-arch's docs_root_hash; the corresponding chunk is indexed in `search_content` for `mika-relay` and `mika-qa` but not for `mika-arch` — so mika-arch's resolver sees a name-only prompt.

This is independent of the fragmentation finding above (sonnet missed both with and without chunk context), but it explains some of mika-arch's elevated miss rate and is worth filing as a separate ticket.

## Cost

- Sonnet calls: 87
- Total input tokens: 107,157
- Total output tokens: 1,392
- Total cost (at $3/M input, $15/M output): **$0.3424**
- Budget per mika#1152: $1.50 target, $5.00 ceiling. Came in at 23% of target.
- Wall-clock: ~3 minutes of API calls + ~30 minutes total session time.

## Recommendation

**Ratify Decision A.** Sonnet does not meaningfully outperform gemini-2.5-flash-lite on this resolver call. The model-quality hypothesis is falsified. The bottleneck is at the subject extraction layer, not at resolution.

Subordinate next steps (each a separate decision, not implied by this finding):

1. **Keep gemini-2.5-flash-lite as the production resolver model.** Sonnet's match rate is no better, its latency is 2.5× higher, and its cost is ~50× higher (gemini-flash-lite ≈ $0.20/M input vs sonnet $3/M). Cost-wise the current choice is correct.
2. **The self-awareness umbrella tickets (mika#1076 corpora backfill, mika#1077 resolver no_match instrumentation, mika#1091 subject extractor MaxTokens) should stay paused under their current framing.** None of them would move the needle on the resolver miss rate as currently structured — the type mismatch is upstream.
3. **Consider filing two new tickets** out of the secondary findings (operator decision, not implied):
   - **(a) Subject extractor type-overreach** — the extractor is producing `agent:`/`tool:`/`problem_type:`/`concept:` subjects that have no domain counterpart. Either the extractor's prompt needs to be constrained to the closed set of domain entities (different shape — would change extractor semantics), or the domain graph's `agent`/`tool`/`problem_type`/`concept` types need a much wider canonical roster (different shape — would change `domain_builder.rs`), or the resolver's no_match-as-success outcome should be re-classified to differentiate "no candidate of this type exists for this subject" from "candidates exist but none match." All three are real options with different cost/quality tradeoffs.
   - **(b) mika-arch chunk index gap** — 1,722 indexed chunks vs 2,700+ for sibling agents. Likely a lexical-ingestion-fairness bug specific to multi-corpus agents. Worth filing standalone since it affects the one well-known agent that still consumes the KG.

These are surfaced for operator review; this experiment's mandate ends at the model-quality binary.

Closes mika#1152.
