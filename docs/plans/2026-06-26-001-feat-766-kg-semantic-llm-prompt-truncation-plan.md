# Plan: Semantic LLM Prompt Truncation for KG (#766)

**Ticket:** mika#766
**Type:** feat (enhancement)
**Branch:** `feat/766/kg-semantic-llm-prompt-truncation`

## Problem

The KG resolver and extractor truncate prose context for LLM prompts using fixed byte budgets via `safe_truncate(s, max_bytes)`. Two correctness concerns:

1. **Byte budgets ≠ token budgets.** A 2000-byte budget yields ~500–700 tokens for em-dash-heavy docs but ~2000 tokens for pure ASCII. The budget exists to bound prompt size, but the unit is wrong.
2. **Mid-sentence truncation** produces dangling clauses (`"...the CI failure was caused by state drift between the"`), forcing the LLM to guess whether truncated content mattered — a subtle extraction/resolution quality degradation.

## Scope

The ticket prescribes an empirical-first approach: **measure first, implement only if quality impact is measurable.** This plan covers all three steps.

## Current Truncation Sites (Audit)

Six `safe_truncate` call sites exist in the KG subsystem. Three are LLM-prompt-bound (in-scope); two are error-log-bound (out-of-scope); one is the full-doc extraction path (no truncation).

### In-Scope (LLM-prompt-bound)

| Site | File | Line | Budget | Purpose |
|------|------|------|--------|---------|
| **R1** | `entity_resolver.rs` | 1794 | 2000 bytes | `chunk_context` in disambiguation prompt — the primary quality concern |
| **R2** | `entity_resolver.rs` | 1588 | 500 bytes | `bad_output` in retry reinforcement prompt |
| **R3** | `subject_extractor.rs` | 1357 | 500 bytes | `bad_output` in retry reinforcement prompt |

### Out-of-Scope (error-log-bound)

| Site | File | Line | Budget | Purpose |
|------|------|------|--------|---------|
| L1 | `entity_resolver.rs` | 1756 | 200 bytes | Error context for JSON parse failure log |
| L2 | `subject_extractor.rs` | 1450 | 200 bytes | Error context for JSON parse failure log |

### Not Truncated

| Site | File | Purpose |
|------|------|---------|
| E1 | `subject_extractor.rs:1221` | Full annotated document sent to LLM — no explicit truncation |

**Priority:** R1 is the highest-impact site (2000-byte chunk context directly affects disambiguation quality). R2/R3 are retry prompts where truncation of bad output is less quality-sensitive (the LLM just needs to see enough of its prior mistake to correct it). Implementation focuses on R1; R2/R3 get the same treatment at negligible marginal cost.

## Existing Infrastructure

- **`safe_truncate(s, max_bytes) -> &str`** in `mika-common::text` — byte-budget truncation with `floor_char_boundary`. UTF-8 safe but not sentence-aware.
- **`truncate_to_token_budget(summary, max_tokens) -> String`** in `mika-agent::prompt` — token-estimated truncation (4 chars/token heuristic) with word-boundary awareness. Appends a truncation marker. Used for summary injection, NOT for KG.
- **KG provider eval harness** at `tests/eval/kg_provider_eval/` — direct LLM calls with production KG prompts. 15 extraction sample docs + 30 hand-labeled resolution ground-truth cases. Reports per-provider quality/cost/latency.

## Implementation

### Step 1: Quantify — Truncation Quality Comparison Eval

**Goal:** Measure whether sentence-boundary truncation produces measurably better entity resolution quality than byte truncation, using the existing KG eval infrastructure.

#### 1.1 Add `truncate_at_semantic_boundary` to `mika-common::text`

```rust
/// Truncate prose to at most `max_bytes`, preferring to end at the last
/// sentence boundary (`.` `!` `?` or `\n`) within the budget.
/// Falls back to `safe_truncate` (char-boundary) if no sentence boundary exists.
///
/// Use for LLM-prompt-bound truncation where mid-sentence cuts degrade
/// prompt quality. For log lines and error previews, use `safe_truncate`.
pub fn truncate_at_semantic_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let limit = s.floor_char_boundary(s.len().min(max_bytes));
    // Scan backwards for sentence-ending punctuation
    if let Some(end) = s[..limit]
        .rfind(|c: char| matches!(c, '.' | '!' | '?' | '\n'))
    {
        // Include the sentence-ending character
        let boundary = end + s[end..].chars().next().map_or(0, |c| c.len_utf8());
        if boundary > 0 {
            return &s[..boundary];
        }
    }
    // Fallback: char boundary (same as safe_truncate)
    safe_truncate(s, max_bytes)
}
```

**Files:** `crates/mika-common/src/text.rs`

**Tests:** Unit tests for:
- String shorter than limit (no-op)
- String longer with sentence boundary within budget → truncates at boundary
- String longer with no sentence boundary → falls back to `safe_truncate`
- Sentence boundary at various positions (period, newline, exclamation)
- Multi-byte characters near boundary
- Empty string, zero budget

#### 1.2 Add truncation-comparison eval to `tests/eval/kg_provider_eval/`

Create `truncation_eval.rs` as a new eval module alongside `extraction_eval.rs` and `resolution_eval.rs`. This eval:

1. Loads the 30 resolution ground-truth cases from `resolution_ground_truth.toml`
2. For cases where `chunk_context.len() > 2000` (or artificially, by lowering the budget to 500 bytes to ensure most cases trigger truncation), runs disambiguation twice per case:
   - **Variant A:** `safe_truncate(chunk_context, budget)` (current behavior)
   - **Variant B:** `truncate_at_semantic_boundary(chunk_context, budget)` (proposed)
3. Calls the LLM with the same disambiguation prompt structure as `build_disambiguation_prompt`
4. Compares accuracy against ground truth for both variants
5. Reports per-case delta: matched-correct, confidence delta, and any flipped outcomes (A wrong → B right, or vice versa)

**Gate:** Use `MIKA_EVAL_KG_PROVIDERS` gating (existing). Add a `truncation_eval` test function alongside the existing `kg_provider_eval` function.

**Fixture extension:** The existing `resolution_ground_truth.toml` cases have short `chunk_context` strings (typically < 200 bytes). For meaningful truncation testing, add 10 extended-context cases with `chunk_context` > 2000 bytes drawn from real `docs/solutions/` documents. These cases go in a new file `truncation_eval_contexts.toml` to avoid perturbing the existing fixture.

**Files:**
- `tests/eval/kg_provider_eval/truncation_eval.rs` (new)
- `tests/eval/kg_provider_eval/mod.rs` (register new module)
- `docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml` (new fixture)

#### 1.3 Run eval and capture results

Run the truncation eval with the default provider set. Capture the comparison artifact.

**Expected artifact:** A compound doc at `docs/solutions/kg/truncation-quality-comparison-2026-06-26.md` documenting:
- Per-case outcomes for both variants
- Aggregate accuracy (byte vs semantic)
- Qualitative observations (dangling clauses, hallucinated entities)
- Decision: proceed to implementation or close-as-measured

### Step 2: Decision Gate

If the eval shows **indistinguishable quality** (same accuracy, no flipped outcomes):
- Write the comparison doc with "won't implement" decision
- Close the ticket with a reference to the doc

If the eval shows **measurable improvement** (any flipped outcomes where semantic wins, or confidence improvement > 0.05 across cases):
- Proceed to Step 3

### Step 3: Apply Semantic Truncation to LLM-Prompt Sites

#### 3.1 Replace `safe_truncate` at R1 (entity resolver disambiguation)

**File:** `crates/mika-agent/src/kg/entity_resolver.rs`
**Line:** 1794

```rust
// Before:
mika_common::text::safe_truncate(chunk_context, 2000)
// After:
mika_common::text::truncate_at_semantic_boundary(chunk_context, 2000)
```

#### 3.2 Replace `safe_truncate` at R2 and R3 (retry prompts)

**File:** `crates/mika-agent/src/kg/entity_resolver.rs:1588`
**File:** `crates/mika-agent/src/kg/subject_extractor.rs:1357`

```rust
// Before:
mika_common::text::safe_truncate(bad_output, 500)
// After:
mika_common::text::truncate_at_semantic_boundary(bad_output, 500)
```

**Rationale for R2/R3:** Even though retry prompts are less quality-sensitive, the cost of using semantic truncation is zero (same function, same budget). Consistent behavior across all LLM-prompt-bound sites.

#### 3.3 Do NOT change L1/L2 (error-log sites)

Error log truncations stay on `safe_truncate` — these are byte-bound for log-line-width reasons, not LLM quality. Per ticket: "The error-log truncations stay on `safe_truncate`."

#### 3.4 Do NOT add token-counting dependency

The ticket's acceptance criteria explicitly scope out token-counting tokenizer dependencies unless Step 2 justifies it. The sentence-boundary approach achieves the goal (no dangling clauses) without requiring a tokenizer. Token-aware truncation is a separate enhancement if needed.

### Step 4: Tests

#### 4.1 Unit tests for `truncate_at_semantic_boundary`

Already covered in Step 1.1.

#### 4.2 Integration test for disambiguation with semantic truncation

Add one scenario to `tests/eval/grounding_regressions/` or a dedicated test that verifies the disambiguation prompt is well-formed when using semantic truncation — specifically that the truncated context ends at a sentence boundary when one exists within budget.

**File:** `crates/mika-agent/src/kg/entity_resolver.rs` (inline `#[cfg(test)]` module)

Test `build_disambiguation_prompt` with a chunk_context that would produce a mid-sentence cut under byte truncation and verify it ends at a sentence boundary under the new function.

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-common/src/text.rs` | Add `truncate_at_semantic_boundary` + tests |
| `crates/mika-agent/src/kg/entity_resolver.rs` | Replace `safe_truncate` with `truncate_at_semantic_boundary` at lines 1794 and 1588 |
| `crates/mika-agent/src/kg/subject_extractor.rs` | Replace `safe_truncate` with `truncate_at_semantic_boundary` at line 1357 |
| `tests/eval/kg_provider_eval/truncation_eval.rs` | New: truncation quality comparison eval |
| `tests/eval/kg_provider_eval/mod.rs` | Register `truncation_eval` module |
| `docs/solutions/kg/eval-fixtures-2026-04-24/truncation_eval_contexts.toml` | New: extended-context test cases |
| `docs/solutions/kg/truncation-quality-comparison-*.md` | New: comparison artifact (compound doc) |

## Risks and Mitigations

1. **Semantic boundary too aggressive** — if the last sentence boundary is at byte 50 of a 2000-byte budget, we lose 97.5% of available context. Mitigation: fall back to `safe_truncate` when the sentence boundary is too far from the budget (e.g., < 50% of budget utilized). This is a tuning knob, not a blocker.

2. **Newline as sentence boundary** — markdown docs use newlines within sentences (line wrapping). A newline-terminated truncation might still be mid-sentence in the prose sense. Mitigation: prefer `.`/`!`/`?` over `\n` when both are available within the last ~200 bytes of the budget. Implementation: scan for punctuation first; only use newline if no punctuation found.

3. **No measurable quality difference** — the ticket explicitly accounts for this outcome. If Step 1 shows no delta, the ticket closes with a documented decision and the `truncate_at_semantic_boundary` function stays in the codebase (zero cost, useful for future callers).

## Acceptance Criteria (from ticket)

- [ ] Step 1 produces a written quality comparison artifact (compound doc under `docs/solutions/kg/`)
- [ ] Step 2 decision documented with the comparison evidence
- [ ] If implementing: `truncate_at_semantic_boundary` helper + tests + applied to LLM-prompt-bound sites
- [ ] If not implementing: ticket closed with a "won't do, measured" note and a link to the comparison doc

## Out of Scope

- Token-counting tokenizer dependency (unless Step 2 justifies it)
- Truncation of already-tokenized inputs
- Error log truncations (stay on `safe_truncate`)
- Changes to the full-document extraction path (E1 — no truncation applied today)
