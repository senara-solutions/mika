---
title: Grounding eval harness for per-element enumeration and absence-claim rules
date: 2026-05-12
category: best-practices
module: eval-harness
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Adding new grounding assertion helpers to the eval harness
  - Testing response-shape rules that constrain LLM output text structure
  - Regression-guarding prompt rules with frozen fixture tests
tags:
  - eval-harness
  - grounding-assertions
  - per-element-enumeration
  - absence-claim
  - qa-review
  - frozen-fixtures
---

# Grounding eval harness for per-element enumeration and absence-claim rules

## Context

The qa-review skill prompt (Step 2.5.5) added two response-shape rules after mika-skills#159: (1) per-element enumeration — when an AC asserts a condition over a set of elements, the verdict must enumerate every element by name with per-element pass/fail, never aggregate; (2) quote-based absence grounding — absence claims must include the searched heading and list actual headings found. These rules constrain LLM output text shape, not tool invocation, making them candidates for mock-LLM response-shape assertion testing rather than full agent loop integration tests.

## Guidance

### Assertion helper design

New grounding assertion helpers follow the existing convention: `pub fn`, `&AgentTrace` first param, panic with descriptive message on failure. Two patterns emerged:

**Per-element enumeration checker** — operates entirely on the lowercased response text to avoid byte-offset mismatches between `text` and `text.to_lowercase()` when `to_lowercase()` expands multi-byte characters. Uses a 200-char lookahead window after each element name to find pass/fail indicators (`✓`, `✗`, `pass`, `fail`). Collects both missing-element and missing-indicator failures before panicking with a single descriptive message.

**Absence-claim grounding checker** — two-phase: (1) detect absence-claim keywords in the response, (2) if detected, verify the searched heading and evidence markers are present. The keyword list is intentionally conservative (`not present`, `missing`, `absent`, `could not find`, `does not appear`, `no section`) — extend when new phrasings are observed in real LLM outputs. No-op when no absence claim is detected.

### UTF-8 safety when windowing response text

When slicing a window from response text for indicator search, always operate on the same string instance used for `find()`. Searching `lower` (the `to_lowercase()` result) but slicing `text` (the original) creates a byte-offset mismatch: `to_lowercase()` can expand byte length for certain Unicode characters (e.g., German ß → ss), so a byte position in `lower` may not be a valid char boundary in `text`.

```rust
// WRONG — byte-offset mismatch between lower and text
let pos = lower.find(&element_lower)?;
let window = &text[pos..pos + 200]; // may panic on multi-byte

// CORRECT — operate entirely on lower
let pos = lower.find(&element_lower)?;
let window = &lower[pos..end]; // safe, same string
```

### Frozen fixture provenance

Each scenario has a `fixtures/{scenario}_pre_fix.json` with the pre-fix response that demonstrates the failure class. The fixture JSON includes a `provenance` field citing the exact source (e.g., "Faithful reconstruction of the 'Example (WRONG)' block in system_prompt.md Step 2.5.5 line 195-197"). This makes it clear the fixture is not invented but reconstructed from documented failure patterns.

## Why This Matters

Response-shape rules in skill prompts are fragile — they depend on the LLM following structural constraints. Without eval harness tests, regressions are only caught by human reviewers noticing that a qa-review verdict aggregated instead of enumerating. The two-part test pattern (primary + regression-reproduction) proves both that the assertion catches the failure class and that correct responses pass.

## When to Apply

- When a skill prompt adds structural constraints on LLM output text (enumeration rules, quoting rules, format requirements)
- When the constraint is objectively checkable without an LLM judge (pattern matching, substring presence)
- When the failure mode has been observed in production and documented in the prompt's examples

## Examples

The two-part test pattern for scenario 19 (per-element enumeration):

```rust
// Primary: correct per-element enumeration passes
#[tokio::test]
async fn test_per_element_enumeration_correct() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("...per-element verdict...")])
        .build().await?;
    let trace = harness.run("Review PR").await?;
    grounding_assertions::assert_response_contains_per_element_enumeration(
        &trace, &["mika primary", "mika-skills", "mika-platform", "mika-cloud"],
    );
    Ok(())
}

// Regression: aggregated claim is caught
#[tokio::test]
async fn test_regression_aggregate() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("...all 4 below threshold...")])
        .build().await?;
    let trace = harness.run("Review PR").await?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_contains_per_element_enumeration(
            &trace, &["mika primary", "mika-skills", "mika-platform", "mika-cloud"],
        );
    }));
    assert!(result.is_err(), "Should catch aggregated claim");
    Ok(())
}
```

## Related

- mika#1059 — this eval harness issue
- mika-skills#159 — the prompt fix that shipped the per-element enumeration rules
- `crates/mika-agent/tests/eval/grounding_assertions/mod.rs` — assertion helpers
- `crates/mika-agent/tests/eval/grounding_regressions/README.md` — scenario catalog
- `skills/bundled/qa-review/system_prompt.md` Step 2.5.5 — the prompt rules being tested
