---
ticket: mika#1177
branch: fix/1177/mika-arch-asserted-unavailability
status: active
date: 2026-06-12
origin: https://github.com/senara-solutions/mika/issues/1177
execution: code
---

# Plan: extend `ASSERTED_UNAVAILABILITY_PATTERNS` for three escape shapes (mika#1177)

## Problem frame

Adversarial review on mika#894 surfaced three additional escape shapes that the extended P2/P3/P4 regex patterns still don't catch:

- **Shape A — descriptor-word absorption.** `the gh_read tool is not available` — P2 leftmost-match captures `tool` (not `gh_read`); the `enabled_tools` lookup then rejects, so the guard does NOT fire (false negative).
- **Shape B — antonym `unavailable`.** `gh_read is currently unavailable` / `gh_read appears to be unavailable` — P2's terminal alternation `(?:available|callable|accessible)` doesn't include the single-word adjective `unavailable`. No P1-P5 match.
- **Shape C — modal / periphrastic negation.** `gh_read may not be callable`, `gh_read could not be called`, `gh_read doesn't appear to be callable`, `unable to call gh_read` — none match P1-P5. `cannot` (P5) is literal only; `unable to call` is a natural synonym not covered.

Downstream required-tools-gate is the safety net for these cases (architect retries, `gh_read` succeeds), so impact is wasted retry turns, not undetected fabrications landing in verdicts. Still: same family and costs as mika#862 / mika#894.

## Scope boundaries

- Pure regex extension to `crates/mika-agent/src/agent.rs::ASSERTED_UNAVAILABILITY_PATTERNS`. No guard chain reordering, no new guard types, no skill-prompt rules.
- Patterns extended with optional descriptor word, antonym alternation, and modal/periphrastic forms — must preserve the two-layer false-positive filter (snake-case identifier + enabled-set lookup).
- Test discipline per mika#894 Rule 4: every new shape gets a verbatim test fixture in the same PR.
- **Out of scope:** affirmative-state-claim guard (#1331), assert-grounded patterns, downstream gate semantics.

## Implementation Units

### U1 — Shape A: descriptor-word absorption fix

**Goal:** Extend P2 (and P3) so `the gh_read tool is not available` captures `gh_read`, not `tool`.

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (`ASSERTED_UNAVAILABILITY_PATTERNS`, ~line 6295)

**Approach:** Add an optional descriptor noun between the tool name and the copula. Two acceptable shapes:
1. Modify P2 in place: `\b(?P<tool>[a-z_][a-z0-9_]*) (?:tool |function |feature |skill |handler )?(?:is )?(?:\w+ly )?not (?:available|callable|accessible)`
2. Add a new pattern (e.g., P2b) using a `(?:[a-z_][a-z0-9_]*)` group + a non-capturing descriptor word, with the capture group anchored to the prior word.

Either approach is acceptable. The implementer chooses based on which keeps the existing `test_detect_asserted_unavailability_natural_language_filtered` test passing (`the service is not available right now` → still filtered by enabled-set lookup because `service` ∉ enabled set).

**Constraint:** the named capture `(?P<tool>...)` MUST contain the actual tool name (`gh_read`), not the descriptor word (`tool`). Verify via unit test assertion on the captured token.

**Execution note:** test-first. Write the failing unit test (`gh_read tool is not available` → `Some("gh_read")`) before modifying the regex.

**Patterns to follow:**
- `crates/mika-agent/src/agent.rs:6295` — five-pattern Vec construction with named capture groups
- `crates/mika-agent/src/agent.rs:11169-11217` (#894 elided-copula + adverb-interposed tests) — fixture style and tagging conventions

**Test scenarios:**
- **Happy path:** `the gh_read tool is not available` → captures `gh_read`, returns `Some("gh_read")` when `gh_read ∈ enabled_tools`
- **Variants:** `the gh_read function is not callable`, `the gh_read skill is not accessible`
- **Capture-correctness regression:** new test must assert the captured token equals `gh_read`, not `tool` — this is the structural fix being made.
- **Natural-language filter preserved:** `the service tool is not available right now` → still returns `None` (no `service` in enabled set), AND `the storage feature is not callable` → still returns `None`. The descriptor-word relaxation MUST NOT widen false-positive surface.

**Verification:** all three new unit tests pass; all 8 existing `test_detect_asserted_unavailability_*` tests (P1-P5 + natural language + case-insensitive) still pass.

### U2 — Shape B: antonym `unavailable`

**Goal:** Single-word adjective `unavailable` matches asserted-unavailability claims.

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (`ASSERTED_UNAVAILABILITY_PATTERNS`)

**Approach:** Add a new pattern matching `<tool> (?:is )?(?:\w+ly )?unavailable`. Stand-alone pattern (do not fold into P2 because P2's `not (?:available|...)` semantics differ — splitting keeps each pattern's intent legible). Pattern shape:

```rust
r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is )?(?:\w+ly )?unavailable"
```

Position in the Vec: append after the current P5 (mika#1177 patterns go at the end to keep P1-P5 numbering stable for downstream references).

**Execution note:** test-first.

**Patterns to follow:** same as U1.

**Test scenarios:**
- **Happy path:** `gh_read is currently unavailable` → `Some("gh_read")` when `gh_read ∈ enabled_tools`
- **Variants:** `gh_read appears to be unavailable` (the `appears to be` shape is U3, but the bare `gh_read unavailable` should also match), `gh_read is unavailable`, `gh_read structurally unavailable`
- **Natural-language filter preserved:** `the service is currently unavailable` → `None` (`service` ∉ enabled set)

**Verification:** new unit test passes; existing tests still pass.

### U3 — Shape C: modal / periphrastic negation

**Goal:** Modal and periphrastic forms (`may not`, `could not`, `unable to`, `doesn't appear to`) match.

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (`ASSERTED_UNAVAILABILITY_PATTERNS`)

**Approach:** Add two patterns:

```rust
// Periphrastic modal: "X may not / could not / cannot be called/invoked/used"
// (subsumes the existing P5 'cannot call X' shape; keep P5 for back-compat — both passing is fine)
r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:may|could|cannot|can'?t|won'?t|wouldn'?t) (?:not )?be (?:called|invoked|used|accessed|reached)"

// Inverted modal: "unable to call/invoke/use/access/reach X"
r"(?i)\bunable to (?:call|invoke|use|access|reach) (?P<tool>[a-z_][a-z0-9_]*)"
```

The `doesn't appear to be callable` shape: add a third pattern OR fold into an existing pattern with `(?:doesn'?t (?:appear|seem) to )?` as a non-capturing prefix variant. Implementer choice — prefer adding a clean stand-alone pattern over expanding an existing one if expanding compromises readability.

**Execution note:** test-first.

**Test scenarios:**
- **Happy paths (all → `Some("gh_read")`):**
  - `gh_read may not be callable`
  - `gh_read could not be called`
  - `gh_read cannot be invoked here`
  - `gh_read doesn't appear to be callable`
  - `unable to call gh_read`
  - `unable to invoke gh_read in this session`
- **Natural-language filter preserved:**
  - `service may not be called from this context` → `None`
  - `unable to reach the storage service` → `None`

**Verification:** all new unit tests pass; existing tests still pass.

### U4 — Eval-harness regression fixtures

**Goal:** Each new shape has a frozen pre-fix fixture proving the assertion catches the failure class. Mirror mika#894's eval pattern.

**Files:**
- Create: `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_extension_shapes.rs`
- Create: `crates/mika-agent/tests/eval/grounding_regressions/fixtures/asserted_unavailability_descriptor_absorption_pre_fix.json`
- Create: `crates/mika-agent/tests/eval/grounding_regressions/fixtures/asserted_unavailability_antonym_unavailable_pre_fix.json`
- Create: `crates/mika-agent/tests/eval/grounding_regressions/fixtures/asserted_unavailability_modal_negation_pre_fix.json`
- Modify: `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` — register new module

**Approach:** Three eval scenarios + three frozen pre-fix fixtures. For each shape (A/B/C):
- Test: the guard fires (correction emitted, the unavailability claim is rejected) given the new pattern.
- Frozen pre-fix fixture: response containing the verbatim Shape-A/B/C escape phrase + a `gh_read` claim — proves the test would have failed before this fix.

Use the existing `asserted_unavailability_elided_copula.rs` and its three `*_pre_fix.json` siblings as the template.

**Execution note:** test-first. Frozen fixtures stored in `fixtures/`; assertion helpers from `tests/eval/grounding_assertions/mod.rs` (`assert_response_forbids`, `assert_response_contains` for correction message).

**Patterns to follow:**
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_elided_copula.rs` — 3-test file pattern (one per shape)
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` — pre-fix fixture pattern (`test_regression_*_pre_fix_shape` companion test)
- `crates/mika-agent/tests/eval/grounding_regressions/mod.rs:38-41` — module registration

**Test scenarios:**
- **Shape A:** assistant response says `the gh_read tool is not available in this session` → guard fires, correction message naming `gh_read` is emitted, retry occurs
- **Shape B:** assistant response says `gh_read is currently unavailable here` → guard fires
- **Shape C:** assistant response says `unable to call gh_read in this mode` (and one of the modal variants) → guard fires
- **Regression-reproduction (per shape):** frozen pre-fix fixture proves the assertion catches the failure class

**Verification:** three new eval tests pass; existing grounding-regression suite (`cargo test -p mika-agent --test eval grounding_regressions::asserted_unavailability` → 4 existing test files) still passes.

### U5 — Compound doc update

**Goal:** Append the three new shapes to `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 2 (or its successor doc) so the gate-evasion catalogue stays canonical.

**Files:**
- Modify: `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` (or create successor doc if the file size warrants splitting)

**Approach:** Add a new subsection or extend Rule 2 with the three shapes + their evidence references. One verbatim example per shape. Cite mika#1177 PR + acceptance test names.

**Execution note:** docs-only — no behavioral guard, just institutional knowledge update. Run after U1-U4 ship to ensure cited test names are stable.

**Verification:** doc lint passes (markdown well-formed); KG ingestion picks up the new content on next startup (verified manually post-merge).

## Dependencies / sequencing

- U1, U2, U3 are independent — each adds one or two patterns to the same Vec. Implementer may ship them as separate commits within the same PR, or one consolidated commit.
- U4 depends on U1-U3 having landed in the agent.rs source (the eval tests assert on behavior added by U1-U3).
- U5 is final — appends the catalogue after the patterns are stable.

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/agent.rs:6295` — `LazyLock<Vec<Regex>>` construction with named `(?P<tool>...)` capture groups
- `crates/mika-agent/src/agent.rs:6327-6341` — `detect_asserted_unavailability` two-layer filter (snake-case capture + `enabled_tools` lookup)
- mika#894 PR diff — the established shape for "extend patterns + add unit tests + add frozen fixtures"

## Verification (top-level)

- `cargo test -p mika-agent agent::tests::test_detect_asserted_unavailability` — all 8 existing tests pass + ~8 new tests for Shape A/B/C
- `cargo test -p mika-agent --test eval grounding_regressions::asserted_unavailability` — 4 existing test files pass + 1 new file (3+ tests)
- `cargo clippy -p mika-agent` clean
- `cargo fmt --all -- --check` clean

## Risk / known unknowns

- **Capture-group correctness on Shape A:** if the descriptor-word relaxation widens the natural-language false-positive surface, the natural-language filter test will catch it — that's the safety net. The unit test ordering (capture-correctness assertion BEFORE filter assertion) keeps both regression vectors covered.
- **Pattern ordering matters for false positives.** New patterns appended at end of Vec to keep P1-P5 numbering stable. The detector iterates ALL patterns and accepts the first enabled-set hit — order does NOT affect correctness, only diagnostic clarity.
- **Anchor strictness on inverted modal pattern.** `unable to call gh_read` lacks a trailing boundary — if the tool name is followed by punctuation/EOL/whitespace, regex matches naturally. If followed by other word characters (e.g., `gh_readers`), `[a-z_][a-z0-9_]*` greedy matching captures too much. Add `\b` after the capture group if needed.

## Out-of-scope (explicit)

- Affirmative-state-claim patterns (#1331's assert-grounded guard) — separate family.
- Changes to `format_asserted_unavailability_correction` correction message structure — text reuse is fine.
- Refactoring the five-pattern Vec into a structured pattern table (mika#894 considered this; deferred as YAGNI).
