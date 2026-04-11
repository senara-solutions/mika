# Simplify fabricated-action-claim guard tests

**Priority:** P3
**File:** `crates/mika-agent/src/agent.rs`
**Issue:** #308

## Problem

The `detect_fabricated_action_claim` function has 11 unit tests for a 5-line function
that performs two regex matches with an AND condition. Three tests exercise the same
code path (fast-path short-circuit when `github.com/` is absent):

- `test_detect_fabricated_action_claim_no_github_url` -- no `github.com/` in text
- `test_detect_fabricated_action_claim_no_github` -- no `github.com/` in text
- `test_detect_fabricated_action_claim_empty` -- empty string, no `github.com/`

These are functionally identical from the function's perspective. The `plain_repo_url`
test is distinct (has `github.com/` but no resource suffix).

## Recommendation

Collapse the three redundant "no github.com/" tests into one. Keep distinct negative
cases (`no_action_verb`, `plain_repo_url`) since they test different branches. This
reduces 11 tests to 9 with no coverage loss.

The positive-path tests are fine -- each covers a different URL fragment pattern or
verb pattern and documents the supported detection surface.

## Not actionable

The overall guard implementation (agent loop block, regexes, prompt changes) is already
minimal. It follows the exact same structure as the three existing EndTurn guards --
no YAGNI violations, no unnecessary abstraction, no premature generalization. The
two-regex approach is simpler than a combined pattern would be. The fast path is
appropriate. No changes recommended outside the test consolidation.

The markdown-link false-negative gap noted in `review-308-fabricated-action-guard.md`
is a correctness concern, not a simplicity concern -- addressed there, not here.
