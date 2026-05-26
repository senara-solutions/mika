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
| READY | "proceed", "ship it", "dispatch", "good to go", "plan is clean" (affirmative forward-motion signals only; see note below) |
| ITERATE | "needs revision", "another pass", "revise", "address the following", "concerns that require" |
| ESCALATE | "escalate", "human review", "cannot proceed", "fundamental", "out of scope for" |

**Second-pass verdict patterns:**

| Canonical | Paraphrase indicators |
|---|---|
| GROOMED | "groomed", "approved", "plan is ready", "ship", "no remaining concerns" |
| ESCALATE | "escalate", "cannot approve", "human review needed", "fundamental issues remain" |

**Pattern exclusion — `"no blocking"` removed from READY indicators.** The phrase `"no blocking"` is a negated-absence signal, not an affirmative forward-motion signal. It appears naturally in ITERATE-verdict text (e.g., *"F1 is no blocking concern but F2 requires revision"*), producing false-positive READY mappings when ITERATE indicators are paraphrased differently and tier 2 doesn't catch them. The conservative design goal (ambiguous → ESCALATE) is better served by requiring positive forward-motion signals only. Citation: review-guide.md § KISS — the simplest correct pattern for READY is a positive-forward-motion signal; mika#1272 issue body ("Mapping is conservative: ambiguous paraphrases fall back to ESCALATE rather than guess").

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

Implementation note: since `grep` in the current implementation consumes stdin, the function captures stdin into a local variable via `local TEXT; TEXT=$(cat)` before attempting tier 1. This is standard bash, handles the expected 5–20KB architect response sizes without issue, and preserves the existing caller contract (`echo "$ARCH_RESPONSE" | _parse_disposition`). The captured `TEXT` is then piped to both tier 1 and (conditionally) tier 2 via `echo "$TEXT" | ...`.

### Unit 4: Wire fuzzy fallback into `_parse_verdict()`

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Same wiring as Unit 3 but for `_parse_verdict()`.

### Unit 5: Trail annotation for fuzzy matches — side-channel flag

**File:** `skills/bundled/_shared/dispatch-lib.sh`

When a fuzzy match is used (tier 2), the `_trail_append` call in `_iterate_groom_loop` should annotate the trail entry with `(fuzzy)` suffix. E.g., `READY (fuzzy)` instead of `READY`. This surfaces in the grooming-history line of the canonical callout, giving the operator visibility into non-literal matches.

**Mechanism: module-global flag variable (not stdout-channel suffix).** The parser functions (`_parse_disposition()`, `_parse_verdict()`) set a module-global flag `_DISPOSITION_FUZZY=1` when tier 2 fires, and `_DISPOSITION_FUZZY=0` when tier 1 fires. The parser stdout remains a clean canonical value (`READY`, `ITERATE`, or `ESCALATE`) — the `case "$DISPOSITION" in READY) ...` dispatch in `_iterate_groom_loop` (PR#1275, `30917d73`) is unchanged. After the parser call, `_iterate_groom_loop` reads `_DISPOSITION_FUZZY` and appends `(fuzzy)` to the trail entry if set.

This avoids modifying `_iterate_groom_loop`'s case logic — a merged, shipped component whose stdout-channel contract must not change without updating all consumers. The side-channel is orthogonal: it carries metadata about the match without polluting the value channel. Citation: review-guide.md § Orthogonality — changes should propagate minimally; the stdout-channel contract of a filter function must not be changed without updating all consumers.

**Concrete implementation:**
```bash
# Module-global, reset before each parse call
_DISPOSITION_FUZZY=0

_parse_disposition() {
  _DISPOSITION_FUZZY=0
  local TEXT
  TEXT=$(cat)
  local RESULT
  RESULT=$(echo "$TEXT" | grep -oP '(?<=Disposition:\s)(READY|ITERATE|ESCALATE)')
  if [[ -n "$RESULT" ]]; then
    echo "$RESULT"
    return
  fi
  # Tier 2 fallback
  RESULT=$(echo "$TEXT" | _parse_disposition_fuzzy)
  if [[ -n "$RESULT" ]]; then
    _DISPOSITION_FUZZY=1
    echo "$RESULT"
  fi
}
```

The caller reads the flag after the parse:
```bash
DISPOSITION=$(echo "$ARCH_RESPONSE" | _parse_disposition)
local TRAIL_SUFFIX=""
[[ "$_DISPOSITION_FUZZY" == "1" ]] && TRAIL_SUFFIX=" (fuzzy)"
_trail_append "${DISPOSITION}${TRAIL_SUFFIX}"
```

No changes to `_iterate_groom_loop`'s `case "$DISPOSITION" in ...` block are required.

### Unit 6: Regression tests — with source isolation

**File:** `skills/bundled/_shared/tests/test_parse_disposition.sh` (new)

Bash test script (can be run standalone or via `make test-skills`) that:
1. Sources `dispatch-lib.sh` with source-isolation guard (see below).
2. Tests each canonical disposition with literal input → expects exact match.
3. Tests each paraphrase variant from the table above → expects correct canonical mapping.
4. Tests ambiguous input matching multiple patterns → expects ESCALATE (priority rule).
5. Tests completely unrelated text → expects empty output (no match).
6. Tests each canonical verdict (GROOMED/ESCALATE) with literal + paraphrased inputs.

Structure: one assertion function, sequential test cases, exit 1 on first failure. No external test framework dependency.

**Source isolation.** `dispatch-lib.sh` is a 1000+ line orchestration script with potential top-level side effects (trap registrations, `set -e`, variable initialization, early-exit guards). The test script must not depend on those side effects being benign. Two approaches, in order of preference:

1. **Guard variable in `dispatch-lib.sh`** (preferred): Add `if [[ "${_DISPATCH_LIB_SOURCED:-}" == "1" ]]; then return 0 2>/dev/null || true; fi` near the top of `dispatch-lib.sh`, after function definitions but before any top-level imperative code. Set `_DISPATCH_LIB_SOURCED=1` at the end of the function-definitions section. This makes re-sourcing a no-op and ensures the test environment only gets function definitions.
2. **Verify safety before implementation**: Before writing the test script, the implementer must audit `dispatch-lib.sh` for top-level side effects beyond function definitions and variable declarations. If any are found (trap registrations, `set -e`, env var references not present in test context), the guard variable approach (option 1) is mandatory. Document the audit result as a comment in the test script header.

The existing `test-dispatch-lib.sh` test pattern (`mktemp -d` + `trap RETURN` per-test function) should be followed for per-test-case isolation within the script. Citation: review-guide.md § Single Responsibility — the test script's correctness must not depend on side-effect behavior of a 1000-line orchestration script.

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

## Revision history

- rev 2 (2026-05-26): addressed F1 by replacing `READY:fuzzy` colon-suffix stdout design with module-global `_DISPOSITION_FUZZY` flag variable — parser stdout remains clean canonical values, `_iterate_groom_loop` case dispatch unchanged (review-guide.md § Orthogonality); addressed F2 by removing `"no blocking"` from READY paraphrase indicators — negated-absence signals are too loose, READY tier 2 now requires positive forward-motion signals only (review-guide.md § KISS, mika#1272 conservatism goal); addressed F3 by specifying source-isolation guard pattern for test script sourcing `dispatch-lib.sh` — guard variable `_DISPATCH_LIB_SOURCED` prevents top-level side-effect re-execution, with mandatory audit-before-implement fallback (review-guide.md § Single Responsibility). Also sharpened Unit 3 stdin capture mechanism per reviewer's note (`TEXT=$(cat)` with local variable).
