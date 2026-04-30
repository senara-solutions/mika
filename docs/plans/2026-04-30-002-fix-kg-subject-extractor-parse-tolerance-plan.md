---
title: "fix(kg/subject_extractor): brace-matching parser + prompt reinforcement for haiku-class JSON failures"
type: fix
status: active
date: 2026-04-30
---

# fix(kg/subject_extractor): brace-matching parser + prompt reinforcement for haiku-class JSON failures

## Overview

KG subject-extraction batches frequently emit `extraction_parse_failed_retry` → `extraction_semantic_exhausted` → log-and-skip per C2.3, producing 0 entities for whole batches. Root cause is a JSON-parsing intolerance in `parse_extraction_json` at `crates/mika-agent/src/kg/subject_extractor.rs:966` combined with claude-haiku-4-5's known habit of emitting reasoning prose around the JSON object (sibling failure class to mika#768 in permission-policy). The fix is two-part: (a) replace `serde_json::from_str(cleaned)` with a brace-matching extractor that locates the first valid JSON object inside arbitrary surrounding text; (b) strengthen the extraction prompt to explicitly forbid prose-around-JSON, mirroring mika#768's prompt-side intervention. Both changes are additive — no schema changes, no new dependencies, no model swap.

## Problem Frame

Verified evidence (operator DB-evidence pre-check 2026-04-30, since mika#901 verbatim-findings emit isn't shipped):

- **Pending-docs gap is real and biased toward mika-arch secondary corpora:**

  | Agent | Corpus (`docs_root_hash`) | Chunks | Extracted | Pending | % pending |
  |---|---|---|---|---|---|
  | mika-arch | `ac0e96dc51b85b80` | 50 | 17 | **33** | 66% |
  | mika-arch | `98509090f0a833d2` | 51 | 29 | **22** | 43% |
  | mika-arch | `d7107cd14e544043` | 29 | 14 | **15** | 52% |
  | mika-arch / mika / mika-dev / mika-qa | `34b8cf03c80614f9` (shared primary) | 355 | 335 | 20 | 5.6% |
  | odds-engine-{ceo,cto,quant} | `62386bb31b9664e9` | 24 | 24 | 0 | 0% |

- **server.log evidence (cumulative):** 6600 `extraction_parse_failed_retry` events, 5764 `extraction_semantic_exhausted` (final fail), 263 distinct failing trace_ids. Vs ~444 `subject_extraction_complete` boundaries. The C2.3 mask (`failed=0` in batch summaries because failures don't count toward `failed`, only entities) hides the impact in operator-visible logs.

- **Real-time symptom (2026-04-30T15:18-15:22 UTC):** mika-arch and mika-dev `subject_extraction_complete` lines: `completed=26, failed=0, entities=0, relationships=0, llm_calls=26`. Same agent runs sometimes succeed (mika-qa: `entities=3, relationships=1`; mika: `entities=3, relationships=2`) — non-deterministic per-document, consistent with model-output variance.

- **Failure shape from server.log:** `"error":"failed to parse extraction JSON: "` (empty body after the colon — the parser couldn't even start to lex). Strongly suggests the LLM returned reasoning prose with the JSON embedded somewhere inside, and the markdown-stripping path at `parse_extraction_json:1598` test (`strips_markdown`) is insufficient — it strips ` ```json ... ``` ` fences but doesn't tolerate prose AROUND the JSON object.

- **Models in active use** (`kg_extractions.extraction_model`): `claude-haiku-4-5-20251001` (anthropic), `openai/gpt-5-nano` (openrouter). Both fail per ticket.

The C2.3 log-and-skip semantics mean failed docs stay pending and re-extracted next restart, but the parse failure is non-transient (same prompt, similar-shaped output), so the same docs fail again — pending count grows over time as new chunks arrive.

## Requirements Trace

- **R1.** Routine batches over the existing corpora produce `entities > 0` for ≥ 80% of docs (per ticket AC).
- **R2.** Track `entities_extracted` distribution in `kg_extractions` over a 24h window post-deploy (per ticket AC).
- **R3.** Parser tolerates reasoning prose before/after the JSON object — common haiku-class failure mode per mika#768.
- **R4.** Parser remains strict on the JSON object itself — no schema relaxation, no field-fudging. Validation errors still surface to C2.2 semantic retry.
- **R5.** Existing `parse_extraction_json` tests (`strips_markdown`, `plain` at `subject_extractor.rs:1598/1617`) continue to pass.
- **R6.** Cost predictability: no provider-native JSON-mode adoption (provider-specific, requires per-provider plumbing); no model swap (loses haiku speed/cost advantage). The fix stays prompt + parser, both reversible.
- **R7.** No regression on the `claude-haiku-4-5-20251001` and `openai/gpt-5-nano` provider paths — the parser must work for both, since both are in active use.

## Scope Boundaries

- **In scope:**
  - Brace-matching extractor in `parse_extraction_json` (Unit 1).
  - Extraction prompt reinforcement in the system prompt or instruction text constructed in `extract_document` (Unit 2).
  - Behavioral test fixtures replaying haiku-style prose-around-JSON shapes (Unit 3).
- **Out of scope:**
  - **Provider-native JSON-mode / response_format / tool-call output** (Option 2 from the ticket). Provider-specific plumbing across `crates/mika-common/src/llm/`. Defer until parser+prompt fix proves insufficient over a 24h post-deploy window. Per `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`: prompt-level fix is the cheapest first layer; structural escalation only on recurrence.
  - **Model swap** (Option 3). Loses haiku cost/speed advantage. The failure mode is recoverable at the parser layer.
  - **Schema relaxation** (e.g., accepting partially-malformed JSON, missing fields). Would risk silent data corruption in `kg_subject_entities` / `kg_subject_relationships` writes. Validation stays strict.
  - **Re-extraction backfill of historical pending docs.** Once the fix lands, the next restart's `extract_pending(budget)` call naturally re-processes all pending docs (per `crates/mika-agent/CLAUDE.md` § Pending-doc detection). No backfill migration needed.
  - Resolver-side concerns (mika#874, mika#906 — separate tickets in milestone#19).
  - Secondary-corpora resolution shape (mika#877 — separate ticket).

## Phase 0 Pins (load-bearing source verification)

### Pin 1: `parse_extraction_json` current implementation

`crates/mika-agent/src/kg/subject_extractor.rs:966` and surrounding test at line 1598:

```rust
fn parse_extraction_json(&self, text: &str) -> Result<ExtractionOutput> {
    // ... markdown-fence stripping (test confirms at line 1598)
    serde_json::from_str(cleaned).with_context(|| { /* ... */ })
}
```

Tests at line 1598 (`strips_markdown`) and line 1617 (`plain`) confirm:
- ` ```json\n{...}\n``` ` is handled (markdown fences stripped).
- Plain `{...}` is handled.
- **Not tested:** prose before/after JSON, e.g., `Here is the extraction:\n\n{...}\n\nThe entities cover...`.

### Pin 2: Failure-path call site

`subject_extractor.rs:856-880`:

```rust
match self.parse_extraction_json(&text) {
    Ok(output) => return Ok(output),
    Err(_) => {
        // Semantic failure — one retry with reinforcement (C2.2)
        warn!(
            event = "extraction_parse_failed_retry",
            "malformed JSON from LLM — retrying with reinforcement"
        );
        self.retry_with_reinforcement(&request, &text).await
        // ...
        return match self.parse_extraction_json(&text) {
            Ok(output) => Ok(output),
            Err(_) => self.retry_with_reinforcement(&request, &text).await,
            // ...
```

The retry path uses `retry_with_reinforcement` at line 921, which sends a follow-up message asking the LLM to reformat. If that also fails to parse, we hit `extraction_semantic_exhausted` (log-and-skip per C2.3).

**Implication:** Unit 1 must improve `parse_extraction_json` itself, not the retry path. The retry exists for genuinely-malformed JSON; today's failures are well-formed JSON inside prose, where the parser fails on the first byte.

### Pin 3: Extraction prompt site

`extract_document` at line 420 constructs the LLM request. The prompt content lives in `crates/mika-agent/src/kg/subject_extractor.rs` (need to grep for the literal at implementation time — likely a const or `format!()` template near `extract_document`). Unit 2 modifies this prompt to mirror mika#768's intervention pattern.

### Pin 4: Sibling fix reference — mika#768 (issue-body cross-ref, not shipped-evidence)

mika#768 documents identical failure class (haiku emits reasoning prose around JSON) in the permission-policy skill. Per #768 root-cause analysis: *"The prompt currently says things like '...' but doesn't tell the model that the JSON is the ONLY acceptable output."* mika#768 is OPEN (not yet shipped). **Reframe per first-pass F2:** the citation to #768 in this plan is via mika#876's own "Possible overlap" section in the issue body (which directs reuse), not via #768's shipping status. Reasoning chain: recipe applies because (a) the issue body authored the cross-ref AND (b) the mechanics (prompt enforcing JSON-only) are independently sound, not because #768 shipped. Plan-deploy ordering becomes operator decision (sequence vs. independent ship vs. evidence feedback) — for this plan, operator decision is "ship independently of #768 status; mechanics validated by Phase 0 evidence below."

### Pin 5: Truncation rule-out (per first-pass F1, addressing issue-body's "Possible overlap" #766 cross-ref)

The issue body's "Possible overlap" section names two diagnostic alternatives: #768 (prose-around-JSON) AND #766 (prompt-truncation producing empty responses). First-pass F1 flagged that the original Phase 0 ruled out neither — Unit 1 (parser tolerance) is no-op against the truncation hypothesis. Recovery evidence (operator-side, since `llm_calls.input_tokens` is recorded as 0 across all extraction calls — confirmed token-count recording bug, NOT actual zero input):

**Doc-size sampling — largest files across all extraction corpora:**

| Corpus | Largest file (bytes) | Path |
|---|---|---|
| `mika/docs/solutions` | 22,064 | `best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` |
| `mika-platform/docs/solutions` | 17,350 | `cross-repo-patterns/rust-axum-security-hardening-playbook.md` |
| `mika-skills/docs/solutions` | 9,138 | `integration-issues/...gh-token-env...` |
| `mika-cloud/docs/solutions` | 8,939 | `security-issues/customer-lifecycle-scripts-code-review.md` |

**Token estimate (4 chars/token rough):** worst-case doc ~5,500 input tokens. Plus extraction prompt template + schema (~1K tokens, per `build_extraction_prompt` at `subject_extractor.rs:775` — system prompt with JSON schema + chunk markers) → **total worst-case input ~6,500 tokens**.

**Context-window headroom:**
- `claude-haiku-4-5-20251001`: 200K context → 6.5K input is 3.3% utilization, **96.7% headroom**.
- `openai/gpt-5-nano`: 128K-1M context → 6.5K input is < 5% utilization in any case.

**Truncation hypothesis (#766 cross-ref) is RULED OUT.** No doc anywhere across the active corpora approaches the input-context-window limit. Empty-error-body symptom (`failed to parse extraction JSON: ` with empty body) must therefore be one of:

1. **Prose-around-JSON** (Unit 1 fixes via brace-matching extractor) — most likely per #768 sibling failure shape on same model class.
2. **Empty / whitespace response entirely** (Unit 1 is no-op — brace-matching can't extract from nothing; Unit 2 prompt reinforcement should improve adherence).

Both hypotheses converge on the same Unit 1 + Unit 2 fix shape: parser tolerance (recover when there IS JSON to find) + prompt reinforcement (reduce empty/prose-around emissions at the source). Defense-in-depth applies regardless of which hypothesis dominates the empirical mix.

**Verification chain:** if 24h-post-deploy parse-failure rate drops > 50% but stays > 5%, it's evidence the residual failures are empty-response-entirely (Unit 1 no-op, only Unit 2 reduces incidence). Escalation path: structured outputs (Option 2 deferred), which forces non-empty JSON-shaped output at the provider level.

## Context & Research

### Relevant Code

- **Parser:** `crates/mika-agent/src/kg/subject_extractor.rs:966` (`parse_extraction_json`)
- **Failure path:** `subject_extractor.rs:856-895` (parse → retry-with-reinforcement → parse → final-fail-and-skip)
- **Reinforcement prompt:** `subject_extractor.rs:921` (`retry_with_reinforcement`)
- **Extraction prompt construction:** `subject_extractor.rs:420` (`extract_document`)
- **Tests:** `subject_extractor.rs:1598/1617` (`strips_markdown`, `plain`)

### Sibling Failure Class

- **mika#768** — haiku emits reasoning paragraph after JSON in permission-policy skill. Same model class, same JSON-parse-failure pattern, different subsystem. Diagnosis from #768: prompt doesn't enforce JSON-only output.
- **mika#766** — semantic LLM prompt truncation. If extraction prompts exceeded model context, that could produce empty responses. **Confirmed not the cause here** — server.log error shows `failed to parse extraction JSON: ` with empty body suggesting the parser couldn't lex prose, not that the LLM returned empty bytes.

### Institutional Learnings

- `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — informs why Option 2 (provider-native JSON-mode) is deferred: prompt-level fix is the cheapest first layer; structural escalation only on recurrence.
- `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` — the disconfirmation procedure used for this plan's Problem Frame. Result: premise CONFIRMED (66% pending on mika-arch secondary corpora; 6600 parse-failure events on cumulative log).

## Key Technical Decisions

- **Brace-matching, not regex.** The parser locates the first balanced `{...}` in the text. Brace-matching handles nested objects/arrays correctly; regex on JSON is structurally fragile (`{"key": "string with } in it"}` defeats naive regex). Implementation: scan from left; track brace depth + string-literal state (account for `\"` escapes); the substring at depth-0-after-first-`{` is the candidate. Then `serde_json::from_str(candidate)` — if it parses, return; if not, fall through to the existing C2.2 retry path. **Validation stays strict** — only the surrounding-prose tolerance is added.

- **Prompt reinforcement, not parser leniency on schema.** Unit 2 adds a JSON-only-output instruction to the extraction prompt, mirroring mika#768's recipe. The parser still rejects schema-malformed JSON (per R4) — the change is "tolerate reasoning prose," not "accept partial JSON."

- **No structured outputs adoption (Option 2 deferred).** Anthropic's `tool_use` + OpenAI's `response_format` would force the model to emit valid JSON, but require per-provider plumbing in `crates/mika-common/src/llm/`. Risk: each provider has slightly different JSON-mode semantics (Anthropic supports tool-call JSON via `tool_choice`; OpenAI has `response_format: json_object` which requires the prompt to mention "JSON" explicitly; OpenRouter passes-through with provider-specific behavior). Adoption is a 2-3-day plumbing job for an outcome the parser+prompt fix achieves at low cost. Defer to follow-up ticket if 24h-post-deploy data shows parse-failure rate stays > 5%.

- **No model swap (Option 3 deferred).** Both haiku and gpt-5-nano fail today, so model swap doesn't address the root cause (parser intolerance + prompt under-specification). The right escalation if Option 1+2 fails is structured outputs, not model swap.

- **Test-first parser change.** Unit 3 replays the haiku failure shapes (prose-before-JSON, prose-after-JSON, prose-on-both-sides, code-fences-with-comments) as red tests against the current parser, then green after Unit 1 lands. This both pins the failure class and prevents regression.

## Implementation Units

- [ ] **Unit 1: Brace-matching JSON extraction in `parse_extraction_json`**

  **Goal:** Tolerate reasoning prose before/after the JSON object. Parser becomes prose-tolerant but stays schema-strict.

  **Requirements:** R3, R4, R5

  **Dependencies:** None.

  **Files:**
  - Modify: `crates/mika-agent/src/kg/subject_extractor.rs` — `parse_extraction_json` at line 966, plus inline test additions.

  **Approach:**
  - Add a private helper `extract_first_json_object(text: &str) -> Option<&str>` that scans the text for the first balanced `{...}` substring, accounting for string-literal escapes. Implementation sketch:
    ```rust
    fn extract_first_json_object(text: &str) -> Option<&str> {
        let bytes = text.as_bytes();
        let mut start = None;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape_next = false;
        for (i, &b) in bytes.iter().enumerate() {
            if escape_next { escape_next = false; continue; }
            if in_string {
                if b == b'\\' { escape_next = true; }
                else if b == b'"' { in_string = false; }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => {
                    if depth == 0 { start = Some(i); }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return start.map(|s| &text[s..=i]);
                    }
                }
                _ => {}
            }
        }
        None
    }
    ```
  - Modify `parse_extraction_json` to:
    1. Strip markdown fences (existing behavior, keep).
    2. If `serde_json::from_str(cleaned)` succeeds → return.
    3. Otherwise, call `extract_first_json_object(cleaned)` → if Some, `serde_json::from_str(extracted)` → return.
    4. Otherwise, return the original `serde_json` error (preserves existing C2.2 retry semantics).
  - **R5 verification:** existing tests at line 1598 (`strips_markdown`) and 1617 (`plain`) still pass — both cases hit step 2 (clean JSON parses on first try).

  **Patterns to follow:**
  - The existing `parse_extraction_json` shape (markdown-strip, then parse).
  - Rust's `&str` slice semantics — `extract_first_json_object` returns `Option<&str>` borrowed from input, no allocation.

  **Test expectation:**
  - 4 new unit tests in the existing test module (`subject_extractor.rs:1598+`):
    - `parse_with_reasoning_prefix`: input `"Here is the extraction:\n\n{\"entities\":[...],\"relationships\":[]}"` → parses to `ExtractionOutput`.
    - `parse_with_reasoning_suffix`: input `"{\"entities\":[],\"relationships\":[]}\n\nThe document covers..."` → parses.
    - `parse_with_reasoning_both`: input `"Analysis:\n\n{...}\n\nNotes: ..."` → parses.
    - `parse_with_string_containing_brace`: input `"{\"entities\":[{\"name\":\"foo}\"}],\"relationships\":[]}"` → parses correctly (string literal containing `}` does NOT terminate the object).

  **Verification:**
  - `grep -n 'extract_first_json_object\|parse_extraction_json' subject_extractor.rs` — new helper present, called from `parse_extraction_json`.
  - `cargo test -p mika-agent kg::subject_extractor` — all existing + new tests pass.
  - Replay one of the failing trace_ids from server.log against the new parser using the saved LLM response (if `MIKA_LOG_LLM_BODIES` was on; otherwise just rely on the synthetic test fixtures).

- [ ] **Unit 2: Extraction prompt reinforcement (JSON-only output)**

  **Goal:** Prevent the failure class at the source by instructing the model to emit JSON only — mirrors mika#768's prompt-side intervention pattern.

  **Requirements:** R3 (defense-in-depth), R6, R7

  **Dependencies:** None (orthogonal to Unit 1; both ship in the same PR).

  **Files:**
  - Modify: `crates/mika-agent/src/kg/subject_extractor.rs` — extraction prompt construction inside `extract_document` (line 420). Likely a `format!()` template or a `const SYSTEM_PROMPT: &str` near the function.

  **Approach:**
  - Add a leading instruction line to the prompt: *"Respond with a single JSON object only. Do NOT include any explanatory prose, markdown headers, code fences, reasoning, or summary text before or after the JSON. Your entire response must be parseable as JSON via `serde_json::from_str` from the first to the last byte. Any reasoning or notes belong inside JSON fields, not around them."*
  - Position the instruction at the TOP of the prompt (after the role declaration, before the schema description) so it's the first thing the model sees and structurally precedes the schema explanation. This mirrors the discovery from mika#864 that early-prompt instructions hold better than trailing ones.

  **Patterns to follow:**
  - mika#768's diagnosis: the prompt didn't enforce JSON-only. The fix recipe (verbatim from #768): *"clarify that the JSON is the ONLY acceptable output."*
  - Existing prompt structure in the same file (need to read at implementation time).

  **Test expectation:**
  - No unit test for prompt content (prompts are not behaviorally testable in unit-test scope; verification is empirical via the post-deploy 24h window).
  - Verification deferred to deploy + observation per ticket AC R1/R2.

  **Verification:**
  - Diff review confirms the JSON-only instruction is present at the prompt top.
  - 24h post-deploy: parse-failure rate trends to < 5% per Signal (see "Acceptance" below).

- [ ] **Unit 3: Behavioral test fixtures replaying haiku failure shapes**

  **Goal:** Pin the failure class as red tests against the pre-fix parser; green after Unit 1. Prevents regression.

  **Requirements:** R3, R5, R7

  **Dependencies:** Unit 1 (the parser changes Unit 3 verifies).

  **Files:**
  - Modify: `crates/mika-agent/src/kg/subject_extractor.rs` test module (line 1598+).
  - Optional: Add fixtures in `crates/mika-agent/tests/eval/grounding_regressions/fixtures/` if the existing eval harness fits — but per `crates/mika-agent/CLAUDE.md` § Evaluation — Grounding Regressions, that surface is for response-to-evidence path. Parse-tolerance is closer to unit-test scope; keep tests inline in `subject_extractor.rs`.

  **Approach:**
  - Add the 4 unit tests described in Unit 1's "Test expectation" plus one regression-shape test:
    - `parse_strict_validation_still_rejects_invalid_schema`: input with prose + JSON missing required field → returns Err (not silently dropped). Confirms R4 (validation stays strict).
  - Add one negative-case test:
    - `parse_returns_none_when_no_balanced_braces`: input `"This is just prose, no JSON here."` → `extract_first_json_object` returns None, `parse_extraction_json` returns Err with original parse error (preserves C2.2 retry semantics).

  **Patterns to follow:**
  - Existing test style at `subject_extractor.rs:1598`.
  - Use `Result<ExtractionOutput, _>` matching from existing tests, not `unwrap()`.

  **Test expectation:** 5 new tests added; 0 existing tests broken.

  **Verification:** `cargo test -p mika-agent kg::subject_extractor::tests` reports 5 new tests passing.

## System-Wide Impact

- **Interaction graph:** Unit 1 modifies `parse_extraction_json`'s tolerance. Callers at line 856 + 878 + 941 (one initial parse + two retry-paths) all use the same function — they all gain the same tolerance.
- **Error propagation:** When `extract_first_json_object` returns None AND clean parse fails, `parse_extraction_json` returns the original `serde_json` error. Existing C2.2 semantic retry path triggers as before. R4 + retry semantics unchanged.
- **State lifecycle risks:** None. No schema changes. No DB writes change shape. No new tables.
- **API surface parity:** No new public APIs.
- **Unchanged invariants:** Stage-1 exact-match path (mika#875 disconfirmation), resolver behavior (mika#874), budget enforcement, retry taxonomy, sole-writer contracts on `kg_subject_entities` / `kg_subject_relationships`, MIKA_KG_BATCH_BUDGET semantics.
- **Observability:** Existing `extraction_parse_failed_retry` and `extraction_semantic_exhausted` events continue to fire on genuine failures. Their volume should drop dramatically post-deploy as the dominant failure class (prose-around-JSON) is now handled at parse-time.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Brace-matching extracts a `{}` that's not the actual extraction output (e.g., model emits an example JSON before the real one). | The model is asked for ONE extraction output per document; expecting two top-level objects is unlikely. If observed in practice, escalate to provider-native structured outputs (Option 2 deferred). |
| Prompt reinforcement (Unit 2) doesn't stick — model continues to emit prose. | Unit 1's parser tolerance is the safety net. The combination should converge on > 80% success rate even if Unit 2 has partial adherence. |
| Per-provider variance: gpt-5-nano (openrouter) might fail differently from haiku (anthropic). | The brace-matching extractor is provider-agnostic. Both providers should benefit. If gpt-5-nano has a different failure shape (e.g., truncation rather than prose-around), Unit 3's regression tests will catch it during empirical verification. |
| 24h post-deploy parse-failure rate doesn't reach < 5% threshold. | Escalate to Option 2 (structured outputs) per `engine-guards-vs-prompt-rules` precedent. File follow-up ticket. The parser-tolerance work isn't wasted — it's defense-in-depth even with structured outputs (which can themselves fail in edge cases). |
| New test fixtures embed PII or secrets. | The fixture text is synthetic ("Here is the extraction: {...}"); contains no real LLM outputs. R-controlled. |

## Acceptance

Per ticket AC: routine batches over the existing corpora produce `entities > 0` for ≥ 80% of docs over a 24h window post-deploy.

**Verification path:**

1. **Unit-test gate (CI-time):** `cargo test -p mika-agent kg::subject_extractor::tests` — 5 new tests pass; existing `strips_markdown` + `plain` continue to pass.
2. **Build verification:** None required — the change is library-code; behavioral verification is empirical post-deploy.
3. **Empirical signal (post-deploy, 24h window):**
   - **Signal F (parse-failure-rate):** `grep -c "extraction_parse_failed_retry" /var/log/mika/server.log` per agent per day. Pre-fix: 6600 cumulative, ~70% of attempts. Target post-fix: < 5% of attempts (i.e., < 50/day on the active corpora).
   - **Signal G (entities-extracted distribution):** `SELECT date(created_at), AVG(entities_extracted), COUNT(*) FROM kg_extractions GROUP BY date(created_at)`. Target: avg entities/doc > 1, > 80% of new rows have `entities_extracted > 0`.
   - **Signal H (pending-docs trend):** mika-arch secondary corpora pending-doc count trends to 0 across 2-3 restart cycles. Pre-fix: 33/22/15 pending across 3 corpora. Target post-fix: drops by > 50% on first restart, asymptotes to 0 over 3 restarts.

**Verification is empirical — no in-repo behavioral test surface for "the LLM emits prose-around-JSON" without a live provider call.** The test fixtures (Unit 3) verify the parser's tolerance against synthetic inputs; the actual prose-emission rate is provider-dependent and tracked via the post-deploy signals above.

## Future Work (deferred per `engine-guards-vs-prompt-rules` precedent)

- **Provider-native structured outputs** (Option 2 from ticket). Adopt if 24h-post-deploy data shows parse-failure rate > 5%. Implementation cost: 2-3 days of per-provider plumbing in `crates/mika-common/src/llm/`.
- **Per-model success-rate calibration + routing** (Option 3 from ticket). Defer indefinitely — both haiku and gpt-5-nano fail today, so model swap doesn't address the root cause. If, post-fix, one model converges faster than the other, the existing `MIKA_KG_EXTRACTION_MODEL` env var already supports per-deploy override.

## Sources & References

- Related issue: mika#876
- Sibling failure class: mika#768 (haiku reasoning-prose-around-JSON in permission-policy)
- Sibling milestone tickets: mika#874, mika#875 (closed-as-not-a-bug), mika#906 (shipped 2026-04-30), mika#877 (pending groom)
- Code references:
  - `crates/mika-agent/src/kg/subject_extractor.rs:966` — `parse_extraction_json` (Pin 1)
  - `subject_extractor.rs:856-895` — failure path (Pin 2)
  - `subject_extractor.rs:420` — `extract_document` (Pin 3, prompt construction site)
  - `subject_extractor.rs:1598/1617` — existing parser tests
- Documentation:
  - `crates/mika-agent/CLAUDE.md` § Knowledge Graph — Subject Extractor
  - `crates/mika-agent/CLAUDE.md` § Pending-doc detection (D7)
- Institutional learnings:
  - `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` (informs Option 1 choice + Option 2 deferral)
  - `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` (the procedure used for Phase 0 premise validation)
