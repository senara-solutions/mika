---
module: dispatch-lib
date: 2026-05-27
problem_type: best_practice
component: tooling
severity: medium
tags:
  - dispatch-lib
  - iterate-loop
  - mika-arch
  - disposition
  - fuzzy-matching
  - state-machine
  - paraphrased-output
applies_when:
  - Parsing structured LLM output (dispositions, verdicts, labels) in a bash state machine
  - An LLM-driven workflow depends on exact string matching for routing decisions
  - The LLM sometimes paraphrases instead of producing the exact expected format
---

# Two-tier fuzzy disposition parsing for LLM state machines

## Context

The iterate-loop state machine in `skills/bundled/_shared/dispatch-lib.sh` routes
architect review outcomes via two parsers: `_parse_disposition()` (first-pass:
READY/ITERATE/ESCALATE) and `_parse_verdict()` (second-pass: GROOMED/ESCALATE).
Both originally used strict `grep` for literal `Disposition: <X>` or `Verdict: <X>`
lines. When mika-arch paraphrased (e.g., "Proceed. The plan is clean..." instead of
`Disposition: READY`), the parser emitted nothing and the state machine halted with
an `UNPARSED` failure — stopping the autonomous loop unnecessarily.

This was documented as a known failure mode in
`docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` after the
first dogfood run of the architect skills.

## Guidance

### Two-tier parsing: literal first, fuzzy fallback

Each parser retains its strict `grep` as **tier 1** (zero-cost fast path). If tier 1
finds nothing, a **tier 2** fuzzy matcher runs against the response text looking for
known paraphrase patterns. The fuzzy tier is conservative: ambiguous text defaults to
the most cautious outcome (ESCALATE).

### Disambiguation priority

When multiple paraphrase patterns match the same text, the most conservative
canonical value wins:

- **Dispositions:** ESCALATE > ITERATE > READY
- **Verdicts:** ESCALATE > GROOMED

This ensures that mixed signals never produce a false-positive "go ahead."

### Paraphrase pattern design rules

1. **Use multi-word phrases, not bare words.** A bare keyword like `"ship"` matches
   substrings in unrelated words ("relationship", "ownership"). Use `"ship it"` instead.
2. **Prefer affirmative forward-motion signals for the "go" outcome.** Avoid
   negated-absence patterns like `"no blocking"` — they appear naturally in text
   that's actually recommending revision (e.g., "F1 is no blocking concern but F2
   requires revision").
3. **Log every fuzzy match.** When tier 2 fires, write a WARN to stderr with the
   matched snippet and the canonical value it mapped to. This creates a telemetry
   trail for calibrating the LLM prompt over time.

### Side-channel for match metadata

The parser functions run inside `$(...)` subshells, so they cannot set parent-scope
variables. To communicate whether a fuzzy match was used (for trail annotation),
use a **tmpfile side-channel**:

```bash
_DISPOSITION_FUZZY_FILE="${TMPDIR:-/tmp}/.dispatch-lib-fuzzy-$$"

# Inside the parser — write "1" when tier 2 fires, "0" for tier 1
printf '0' > "$_DISPOSITION_FUZZY_FILE"
# ... tier 1 attempt ...
# If tier 2 fires:
printf '1' > "$_DISPOSITION_FUZZY_FILE"

# Helper for callers (reads the tmpfile after $(...) returns)
_disposition_was_fuzzy() {
    [ -f "$_DISPOSITION_FUZZY_FILE" ] && \
      [ "$(cat "$_DISPOSITION_FUZZY_FILE" 2>/dev/null)" = "1" ]
}
```

This keeps the parser's stdout contract clean (canonical value only) while letting
callers annotate trail entries with `(fuzzy)` for operator visibility.

## Why This Matters

LLM-driven automation pipelines that depend on exact string matching for control flow
are inherently fragile. Models drift, paraphrase, and occasionally omit structured
markers. A two-tier approach provides defense-in-depth: the fast literal match handles
99% of cases, and the fuzzy fallback catches the rest with conservative
disambiguation. Without it, the autonomous loop halts on every paraphrased
disposition, requiring manual intervention.

## When to Apply

- Any bash or script-based state machine that parses structured LLM output for routing
- Especially when the LLM is an external agent (like mika-arch) whose prompt you
  control but whose output you cannot guarantee will be perfectly formatted
- When the failure mode of an unparsed output is "halt the pipeline" rather than
  "degrade gracefully"

## Examples

**Before (v1 — literal only):**
```bash
_parse_disposition() {
    grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
        | grep -oE '(READY|ITERATE|ESCALATE)' | head -1
}
# Input: "Proceed. The plan is clean." → output: (nothing) → UNPARSED → halt
```

**After (v2 — two-tier):**
```bash
_parse_disposition() {
    printf '0' > "$_DISPOSITION_FUZZY_FILE"
    local text; text=$(cat)
    local result
    result=$(printf '%s' "$text" | grep -oE 'Disposition:...(READY|ITERATE|ESCALATE)' \
        | grep -oE '(READY|ITERATE|ESCALATE)' | head -1)
    if [ -n "$result" ]; then echo "$result"; return; fi
    # Tier 2 fallback
    result=$(printf '%s' "$text" | _parse_disposition_fuzzy)
    if [ -n "$result" ]; then
        printf '1' > "$_DISPOSITION_FUZZY_FILE"
        echo "$result"
    fi
}
# Input: "Proceed. The plan is clean." → fuzzy match "proceed" → READY (fuzzy)
```

## Related

- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — original
  observation of disposition drift during architect skill dogfooding
- `docs/solutions/best-practices/required-suffix-line-guard-verdict-ghosting-structural-fix-2026-04-29.md` —
  related verdict-ghosting fix using structural prompt guards
- mika#1272 — the ticket that implemented this pattern
- mika#1271 — parent ticket: iterate-loop state machine contract refactor
