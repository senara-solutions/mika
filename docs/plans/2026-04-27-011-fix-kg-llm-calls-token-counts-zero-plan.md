# Plan: Fix llm_calls token counts always 0 for KG callsites (mika#799)

**Ticket:** senara-solutions/mika#799
**Branch:** `fix/799/anthropic-llm-calls-tokens-zero`
**Type:** bug, p1-important, agent-core
**Authored:** 2026-04-27
**Architect review:** mika-arch session `acbd0d4f-6b67-48a4-b431-ee769b18db0a` — Disposition: ITERATE on first pass; revisions below address all 9 findings (F2 two-rows-not-aggregation is load-bearing).

## Problem

The ticket reports: every Anthropic `llm_calls` row records `input_tokens=0, output_tokens=0` post-restart on main @ `3907e833`, despite logs showing real values. Cost tracking, per-agent spend, and forensics all read this column → all wrong.

**Investigation extends Vincent's 2026-04-25 audit** (issue comment 11:07Z) which correctly narrowed scope to KG paths and ruled out conversation-mode regression. This plan goes further: the KG-path zeros are not a #795/#796 regression at all; they are pre-existing hardcoded zeros at the two KG callsites that have been there since the KG subsystem first shipped. #795/#796 amplified visibility (more KG calls per restart) but did not change behavior.

**Evidence:**

1. **Hardcoded zeros at the two KG call sites:**
   - `crates/mika-agent/src/kg/subject_extractor.rs:1276-1277`:
     ```rust
     0, // input_tokens not available from extraction calls
     0, // output_tokens not available
     ```
   - `crates/mika-agent/src/kg/entity_resolver.rs:1133-1134`:
     ```rust
     0,
     0,
     ```
2. **The hardcoded zeros were introduced when these subsystems were initially shipped:**
   - `git log -- crates/mika-agent/src/kg/subject_extractor.rs` → first appearance commit `4333a54e` (#690, "subject graph extraction" — Feb 2026).
   - `git log -- crates/mika-agent/src/kg/entity_resolver.rs` → first appearance commit `b4f0eb92` (#691, "entity resolution" — Feb 2026).
3. **`agent.rs` was NOT touched in #795 or #796:**
   - `git diff bb593c39^..3907e833 -- crates/mika-agent/src/agent.rs` returns empty.
   - Conversation-mode `save_llm_call` site at `agent.rs:686-700` correctly passes `resp.usage.input_tokens`, `resp.usage.output_tokens`, `resp.usage.cache_read_input_tokens`, `resp.usage.cache_creation_input_tokens`.
4. **Why #795/#796 made the bug visible:**
   - #778 introduced per-agent KG (`[kg].docs_root` in identity.toml). After deploy, more agents now run their own resolution + extraction against per-agent corpora, multiplying KG-path llm_calls volume.
   - The operator's reproduction (`MIKA_KG_RESOLUTION_MODEL=anthropic/claude-sonnet-4-6`) routed all that volume through Anthropic. So the 1,933 post-restart Anthropic llm_calls rows in the reported window were 1,933 KG-path calls — every one of which has been hardcoded to 0 since #690/#691.

## Decision

**Thread real `Usage` from each LLM response through to `save_llm_call` at both KG callsites, writing one `llm_calls` row per LLM API call.**

The fix is scoped, low-risk, and matches the existing convention at `agent.rs:686-700` (the canonical "this is how to save an llm_calls row with real tokens" template). No new types, no schema changes.

## Architecture

### Subject extractor (`crates/mika-agent/src/kg/subject_extractor.rs`)

- The LLM call lives in `extract_with_retry` (around line 853 / 875 / 938 — the retry-with-reinforcement path also calls `send_message`). Each successful `send_message` returns `LlmResponse` with `response.usage: Usage`.
- Currently the extraction path returns only the parsed `ExtractionOutput` (`Result<Option<ExtractionOutput>>`), discarding `usage`.
- Change: thread `usage` alongside the parsed output. Concretely, change the return shape from `Option<ExtractionOutput>` to a small struct `ExtractionResult { output: ExtractionOutput, usages: Vec<Usage> }`. **`usages` is a Vec because the retry-with-reinforcement path may make a second LLM call** — each call gets its own entry. Single-attempt extractions have `usages.len() == 1`; retried extractions have `usages.len() == 2`.
- `store_llm_call` signature changes from `(_doc_path, latency_ms, _output)` to `(_doc_path, latency_ms_per_call: u64, usage: &Usage, kg_phase: &str)`. Caller invokes `store_llm_call` **once per element of `usages`**, with `kg_phase = "kg_extraction"` for the first call and `kg_phase = "kg_extraction_retry"` for the retry. Per-call `latency_ms` is captured at each `send_message` site (not summed).

### Entity resolver (`crates/mika-agent/src/kg/entity_resolver.rs`)

- Same shape. The Stage-2 disambiguation LLM call returns `LlmResponse` with `usage`.
- `store_llm_call` signature gains `usage: &Usage` and `kg_phase: &str`. Replace hardcoded `0, 0`. Stage-1 (exact match) costs no LLM call and writes no `llm_calls` row, untouched.
- Resolver's retry behavior (per `resolution_retry_transport_failed` warn at line 1111-1116): if the resolver also has a retry path that fires a second LLM call, that call gets its own `llm_calls` row with `kg_phase = "kg_resolution_retry"`. If resolver only has transport-retry inside the LLM provider (no second `send_message` call), it stays as a single row.

### Why two rows per logical operation, not one aggregated row

Per architect Finding 2 (load-bearing):

1. **Cache fields aren't additive.** Cache reads on attempt 2 are cheaper than attempt 1's full input; aggregating produces values that don't correspond to any real billing event.
2. **Cost dashboards lose granularity.** Per-call cost = `input × in_rate + output × out_rate + cache_read × cr_rate + cache_creation × cc_rate`. Summing inputs across two calls and computing once produces wrong dollars when rates differ across cache states.
3. **Retry-rate observability breaks.** "How often does the retry-with-reinforcement path fire?" is unanswerable from aggregated rows. Two rows = two `llm_calls.id`s = countable retries via `SELECT COUNT(*) ... WHERE source = 'kg_extraction_retry'`.

`llm_calls` is a per-API-call ledger, not a per-logical-operation ledger. Same shape as financial transaction tables: one row per ledger entry, not per business intent.

### `Usage` struct fields to forward

From `crates/mika-common/src/llm/types.rs:162-168`:
```rust
pub input_tokens: u64,
pub output_tokens: u64,
pub cache_creation_input_tokens: Option<u64>,
pub cache_read_input_tokens: Option<u64>,
```

`save_llm_call` already takes all four (verified via `agent.rs:687-690` callsite). Pass the cache fields too — costing for prompt caching matters for KG bulk extraction.

### `store_llm_call` signature parity (per architect Finding 9)

After the fix, both `subject_extractor::store_llm_call` and `entity_resolver::store_llm_call` have the identical signature shape:

```rust
async fn store_llm_call(
    &self,
    context_key: &str,        // doc_path for extractor; entity_key for resolver
    latency_ms: u64,
    usage: &Usage,
    kg_phase: &str,           // "kg_extraction" / "kg_extraction_retry" / "kg_resolution" / "kg_resolution_retry"
)
```

(The first arg's semantic differs between the two — doc_path vs entity_key — but the shape is identical: `&str` context, latency, usage, phase.)

## Files

### Modified
- `crates/mika-agent/src/kg/subject_extractor.rs`:
    - Add `ExtractionResult { output: ExtractionOutput, usages: Vec<Usage> }`.
    - Thread `Usage` from each `send_message` callsite (lines 853, 875, 938) into `usages`.
    - `store_llm_call` signature gains `usage: &Usage` + `kg_phase: &str`. Called once per element of `usages` with appropriate phase tag.
    - **Delete the misleading comment** at line 1276 ("input_tokens not available from extraction calls"). Name the deletion in the commit message (per architect Finding 5).
- `crates/mika-agent/src/kg/entity_resolver.rs`:
    - Same threading shape for Stage-2 disambiguation call.
    - `store_llm_call` signature parity with extractor's.
    - If a retry callsite exists, it writes a second row tagged `kg_resolution_retry`.
- `crates/mika-agent/CLAUDE.md` — strike the "input_tokens not available" lineage; add a one-liner under § Knowledge Graph clarifying that KG callsites write per-API-call `llm_calls` rows with `kg_phase`-tagged `source` field.

### Tests (per architect Findings 7 + 8)

Inline `#[cfg(test)] mod tests` in both modules. Each test asserts all four `Usage` fields are forwarded correctly:

```rust
let mock_resp = LlmResponse {
    usage: Usage {
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_input_tokens: Some(20),
        cache_read_input_tokens: Some(10),
    },
    ...
};
// after the call:
let row = fetch_latest_llm_call(&db).await;
assert_eq!(row.input_tokens, 100);
assert_eq!(row.output_tokens, 50);
assert_eq!(row.cache_creation_input_tokens, Some(20));
assert_eq!(row.cache_read_input_tokens, Some(10));
```

**Provider-agnostic verification (Finding 8):** include at least one Anthropic-routed test case AND one non-Anthropic-routed test case (e.g., OpenAI or DeepSeek mock). Mocks are easy — `LlmResponse.usage` is provider-agnostic; the test just exercises that the threading works regardless of provider name.

**Two-rows verification (Finding 2):** include a test that forces the retry-with-reinforcement path (mock a malformed JSON on first attempt, valid on second) and asserts:
- Two `llm_calls` rows are written.
- Each has correct usage from its respective LLM call.
- One row has `source = 'kg_extraction'` and the other `source = 'kg_extraction_retry'`.

### No change
- `crates/mika-agent/src/agent.rs` — already correct; reference site for KG.
- `crates/mika-agent/src/db.rs` — `save_llm_call` signature already accepts all needed fields.
- `crates/mika-common/src/llm/*.rs` — `LlmResponse.usage` already populated by all providers.
- Schema — no migration; `llm_calls` columns already exist (and are filled to 0 for the broken rows; we do not retroactively backfill — see § Out of scope).

## Verification

### Real-system smoke
1. Build with the fix.
2. Restart `mika-server`.
3. Trigger KG resolution work using `MIKA_KG_RESOLUTION_MODEL=anthropic/claude-sonnet-4-6` (or any Anthropic-routed call).
4. Query:
   ```sql
   SELECT MAX(input_tokens), MAX(output_tokens), COUNT(*)
   FROM llm_calls
   WHERE provider='anthropic'
     AND source IN ('kg_resolution','kg_resolution_retry','kg_extraction','kg_extraction_retry')
     AND created_at >= '<post-restart>';
   ```
   Expect non-zero MAX values, COUNT matches the operator-side activity.
5. Spot-check log lines: pick three random `llm_call completed` log entries from the same window and confirm their `input_tokens` / `output_tokens` match the stored row by joining on `trace_id`.
6. **Conversation-mode regression check:** trigger a non-KG Anthropic call (e.g., `mika ask --agent <agent>` with a default-model Anthropic config). Confirm its `llm_calls` row has non-zero tokens. (Sanity check that we didn't accidentally break the previously-correct path.)
7. **Retry-row verification:** if a retry path naturally fires (or via a deliberate malformed-corpus test fixture), confirm two rows appear with distinct `source` tags.

### Dashboard signal
- Per-agent cost view shows non-$0 for KG work after the deploy.

## Out of scope

- **Backfilling the 1,933 broken rows.** Old rows stay at 0; we don't have the original token counts (logs may have rotated). Operators querying historical KG cost should know the columns are stale before this fix. **SQL caveat for historical aggregates** (per architect Finding 4):
    ```sql
    -- "broken-row count" the operator can subtract from any historical KG-cost aggregate.
    -- Honest caveat: any pre-fix `0, 0` row from a KG-routed model is likely undercount.
    -- Conversation-mode `0, 0` rows (small no-op calls) may be genuine; do not subtract them.
    SELECT COUNT(*)
      FROM llm_calls
     WHERE input_tokens = 0
       AND output_tokens = 0
       AND source IN ('kg_resolution','kg_extraction')  -- NOT IN ('null','none','...')
       AND created_at < '<fix-deploy-timestamp>';
    ```
    The dashboard documentation should annotate the pre-fix window with this caveat.
- **Other providers.** The bug is at the KG callsites (provider-agnostic — the hardcoded zeros affect any provider routed through KG). The ticket focused on Anthropic because that's what the operator routed; the fix benefits every provider equally. Test matrix exercises both Anthropic and one non-Anthropic mock to make the provider-agnostic claim mechanically verified rather than asserted.
- **Per-call cost computation.** This plan stores the raw token counts. Cost = tokens × pricing is a downstream concern; not in scope here.
- **Adding `usage` fields to error-path llm_calls rows beyond what the provider returned before erroring.** Best-effort only.
- **Eval-harness integration test against real provider** (per architect Finding 3). Unit tests with mocked `LlmResponse` cover the threading; a real-provider eval-harness test covers full-path wiring but adds API-key gating burden. **Filed as follow-up:** I'll open a sub-issue / sibling ticket post-merge titled "kg/eval: real-provider integration test for KG-path llm_calls token recording" with explicit assertion `post-fix, KG-path llm_calls rows from a real-provider call have non-zero tokens`. Linked from this PR's description.

## Risks

1. **API drift across providers.** Some providers (e.g., DeepSeek, Groq) might not populate cache fields. `Usage.cache_*` are `Option<u64>`, so `None` continues to map to NULL in the DB — no risk.
2. **Tokio test harness friction.** The KG modules use the `MockLlmProvider`. If existing extractor tests don't mock `usage`, they'll need an update to include non-zero usage values. Not invasive — the field shape is already there.
3. **Retry path that doesn't actually exist.** The plan assumes both extractor and resolver have retry-with-reinforcement paths that fire a second `send_message`. If only the extractor does (and resolver's retry is provider-internal), only the extractor gets two rows. This is correct — the rule is "one row per `send_message` call." No special-casing.

## Companion / related

- #795 (per-agent KG docs_root) — visibility multiplier (more agents = more KG calls = more visible 0-rows). Not the cause.
- #796 (KG management CLI) — visibility multiplier. Not the cause.
- #690 (subject graph extraction) — original site of `subject_extractor.rs:1276` zeros.
- #691 (entity resolution) — original site of `entity_resolver.rs:1133` zeros.
- Vincent's comment on #799 (2026-04-25 11:07Z) — narrowed scope to KG paths; this plan extends that narrowing to identify origin in #690/#691.

## Diagnostic correction note (for PR description and ticket comment post-merge)

Vincent's 2026-04-25 audit on this ticket correctly narrowed scope to KG paths and ruled out the conversation-mode regression. This investigation extends that narrowing: the KG-path zeros aren't a #795/#796 regression at all — they've been hardcoded `0, 0` since #690 (`subject_extractor.rs`, ~Feb 2026) and #691 (`entity_resolver.rs`, ~Feb 2026). #795/#796 amplified visibility by routing more volume through Anthropic per #778's per-agent KG; the underlying bug predates them by months.

Operators reading the ticket later should know: the fix landed at the KG callsites, not in the agent loop or LLM provider code. Pre-fix rows from KG-path Anthropic calls undercount cost; post-fix rows record real values.

## Architect-review changelog

This plan was revised once after a first-pass review by mika-arch (session `acbd0d4f-6b67-48a4-b431-ee769b18db0a`). Findings addressed:

- **F1** — Reframed the diagnostic note to **extend** Vincent's 2026-04-25 audit (which narrowed to KG), not contradict it. Plan § Problem now opens with "extends Vincent's 2026-04-25 audit"; new § "Diagnostic correction note" is explicit about the two-step refinement (Vincent narrowed → this PR identifies origin).
- **F2 (load-bearing)** — Changed first-pass + retry-pass from aggregation to **two rows** (one per LLM API call). Added `kg_phase` parameter to `store_llm_call` (`kg_extraction` / `kg_extraction_retry` / `kg_resolution` / `kg_resolution_retry`) so retry-rate observability is queryable. Added named test for two-rows behavior.
- **F3** — Integration test deferred to a follow-up issue, **explicitly named in § Out of scope** with the exact assertion shape; will be filed post-merge and linked from PR description.
- **F4** — Added SQL caveat for historical aggregates in § Out of scope, scoped to KG-path rows only (`source IN ('kg_resolution','kg_extraction')`).
- **F5** — Plan § Files / Modified explicitly names the comment deletion at `subject_extractor.rs:1276` and instructs commit message to call it out so future `git blame` lands on the fix commit with explanation.
- **F6** — Confirmed: keep underscored params; minimal diff. No-op.
- **F7** — Added explicit four-field test assertion enumeration: `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`.
- **F8** — Added provider-agnostic verification: test matrix includes one Anthropic-routed mock AND one non-Anthropic-routed mock.
- **F9** — Added § "`store_llm_call` signature parity" stating the post-fix signatures are identical across both modules.
