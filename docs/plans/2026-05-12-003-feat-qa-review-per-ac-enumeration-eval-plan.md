# Plan: Grounding-Regression Eval Harness for qa-review Per-AC Enumeration

**Issue:** mika issue#1059
**Type:** feat
**Branch:** `feat/1059/eval-grounding-regression-eval-harness`

## Context

Follow-up from `mika-skills#159` which shipped per-element enumeration + absence-claim grounding rules in the qa-review skill prompt (`system_prompt.md` Step 2.5.5). This ticket adds eval harness tests to regression-guard those two rules.

## Pinned Sources (F1 — mika-arch first-pass)

### Source 1: Verbatim prompt rules from qa-review `system_prompt.md` Step 2.5.5

**Per-element enumeration (mandatory when AC contains multi-element thresholds):**

> When an AC bullet asserts a condition over a set of elements (e.g., "X% for all N corpora", "field present in all M responses", "no regressions in N tests"), the verdict MUST:
>
> 1. **Enumerate every element by name** with its observed value. Never aggregate into a single claim like "all N elements pass/fail".
> 2. **State per-element pass/fail** using the AC's threshold: `<element>: <observed_value> → [✓ pass | ✗ fail]`
> 3. **Quote the source** when asserting presence/absence. Before claiming "section X absent", quote the heading you searched for. If the heading exists but content is disputed, quote the actual content.

**Quote-based grounding for absence claims:**

> When the verdict asserts that content is absent (e.g., "R5 section missing", "no test coverage for X"):
>
> 1. State the exact heading/marker you searched for.
> 2. If found: quote the first 2 lines of content under that heading.
> 3. If not found: state `searched for "<heading text>" — not present in PR body sections: <list of actual section headings found>`.

### Source 2: Exemplar scenario — `auto_merge_vs_merged.rs` (scenario 2)

Pattern: `EvalHarness::builder().responses(vec![...]).build().await?` → `harness.run("prompt").await?` → hard assertions via `grounding_assertions::*`.

```rust
// Primary test: mock produces correct response
#[tokio::test]
async fn test_auto_merge_not_claimed_as_merged() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_gh", json!({...})),
            text_response("PR #735 is currently open with auto-merge enabled..."),
        ])
        .build().await?;
    let trace = harness.run("Is PR 735 merged?").await?;
    grounding_assertions::assert_response_forbids(&trace, FORBIDDEN_COMPLETION_WORDS);
    Ok(())
}

// Regression test: mock produces pre-fix response, assertion framework catches it
#[tokio::test]
async fn test_regression_auto_merge_falsely_claimed_merged() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("run_gh", json!({...})),
            text_response("PR #735 has been merged..."),  // pre-fix
        ])
        .build().await?;
    let trace = harness.run("Is PR 735 merged?").await?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_forbids(&trace, FORBIDDEN_COMPLETION_WORDS);
    }));
    assert!(result.is_err(), "Pre-fix regression: should have caught...");
    Ok(())
}
```

### Source 3: Existing `grounding_assertions/mod.rs` exports

Current helpers (all take `&AgentTrace` + scenario-specific params, panic on failure):

| Helper | Signature | Purpose |
|--------|-----------|---------|
| `assert_response_forbids` | `(trace, forbidden: &[&str])` | Response must NOT contain any forbidden words |
| `assert_tool_called_before_response` | `(trace, tool_name: &str)` | Named tool must have been called |
| `assert_any_tool_called_from` | `(trace, tool_names: &[&str])` | At least one tool from set must have been called |
| `assert_response_contains_in_order` | `(trace, items: &[&str])` | Items appear in monotonically increasing positions |
| `assert_response_contains_question` | `(trace)` | Response contains `?` |
| `assert_response_contains` | `(trace, expected: &str)` | Case-insensitive substring match |

New helpers follow this exact convention: `pub fn`, `&AgentTrace` first param, panic with descriptive message on failure.

### Source 4: Module structure (scenario numbering convention)

`mod.rs` lists 12 module entries. Multiple `#[tokio::test]` functions per module. The README numbers scenarios 1–18 by counting test functions across modules. New scenarios continue the README numbering (19, 20) with one module per scenario pair (primary + regression).

**Rules under test (paraphrased from pinned Source 1):**

1. **Per-element enumeration:** When an AC asserts a condition over a set of elements, the verdict MUST enumerate every element by name with observed value and per-element pass/fail. Never aggregate ("all N pass/fail").
2. **Quote-based absence grounding:** When the verdict asserts content is absent, state the exact heading searched for and list actual section headings found.

## Design Decisions

### D1: Test shape — mock-LLM response assertions, not full skill integration

The existing grounding regression harness (`tests/eval/grounding_regressions/`) uses `EvalHarness` with `MockLlmProvider` to test response-shape correctness. The qa-review per-AC enumeration rules are response-shape rules — they constrain the output text, not tool invocation. The correct test shape is:

- Mock the LLM to produce a **correct** per-element enumeration response → assert it contains the required structural markers
- Mock the LLM to produce an **incorrect** aggregated response (pre-fix fixture) → assert the assertion framework catches it

This follows the existing two-part pattern (primary + regression-reproduction) from scenarios 1–18.

### D2: New assertion helper — `assert_response_contains_per_element_enumeration`

The existing `grounding_assertions` helpers don't have a shape that checks for per-element enumeration structure. We need a new helper:

```rust
pub fn assert_response_contains_per_element_enumeration(
    trace: &AgentTrace,
    elements: &[&str],  // expected element names (e.g., ["mika primary", "mika-skills", ...])
)
```

This asserts that:
- Each named element appears in the response
- Each element is followed by a pass/fail indicator (`✓` or `✗`, or the words `pass`/`fail`)

This is intentionally loose on formatting (doesn't require exact `→` arrows or checkmark style) but strict on the structural requirement: every element must be named individually.

### D3: New assertion helper — `assert_absence_claim_grounded`

For the quote-based absence grounding rule:

```rust
pub fn assert_absence_claim_grounded(
    trace: &AgentTrace,
    searched_heading: &str,  // the heading that was searched for
)
```

This asserts that when the response claims something is absent, it includes:
- The searched heading text
- A list of actual headings found (indicated by phrases like "sections:" or "headings found:")

Absence-claim detection uses keyword matching: "not present", "missing", "absent", "could not find", "does not appear", "no section". This keyword list is intentionally conservative and should be extended when new absence phrasings are observed in real LLM outputs (F5 — mika-arch first-pass).

### D4: Scenario structure — two scenarios (19, 20)

Following README conventions:

| # | Scenario | Assertion types | Tags |
|---|----------|-----------------|------|
| 19 | `qa_review_per_element_enumeration` | per-element-enumeration, contains | `grounding:per-element-enumeration-correct` (success), `grounding:aggregate-claim-suppressed` (success) |
| 20 | `qa_review_absence_claim_grounded` | absence-grounded, contains | `grounding:absence-claim-grounded` (success), `grounding:absence-claimed-without-evidence` (failure) |

### D5: Tag vocabulary additions

New tags in the `grounding:*` namespace:

| Tag | Trigger condition | Type |
|-----|-------------------|------|
| `grounding:per-element-enumeration-correct` | Agent correctly enumerated each element by name with per-element pass/fail | Success |
| `grounding:aggregate-claim-suppressed` | Agent correctly avoided aggregating multi-element AC into a single claim | Success |
| `grounding:absence-claim-grounded` | Agent correctly grounded absence claim with searched heading + actual headings | Success |
| `grounding:absence-claimed-without-evidence` | Agent claimed absence without quoting the searched heading or listing found headings | Failure |

### D6: Frozen fixtures with verbatim text and provenance (F2 — mika-arch first-pass)

Two pre-fix fixtures under `fixtures/`, plus the primary test mock responses. All four are reconstructions from the documented failure patterns in the qa-review prompt's Step 2.5.5 examples — the prompt itself includes both the correct and WRONG patterns, making the fixture provenance direct.

**Fixture 1 — Primary (scenario 19): correct per-element enumeration**

Provenance: Faithful reconstruction of the "Example (correct)" block in `system_prompt.md` Step 2.5.5 line 184–192.

```
PLAN-AC VERIFICATION:

- [❌] unsatisfied: coverage ≥50% for all 4 corpora
  - mika primary: 70.8% → ✓ pass
  - mika-skills: 52.9% → ✓ pass
  - mika-platform: 47.9% → ✗ fail (below 50%)
  - mika-cloud: 31.2% → ✗ fail (below 50%)
  Result: 2/4 pass, 2/4 fail — AC unsatisfied

VERDICT: block[ac]
```

**Fixture 2 — Regression (scenario 19): aggregated claim (pre-fix)**

Provenance: Faithful reconstruction of the "Example (WRONG)" block in `system_prompt.md` Step 2.5.5 line 195–197. This is the exact failure mode the prompt rule was added to prevent.

```
PLAN-AC VERIFICATION:

- [❌] unsatisfied: coverage ≥50% for all 4 corpora — all 4 below threshold

VERDICT: block[ac]
```

**Fixture 3 — Primary (scenario 20): grounded absence claim**

Provenance: Faithful reconstruction of rule 3 in the "Quote-based grounding for absence claims" block in `system_prompt.md` Step 2.5.5 line 199–206.

```
PLAN-AC VERIFICATION:

- [❌] unsatisfied: R5 — Rollback procedure documented in PR body
  searched for "## R5 — Rollback procedure" — not present in PR body sections: Summary, Test plan, Breaking changes, Migration steps

VERDICT: block[ac]
```

**Fixture 4 — Regression (scenario 20): ungrounded absence claim (pre-fix)**

Provenance: Reconstruction of the scan-and-miss failure mode described in `system_prompt.md` Step 2.5.5 line 207 ("This prevents the scan-and-miss failure mode where the LLM asserts absence without actually verifying").

```
PLAN-AC VERIFICATION:

- [❌] unsatisfied: R5 — Rollback procedure documented in PR body — R5 section missing

VERDICT: block[ac]
```

Frozen fixture JSON files (`fixtures/*.json`) wrap these in the standard `{"text": "..."}` format used by `MockLlmProvider::text_response()`.

### D7: No new EvalHarness changes needed

The existing `EvalHarness::builder().responses(vec![...]).build()` pattern is sufficient. These scenarios don't need skill context injection, KG fixtures, or special tool configurations. They test response text shape, which is fully controlled by the mock LLM responses.

## Implementation Steps

### Step 1: Add assertion helpers to `grounding_assertions/mod.rs`

Add two new helpers:
- `assert_response_contains_per_element_enumeration(trace, elements)` — checks each element name appears with a pass/fail indicator
- `assert_absence_claim_grounded(trace, searched_heading)` — checks absence claims include the searched heading and a list of actual headings

Add unit tests for both in the `#[cfg(test)]` module.

### Step 2: Create scenario 19 — `qa_review_per_element_enumeration.rs`

File: `crates/mika-agent/tests/eval/grounding_regressions/qa_review_per_element_enumeration.rs`

**Primary test:** Mock LLM produces a correct per-element enumeration verdict for "coverage ≥50% for all 4 corpora" with mixed pass/fail:
```
- [❌] unsatisfied: coverage ≥50% for all 4 corpora
  - mika primary: 70.8% → ✓ pass
  - mika-skills: 52.9% → ✓ pass
  - mika-platform: 47.9% → ✗ fail (below 50%)
  - mika-cloud: 31.2% → ✗ fail (below 50%)
  Result: 2/4 pass, 2/4 fail — AC unsatisfied
```

Assertions:
- `assert_response_contains_per_element_enumeration` with all 4 corpus names
- `assert_response_contains` for "2/4 pass" (result summary)

**Regression test:** Mock LLM produces the pre-fix aggregated response ("all 4 corpora below threshold"). Assert `assert_response_contains_per_element_enumeration` panics.

### Step 3: Create scenario 20 — `qa_review_absence_claim_grounded.rs`

File: `crates/mika-agent/tests/eval/grounding_regressions/qa_review_absence_claim_grounded.rs`

**Primary test:** Mock LLM produces a grounded absence claim:
```
searched for "## R5 — Rollback procedure" — not present in PR body sections: Summary, Test plan, Breaking changes, Migration steps
```

Assertions:
- `assert_absence_claim_grounded` with "R5" or "Rollback procedure"
- `assert_response_contains` for "not present"

**Regression test:** Mock LLM produces ungrounded "R5 section missing" without evidence. Assert `assert_absence_claim_grounded` panics.

### Step 4: Create frozen fixtures

- `fixtures/qa_review_per_element_enumeration_pre_fix.json`
- `fixtures/qa_review_absence_claim_grounded_pre_fix.json`

### Step 5: Register modules in `mod.rs`

Add to `crates/mika-agent/tests/eval/grounding_regressions/mod.rs`:
```rust
pub mod qa_review_per_element_enumeration;
pub mod qa_review_absence_claim_grounded;
```

### Step 6: Update README capability matrix

Add scenarios 19–20 to the capability × status matrix and tag vocabulary in `grounding_regressions/README.md`.

## Acceptance Criteria Mapping

- [x] Test fixture exercises the per-element enumeration rule → Step 2 primary test
- [x] Test asserts correct element-by-element output shape → Step 2 `assert_response_contains_per_element_enumeration`
- [x] Test fails if the LLM aggregates instead of enumerating → Step 2 regression test

## Risk Assessment

**Low risk.** Additive test files only — no production code changes. Follows established patterns from scenarios 1–18. No new dependencies.

## References

- Plan: `mika-skills` branch `fix/159/qa-review-per-ac-enumeration-per-corpus` @ `docs/plans/2026-05-10-001-fix-qa-review-per-ac-enumeration-per-corpus-plan.md`
- Prompt: `mika/skills/bundled/qa-review/system_prompt.md` Step 2.5.5
- Existing grounding regression tests: `crates/mika-agent/tests/eval/grounding_regressions/`
- Grounding assertions: `crates/mika-agent/tests/eval/grounding_assertions/mod.rs`
