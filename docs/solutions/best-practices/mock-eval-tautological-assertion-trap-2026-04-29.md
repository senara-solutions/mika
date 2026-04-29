---
title: "Mock-tier eval tests: tautological-assertion trap"
date: 2026-04-29
module: crates/mika-agent
component: eval-harness
tags:
  - testing
  - eval
  - mock-llm
  - assertion-strength
  - skill-output-contract
problem_type: best_practice
category: best-practices
track: knowledge
applies_when:
  - Writing per-skill eval tests with `MockLlmProvider`
  - Asserting on `trace.output.text` after a canned LLM response
  - Validating output-shape contracts (suffix lines, structural sections, scope markers)
ticket: senara-solutions/mika#888
references:
  - crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs
  - crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs
  - docs/solutions/architecture-patterns/agent-eval-testing-harness-mock-provider.md
  - docs/solutions/741-grounding-fabrication-regression-scenarios.md
---

# Mock-tier eval tests: the tautological-assertion trap

## Context

`MockLlmProvider` returns canned text via `EvalHarness::builder().responses(vec![text_response("...")])`. The harness routes the canned text through the agent loop and surfaces it on `trace.output.text`. This is the right pattern for testing engine handling of skill output contracts (post-condition guards, shape preservation, suffix-line discipline) — but it has a sharp failure mode: **assertions on tokens that appear in the canned mock are tautological** and prove nothing about engine behavior.

This compound was extracted from /ce:review feedback on PR mika#888 (eval test for `mika-arch-groom-milestone`), where two reviewers (correctness + testing) independently flagged a scenario whose stated intent was "verify n=1 milestone briefs still emit milestone-shape output, not per-ticket output" — but the test only asserted that `Scope: milestone` and `#900` were present, both of which are in the mock response verbatim. A per-ticket-shape response that happened to include those tokens would also pass.

This doc complements [741-grounding-fabrication-regression-scenarios.md](../741-grounding-fabrication-regression-scenarios.md) (which warns against *conditional* assertions that silently skip on bad state) and the canonical [agent-eval-testing-harness-mock-provider.md](../architecture-patterns/agent-eval-testing-harness-mock-provider.md). The failure mode here is different: assertions are unconditional but pull their truth value from the mock setup, not from engine behavior.

## Guidance

When asserting on `trace.output.text` after a `MockLlmProvider` response:

**Rule 1 — Distinguish what the mock produces from what the engine preserves.** A `contains()` check on a token that the mock authored is a tautology. The engine just hands the canned text back through. To prove the engine did something, the assertion must turn on a property the mock alone cannot establish.

**Rule 2 — Assert on structural distinguishers, not on tokens.** When the test claims a shape (milestone vs per-ticket, verdict vs disposition, error vs success), assert on the *load-bearing structural markers* that distinguish that shape from its alternatives — section headers, frontmatter keys, the literal final line, presence/absence of nested blocks. Tokens that any shape might include are too weak.

**Rule 3 — Always assert `trace.llm_call_count` for guard-pass tests.** A regression where a post-condition guard incorrectly fires on a valid response would pass every text-content assertion (the mock returns the same canned text on the retry). The only structural signal that the guard accepted the first response is `llm_call_count == 1`. For guard-fires tests, assert `> 1`.

**Rule 4 — Use the literal-final-line helper as a stricter contract than the engine guard, not a mirror.** The required-suffix-line guard accepts a match in any of the last 3 non-empty lines (`crates/mika-agent/CLAUDE.md` § Post-Conditions guard #8); a test that asserts `last_nonempty_line(output) == "Disposition: READY"` is strictly stronger than what the guard checks. That's intentional — the literal-final-line discipline (per [mika-arch-first-dogfood-2026-04-25.md](mika-arch-first-dogfood-2026-04-25.md)) is the contract downstream parsers depend on. Document the helper as the strict subset, not as a mirror of the guard.

## Why This Matters

The gap between "test passes" and "test would catch a regression" is invisible until the regression hits. A test built on tautological assertions reports green on a fully broken pipeline:

- Engine fails to keyword-match the skill → mock response still flows through (no skill injection means the system prompt is empty, but the mock text is still the LLM output) → `output.contains(token-from-mock)` passes.
- Suffix-line guard incorrectly fires on a valid response → re-prompt issued → second mock response also contains the token → assertion passes; only `llm_call_count` would catch the extra call.
- Output-shape regression (engine truncates, reorders, or drops sections) → mock authoritatively contains the token → assertion still passes if the truncation didn't hit that specific line.

In all three cases, the test was supposed to be the regression net. It wasn't. The team ships a degraded engine state on green CI.

The mock pattern is correct for what it can prove (engine wiring, post-condition guard handling, output-shape preservation). The trap is when assertion strength doesn't match assertion intent.

## When to Apply

This guidance applies whenever a test:

- Uses `MockLlmProvider` via `EvalHarness::builder().responses(...)` (canned LLM output)
- Asserts on `trace.output.text` content
- Has a stated intent of validating engine *behavior* (shape preservation, guard pass/fire, post-condition discipline) rather than mock plumbing

It does NOT apply to:

- Real-provider eval matrix tests (gated behind `MIKA_EVAL_REAL_PROVIDERS`) — those test prompt behavior, not engine handling, and the LLM-authored response is the unit of evidence.
- Pure plumbing tests (e.g., `test_per_skill_provider_override.rs`) where the assertion is "the harness recorded the correct provider name" — the mock is the unit under test and tautological-by-construction is fine.
- Bundled-skills load tests (`bundled_skills_load.rs`) which exercise discovery, not output.

## Examples

### Bad: tautological mock assertion

```rust
// Mock authoritatively contains "Scope: milestone" and "#900".
let mock = "...
Scope: milestone
Disposition: READY
... #900: standalone fix ...";

// These pass even if the engine never matched the skill, never ran the
// guard, never did anything except echo the mock back. They're properties
// of the *mock setup*, not of *engine behavior*.
assert!(output.contains("Scope: milestone"));
assert!(output.contains("#900"));
```

### Good: structural-distinguisher assertions + guard-pass count

```rust
// Section headers that appear in milestone-shape output but NOT in
// per-ticket-shape output. The mock contains them — and the test claims
// the engine preserved the milestone shape — so asserting their presence
// catches a regression where the engine truncates or restructures.
assert!(output.contains("Per-sub-issue disposition summary:"));
assert!(output.contains("Sequencing:"));

// Literal-final-line discipline: stricter than the engine guard's
// last-3-line window. Catches mock responses where the disposition slipped
// off the final line and the guard's tolerance accepted it.
assert_eq!(last_nonempty_line(output), "Disposition: READY");

// The load-bearing structural signal that the post-condition guard
// accepted the first response. Without this, a guard-fires regression
// passes silently because the second mock response is identical.
assert_eq!(trace.llm_call_count, 1);
```

The `Scope: milestone` and `#900` checks are still useful as readability anchors — they document what the test is "about" — but they cannot stand alone. Pair them with structural distinguishers and the call-count assertion.

## Prevention

- **Code review heuristic:** for every `assert!(output.contains(X))` after a mock response, ask: "if I removed this assertion, what would the test fail on? Is that what it's supposed to fail on?" If the only protection is text-content, you have a tautology.
- **Per-skill eval template:** when adding a new file under `crates/mika-agent/tests/eval/skills/`, start by listing the structural distinguishers (section headers, frontmatter, final-line shape) before writing assertions. The token list comes from the distinguishers, not from re-reading the mock.
- **Pair with `llm_call_count`:** every guard-pass test (mock returns valid output, guard should accept) needs `assert_eq!(trace.llm_call_count, 1)`. Every guard-fires test needs `> 1`. This is non-negotiable for tests that name a post-condition guard in their docstring.
- **Helper docstrings:** when a test helper looks like an engine guard (e.g., `last_nonempty_line` vs the suffix-line guard's last-3-line window), document the strictness relationship in the docstring. Future authors reading "mirrors the engine guard" will write tests that fail for the wrong reason.
