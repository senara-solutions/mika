---
module: eval
tags: [grounding, fabrication, regression, testing, knowledge-graph]
problem_type: testing_infrastructure
---

# Grounding + Fabrication Regression Scenarios (#741)

## Problem

The KG milestone #14 retrospective catalogued four concrete fabrication classes that shipped undetected for weeks. Each was caught only by coincidence — a provider switch, an operator's sanity read, a downstream failure. Under model or prompt changes these will silently regress. No eval exercised the ground-truth check.

## Solution

Five eval scenarios under `crates/mika-agent/tests/eval/grounding_regressions/`, one per fabrication class from the retrospective plus one KG-response-ignored class routed from #740 D4.

### Assertion Framework

Four hard-assertion helpers in `tests/eval/grounding_assertions/mod.rs`:

- **`assert_response_forbids(&[&str])`** — word-boundary check that response does NOT contain forbidden words. Splits on whitespace, strips surrounding punctuation, case-insensitive. Intentionally does NOT match substrings inside compound words (e.g., "unmerged" does NOT trigger "merged").
- **`assert_any_tool_called_from(&[&str])`** — at least one tool from the set was called before EndTurn.
- **`assert_response_contains_in_order(&[&str])`** — items appear in response in order, non-overlapping.
- **`assert_response_contains(&str)`** — case-insensitive substring check.

### Frozen Regression Fixtures

Each scenario has a `fixtures/{scenario}_pre_fix.json` file containing the pre-fix response. The regression-reproduction test uses `std::panic::catch_unwind` to invoke the actual assertion helpers and verify they panic on the pre-fix input. This structural pattern proves the assertion framework catches each failure class — not just that the current code happens to pass.

### Key Design Decisions

1. **Hard assertions only, no LLM-judge** — each fabrication class has objectively checkable signals. LLM-as-judge would soften precision and introduce the same class of error it's supposed to detect.

2. **Word-boundary matching, not substring** — `assert_response_forbids` intentionally uses word-boundary tokenization. This avoids false positives on compound words ("auto-merge" does NOT trigger "merge") but means the forbidden list must include exact token forms.

3. **Regression-reproduction via `catch_unwind`** — regression tests must call the actual assertion helpers inside `catch_unwind` and verify they panic. Inline reimplementation of assertion logic creates latent divergence risk and doesn't prove the framework itself works.

4. **Unconditional assertions** — assertions must not be gated behind conditional checks (e.g., `if calls.len() >= 2`). If the precondition isn't met, that's a test failure, not something to silently skip.

## Lessons Learned

- **Conditional assertion guards are a testing anti-pattern.** The initial implementation wrapped the GraphQL fabrication check in `if gh_calls.len() >= 2`, which would silently pass if the mock produced fewer calls. Always assert preconditions explicitly.

- **Regression tests must invoke the framework, not reimplement it.** The initial regression tests used inline `contains()` checks instead of `catch_unwind` on the actual assertion helpers. If the helpers had bugs, the regression tests would not catch them.

- **`assert_response_contains_in_order` must advance past match END, not START+1.** The initial implementation advanced by 1 byte past the match start, creating a latent overlap bug where adjacent items could match overlapping positions. Fixed to advance past the full match length.

## Files

- `crates/mika-agent/tests/eval/grounding_assertions/mod.rs` — shared assertion helpers
- `crates/mika-agent/tests/eval/grounding_regressions/` — 5 scenario files + README + fixtures
- `crates/mika-agent/tests/eval.rs` — module registration
- `crates/mika-agent/CLAUDE.md` — eval section cross-reference
