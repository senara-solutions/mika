---
title: "Close elided-copula regex gap in `asserted_unavailability` guard (mika#894)"
date: 2026-05-13
type: fix
ticket: mika#894
related_tickets:
  - mika#862  # original guard ticket
  - mika#881  # PR that shipped mika#862
  - mika#654  # recurrence-1 catalogue entry
  - mika#788  # recurrence-2 (different family, sufficiency hallucination)
  - mika#863  # quoted-resource pre-fetch (different family, Rule 1 surface)
  - mika#886  # 2026-04-29 instance, elided-copula
  - mika#890  # adjacent family, transport-contract
  - mika#891  # contract-level fix for mika#890
  - mika#893  # 2026-04-29 grooming session that triggered this ticket
status: draft
---

# Plan — Close elided-copula regex gap in `asserted_unavailability` guard

## Premise correction (architect, read this first)

The ticket body proposes three fix surfaces for the architect to choose between:

> 1. **Engine-side: extend the required-tools-gate post-condition** …
> 2. **Skill-prompt-side: add explicit instruction** …
> 3. **Hybrid:** ship both.

**None of these match the actual gap.** The structural guard the ticket asks for *already exists*. mika#862 was implemented and merged via PR #881 on 2026-04-29 (the same day mika#893 ran). The implementation is in `crates/mika-agent/src/agent.rs`:

- `ASSERTED_UNAVAILABILITY_PATTERNS` static (five regex patterns) — `agent.rs:5049–5069`
- `detect_asserted_unavailability()` fn — `agent.rs:5081–5095`
- `asserted_unavailability_satisfied()` fn — `agent.rs:5107–5113`
- Guard fire-site in the post-condition chain — `agent.rs:1380–1402`
- Eval coverage — `tests/eval/grounding_regressions/asserted_unavailability_{caught,genuine}.rs`

The recurrence in mika#893 is **not** a missing guard. It is a **regex coverage gap in the existing guard**. The mika#893 phrasing `gh_read not callable in CLI session` elides the copula (`is`), so pattern 2 (`\b(?P<tool>[a-z_][a-z0-9_]*) is not (?:available|callable|accessible)`) does not match. The required-tools gate caught the omission one post-condition later — that is why the architect retried gh_read and succeeded on the retry, exactly as the ticket reports. The asserted-unavailability guard was bypassed, but the safety net behind it caught the same fabrication a turn later.

This re-grounding is the load-bearing decision for the plan. Architect: if you disagree that the gap is regex coverage (not guard absence), say so explicitly in the first-pass review. Everything below depends on this premise being right.

## Evidence — which N=6 phrasings escape the current regex

Verified by running the production regex patterns against each catalogued phrasing:

| # | Ticket | Catalogued phrase | Matches current regex? | Why / why not |
|---|---|---|---|---|
| 1 | mika#654 | "gh_read is skill-scoped, not callable" | YES (P4) | Has "is" before "skill-scoped". |
| 1' | mika#654 (variant) | "gh_read skill-scoped, not callable" | NO | Elides "is" → P4 requires literal "is skill-scoped". |
| 3 | mika#863 | "gh_read structurally not callable" | NO | Adverb between tool and "not"; P2 requires `<tool> is not` adjacent. |
| 3' | mika#863 (variant) | "gh_read is structurally not callable" | NO | Same — adverb breaks adjacency. |
| 4 | mika#886 | "gh_read not callable in CLI session" | NO | Elides "is" → P2 requires `<tool> is not`. |
| 6 | mika#893 | "gh_read not callable in this CLI session" | NO | Same shape as #4. |

Mika#788 and mika#890 are not in the asserted-unavailability family (sufficiency hallucination and transport-contract retry, respectively) and are correctly out of scope for this plan.

The dominant escape shape is **elided copula** (4 of 5 unmatched phrasings). Adverb interposition is the secondary shape (mika#863 variants).

## Fix — extend regex pattern coverage

### Change 1 — Patterns 2 and 3: optional copula + optional adverb

Current:
```rust
regex::Regex::new(
    r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) is not (?:available|callable|accessible)",
)
.expect("asserted_unavailability pattern 2"),
regex::Regex::new(
    r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) isn'?t (?:available|callable|accessible)",
)
.expect("asserted_unavailability pattern 3"),
```

Proposed:
```rust
regex::Regex::new(
    r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is (?:\w+ly )?)?not (?:available|callable|accessible)",
)
.expect("asserted_unavailability pattern 2"),
regex::Regex::new(
    r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) isn'?t (?:\w+ly )?(?:available|callable|accessible)",
)
.expect("asserted_unavailability pattern 3"),
```

What changed:
- `is` is now optional in P2: `(?:is (?:\w+ly )?)?` — match `X is not Y`, `X not Y`, or `X is structurally not Y`.
- A single adverb is permitted between `is`/`isn't` and `not`/the predicate in both patterns. `(?:\w+ly )?` permits one adverb of shape `*ly` (covers "structurally", "currently", "presently", "literally" — the natural-language adverb forms encountered in N=6).
- Snake-case tool-name capture is unchanged: `[a-z_][a-z0-9_]*`.
- Enabled-set filter is unchanged: any spurious capture (e.g. `service`, `tool`) is rejected by `enabled_tools.contains()`.

P3 (contracted `isn't`) gains only the adverb option — the contraction inherently keeps the copula, so no elision case there.

### Change 2 — Pattern 4: optional copula on `skill-scoped`

Current:
```rust
regex::Regex::new(r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) is skill-scoped")
    .expect("asserted_unavailability pattern 4"),
```

Proposed:
```rust
regex::Regex::new(r"(?i)\b(?P<tool>[a-z_][a-z0-9_]*) (?:is )?skill-scoped")
    .expect("asserted_unavailability pattern 4"),
```

What changed: `is` is now optional. Closes the `gh_read skill-scoped, not callable` shape from mika#654-variant.

### Change 3 — Patterns 1 and 5: leave alone

P1 (`I don't have access to X`) and P5 (`cannot call X`) had no observed elision in the N=6 catalogue. Their phrasing is verb-led and grammatical; elision is structurally unlikely. Defer.

### Why this shape

- **Surgical extension over rewrite.** The guard's architecture (turn-start enabled-tool snapshot, single-retry semantics, dynamic correction message) is sound. The coverage failure is at the regex layer.
- **Two-layer filter remains the safety net.** Making "is" optional in P2 risks catching natural-language phrases like "the service not available" — but `service` is not in `enabled_tools`, so the enabled-set lookup rejects it (same defense that exists today for the unbounded patterns).
- **No new state, no chain reordering.** The fire-site at `agent.rs:1380` is untouched. The retry label `"asserted_unavailability"` and its `intent_guard_retries` interaction are untouched. The correction message text is untouched.

## Test coverage

### New eval scenarios — `tests/eval/grounding_regressions/asserted_unavailability_elided_copula.rs`

Three scenarios mirroring the existing `asserted_unavailability_caught.rs` shape:

1. **`test_asserted_unavailability_elided_copula_caught`** — Mock turn 1 emits `search_memory not callable in CLI session` (literal mika#893 phrasing, tool name substituted). Assert: `llm_call_count > 1` (guard fired) AND `search_memory` was called on retry. Pre-fix: would have escaped — extended regex catches it.

2. **`test_asserted_unavailability_elided_skill_scoped_caught`** — Mock turn 1 emits `search_memory skill-scoped, not callable here`. Assert: guard fires, retry-call happens. Pre-fix: would have escaped P4.

3. **`test_asserted_unavailability_adverb_interposed_caught`** — Mock turn 1 emits `search_memory is structurally not callable in this session`. Assert: guard fires, retry-call happens. Pre-fix: would have escaped P2.

Each scenario also gets a `fixtures/*_pre_fix.json` frozen fixture (matching the existing pattern at `tests/eval/grounding_regressions/fixtures/asserted_unavailability_caught_pre_fix.json`) so the regression-reproduction tests have a frozen failure shape to assert against.

### Unit tests — extend the existing tests block in `agent.rs`

The existing `#[cfg(test)] mod tests` block at `agent.rs:8499–8623` already covers:
- Pattern 1 — "I don't have access to"
- Pattern 2 — "is not available/callable/accessible"
- Pattern 3 — "isn't available/callable/accessible"
- Pattern 4 — "is skill-scoped"
- Pattern 5 — "cannot call"
- Tool-not-in-registry filter
- Natural-language filter ("the service is not available")
- Case-insensitive capture

Add three new unit tests (one per shape), each calling `detect_asserted_unavailability()` directly with a fixed `enabled_tools` set:

- `test_detect_asserted_unavailability_elided_copula` — `"gh_read not callable in CLI session"` returns `Some("gh_read")`.
- `test_detect_asserted_unavailability_adverb_interposed` — `"gh_read is structurally not callable"` returns `Some("gh_read")`. Also `"gh_read structurally not callable"`.
- `test_detect_asserted_unavailability_elided_skill_scoped` — `"gh_read skill-scoped"` returns `Some("gh_read")`.

And one regression test for the existing natural-language filter: confirm `"the service not available"` (elided form of an existing tested phrase) still returns `None` because `service` is not in the enabled set. This pins the two-layer defense intact.

### Existing tests must continue to pass

- `test_detect_asserted_unavailability_natural_language_filtered` — phrase `"the service is not available"` returns `None`. Extended P2 with `(?:is (?:\w+ly )?)?` still captures `service`, and `service` is still not in `enabled_tools`. Test still passes.
- `test_asserted_unavailability_genuine.rs` — agent claims unavailability of a tool that is genuinely disabled. Extended regex still captures the tool name, but `enabled_tools.contains()` returns `false` → `detect_asserted_unavailability` returns `None`, guard does not fire. Test still passes.

## Compound doc update

Update `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`:

1. **Add recurrence-3 to the catalogue** under "The Two Recurrences" (rename heading to "The Recurrences" since N=3 now):
   - mika#893 (2026-04-29) — recurrence 3: elided-copula shape. Same underlying failure (claim unavailability without attempting the call); new linguistic manifestation. The structural guard from mika#862 fired on the canonical phrasings but missed the elided-copula form. Closed by mika#894 regex extension.

2. **Add a Rule 4 (or sub-rule under Rule 2)** — "Pattern coverage is verbatim". When a structural guard uses regex pattern matching, the test fixture set must include every catalogued recurrence's *verbatim* phrasing (not a normalized form). A pattern that catches "X is not Y" but misses "X not Y" is a false-floor guard — it appears to protect against the failure class but only protects against the typed-out form. Defense: any new recurrence catalogue entry MUST be added as a regex test fixture in the same PR that catalogues it.

   **Canonical fixture locations** (cite these in Rule 4 so the rule is actionable, not advisory — per first-pass NF1):
   - `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` (Patterns 1, 2, 4 — canonical phrasings)
   - `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_genuine.rs` (false-positive defense — genuinely disabled tool)
   - `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_elided_copula.rs` (Patterns 2, 3, 4 elided/adverb-interposed shapes — new in this PR)
   - `crates/mika-agent/src/agent.rs` `#[cfg(test)] mod tests` block — unit tests for `detect_asserted_unavailability()` directly (registry-filter and case-insensitive coverage)

   A new contributor adding a recurrence to the N catalogue should add the verbatim phrasing to whichever file's shape it matches, and add a frozen `*_pre_fix.json` fixture to `tests/eval/grounding_regressions/fixtures/` alongside.

3. **Update the "Forward Work" section** — note that the structural counterpart is *operational and regex-extended*, with the verbatim-fixture rule now governing pattern maintenance.

4. **Update "Citations"** — add mika#894 alongside mika#862 as the regex-extension follow-up.

The doc-level update is in the same PR as the regex extension. Per the doc's own Rule 3 ("the catalogue is not the enforcement"), the doc edit is documentation, not the fix. The regex change is the fix.

## What this plan does NOT do

- Does not add a new EndTurn post-condition.
- Does not change the post-condition chain ordering.
- Does not modify the required-tools gate, the persistence guard, or any sibling guard.
- Does not modify the skill prompts. (Per `feedback_prompt_enforcement_fragile`, prompt-only enforcement is known to drift under load. The compound doc's Rule 3 makes the same point about catalogues.)
- Does not address the adjacent families: quoted-resource pre-fetch (mika#863, Rule 1 surface), transport-contract retry (mika#890), Mode 3 contract-fabrication. Those are separate tickets and stay out of scope.
- Does not promote the "fabrication-to-avoid-work" meta-family to a unifying compound doc — the existing compound's footer flags promotion-on-N=4, this is N=3 within the asserted-unavailability sub-family.

## Acceptance criteria (mapped to the ticket body)

The ticket's acceptance section asks for four checkable items. Mapping:

- [ ] **"A specific gate (engine-side or skill-prompt structural) prevents asserted-unavailability disclosures from being emitted without an accompanying tool-call attempt earlier in the same turn."** — Existing guard does this. This plan closes the regex coverage gap so the guard fires on the elided-copula and adverb-interposed shapes.
- [ ] **"Test fixture: synthetic scenario where mika-arch's first instinct would be to disclose tool unavailability."** — Three new eval scenarios under `tests/eval/grounding_regressions/` plus three new unit tests in `agent.rs`.
- [ ] **"Post-fix verification on a fresh /mika-groom-ticket grooming run: the architect's first-turn disclosures contain only verified-this-turn tool-call evidence (or no unavailability claims at all)."** — Verifiable by re-running `/mika-groom-ticket` against any pending bug after the patch is deployed. The compound-doc update in this PR will be the first such verification.
- [ ] **"Documented in `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` as Rule 3 (or appropriate rule slot): 'Asserted-unavailability without same-turn attempt is gate evasion.'"** — The doc already documents this as Rule 2 (the ticket author misnumbered it). This PR adds the verbatim-fixture sub-rule and the recurrence-3 entry.

## Files touched

| File | Change |
|---|---|
| `crates/mika-agent/src/agent.rs` | Extend P2, P3, P4 regex; add three unit tests. |
| `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_elided_copula.rs` | New file — three eval scenarios. |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | `pub mod asserted_unavailability_elided_copula;`. |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/asserted_unavailability_elided_copula_*_pre_fix.json` | Three new frozen fixtures. |
| `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` | Add recurrence-3, add verbatim-fixture sub-rule, update Forward Work + Citations. |
| `crates/mika-agent/CLAUDE.md` | Section 6c — update line listing the five patterns to mention the elided-copula + adverb-interposed extensions; add mika#894 alongside mika#862 as the operational citation. |

No schema migration. No CLI surface change. No skill manifest change.

## Estimated effort

Small. ~20 lines of regex/test code in `agent.rs`, ~120 lines of new eval scenarios and fixtures, ~40 lines of compound-doc edits. One PR, one round of CI. The regex-only-with-fixture-only nature means the risk of regression is bounded to the asserted-unavailability guard's eval suite, which is targeted and fast.

## Open questions for the architect

1. Is the premise correction (Premise correction section, top) accepted? If not, the rest of the plan needs to be reframed.
2. Is the adverb-interposition extension `(?:\w+ly )?` appropriate, or should it be a closed adverb-list (e.g. `(?:structurally|currently|presently|literally)?`) to bound false positives more tightly? The closed lis