# Plan: Iterate-loop state machine — handle paraphrased architect dispositions

**Ticket:** mika#1272
**Type:** enhancement
**Date:** 2026-05-26

## Context

The iterate-loop state machine in `skills/bundled/_shared/dispatch-lib.sh` uses two verdict parsers:
- `_parse_disposition()` (first-pass): expects literal `Disposition: READY|ITERATE|ESCALATE`
- `_parse_verdict()` (second-pass): expects literal `Verdict: GROOMED|ESCALATE`

When mika-arch paraphrases (documented in `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — e.g., "Proceed. The plan is clean..." instead of `Disposition: READY`), the parser emits nothing and the state machine falls through to the `*` case, treating it as `UNPARSED` → hard failure (`return 1`). This halts the autonomous loop unnecessarily.

## Design

### Strategy: Two-tier parsing — literal first, fuzzy fallback

Each parser retains its strict `grep` as tier 1 (zero-cost fast path). If tier 1 finds nothing, a tier 2 fuzzy matcher runs against the response text looking for known paraphrase patterns. The fuzzy tier is conservative: ambiguous text defaults to `ESCALATE`.

### Paraphrase patterns (derived from dogfood observations + anticipated drift)

**First-pass disposition patterns:**

| Canonical | Paraphrase indicators |
|---|---|
| READY | "proceed", "ship it", "dispatch", "good to go", "plan is clean", "no blocking" (affirmative + no concerns) |
| ITERATE | "needs revision", "another pass", "revise", "address the following", "concerns that require" |
| ESCALATE | "escalate", "human review", "cannot proceed", "fundamental", "out of scope for" |

**Second-pass verdict patterns:**

| Canonical | Paraphrase indicators |
|---|---|
| GROOMED | "groomed", "approved", "plan is ready", "ship", "no remaining concerns" |
| ESCALATE | "escalate", "cannot approve", "human review needed", "fundamental issues remain" |

### Disambiguation rules

1. If multiple paraphrase patterns match, apply priority: ESCALATE > ITERATE > READY (most conservative wins).
2. If no pattern matches at all, emit nothing (same as today — falls to `*` case which returns 1).
3. Every fuzzy match logs the original text snippet + the mapped disposition at WARN level for telemetry drift tracking.

## Implementation units

### Unit 1: `_parse_disposition_fuzzy()` helper

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Add a new function `_parse_disposition_fuzzy()` that:
1. Reads stdin (architect response text).
2. Applies case-insensitive pattern matching against the paraphrase indicators above.
3. Applies the disambiguation priority (ESCALATE > ITERATE > READY).
4. Emits the canonical disposition on stdout, or nothing if no match.
5. If a match is found, writes a WARN line to stderr: `"_parse_disposition_fuzzy: mapped paraphrased disposition → <CANONICAL> (matched: '<snippet>')"`.

Implementation: bash `grep -qi` or `awk` against the response text. Keep it POSIX-ish (the script uses `#!/usr/bin/env bash` but targets portability within GNU/Linux).

### Unit 2: `_parse_verdict_fuzzy()` helper

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Same pattern as Unit 1 but for second-pass verdicts (GROOMED vs ESCALATE). Simpler — only two possible outcomes with clearer semantic distance.

### Unit 3: Wire fuzzy fallback into `_parse_disposition()`

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Modify `_parse_disposition()` to:
1. Try the existing strict `grep` (tier 1).
2. If tier 1 emits nothing, pipe the same input through `_parse_disposition_fuzzy()` (tier 2).
3. Return whatever tier 2 emits (may still be nothing → `*` case fires).

The function remains a stdin filter emitting 0 or 1 lines on stdout. Callers (`_iterate_groom_loop`) are unchanged.

Implementation note: since `grep` in the current implementation consumes stdin, the function needs to capture stdin into a variable first, then attempt tier 1, then conditionally attempt tier 2.

### Unit 4: Wire fuzzy fallback into `_parse_verdict()`

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Same wiring as Unit 3 but for `_parse_verdict()`.

### Unit 5: Trail annotation for fuzzy matches

**File:** `skills/bundled/_shared/dispatch-lib.sh`

When a fuzzy match is used (tier 2), the `_trail_append` call in `_iterate_groom_loop` should annotate the trail entry with `(fuzzy)` suffix. E.g., `READY (fuzzy)` instead of `READY`. This surfaces in the grooming-history line of the canonical callout, giving the operator visibility into non-literal matches.

Modify the state machine in `_iterate_groom_loop` to detect fuzzy matches (compare tier 1 output vs tier 2 output, or pass a flag from the parser). Simplest approach: have `_parse_disposition()` emit `READY` (literal) or `READY:fuzzy` (paraphrased) — the caller splits on `:` to get the canonical value and the match-type indicator.

### Unit 6: Regression tests

**File:** `skills/bundled/_shared/tests/test_parse_disposition.sh` (new)

Bash test script (can be run standalone or via `make test-skills`) that:
1. Sources `dispatch-lib.sh` (or the relevant function definitions).
2. Tests each canonical disposition with literal input → expects exact match.
3. Tests each paraphrase variant from the table above → expects correct canonical mapping.
4. Tests ambiguous input matching multiple patterns → expects ESCALATE (priority rule).
5. Tests completely unrelated text → expects empty output (no match).
6. Tests each canonical verdict (GROOMED/ESCALATE) with literal + paraphrased inputs.

Structure: one assertion function, sequential test cases, exit 1 on first failure. No external test framework dependency.

## Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Fuzzy patterns are too broad → false-positive READY on text that's actually neutral | Medium | Priority rule (ESCALATE wins ties) + conservative pattern choice (require affirmative + no-concerns signals for READY) |
| New paraphrase variants appear not covered by patterns | High (ongoing) | WARN logs on `*` case still fire; the trail captures `UNPARSED`; operator can add patterns iteratively |
| Performance overhead of double-parsing | Negligible | Only triggered when tier 1 fails (rare for well-prompted arch); text is <10KB |

## Out of scope

- Tightening mika-arch prompts to reduce paraphrasing — separate concern (prompt calibration, not parser robustness).
- LLM-based disposition classification — overengineered for a bash parser; regex/grep patterns are sufficient given the small taxonomy.
- Changing the `_write_canonical_callout` output format — that function already handles any canonical disposition value.
