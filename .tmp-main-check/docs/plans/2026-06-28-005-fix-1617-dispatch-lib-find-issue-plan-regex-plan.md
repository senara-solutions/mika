# Plan: fix(dispatch-lib): widen `_find_issue_plan` with tier-3 broad content scan

**Ticket:** mika issue#1617

## Problem

`_find_issue_plan` in `dispatch-lib.sh` uses a two-tier discovery strategy:

1. **Tier 1 (filename):** glob `*-${ISSUE_NUM}-*-plan.md` — fast, exact.
2. **Tier 2 (anchored header):** grep first 20 lines for four prefixes (`**Ticket:**`, `**Issue:**`, `ticket:`, `issue:`) followed by `mika ... #${ISSUE_NUM}`.

When `/ce:plan` produces a filename with a sequential daily counter (e.g., `-001-` instead of `-1617-`) AND uses frontmatter/header shapes that don't match the four-prefix union, both tiers miss. The dispatch fails with `PIPELINE FAILURE: _find_issue_plan returned empty`.

Five occurrences this month (N=5). Founding incident: mika-cloud#134 (`2026-06-28-001-fix-console-agent-image-tag-plan.md` — no `-134-` slot, no `issue:`/`ticket:` line in first 20 lines).

## Solution

Add a **tier-3 broad content scan** that catches the remaining shapes `/ce:plan` has been observed to produce, without requiring template-side coordination.

### Tier 3: broad issue-number reference in header zone (first 50 lines)

After tiers 1 and 2 miss, scan `*-plan.md` files (>500 bytes, most-recent first) for the issue number in any of these patterns within the first 50 lines:

- **`#${ISSUE_NUM}\b`** — bare hash-prefixed reference. Covers: `mika#N`, `(#N)` in H1, `Closes #N`, `issue #N`, `senara-solutions/mika#N`, etc.
- **`(issue|ticket|number|id):\s*${ISSUE_NUM}\b`** — YAML frontmatter key with bare numeric value (no `#` prefix). Covers: `issue: 1617`, `number: 1617`, etc.

**Why 50 lines (not 20)?** Tier 2's 20-line zone was chosen to exclude body prose that quotes other tickets' `**Ticket:**` headers (the mika#1421 self-test regression). Tier 3's patterns are less structured (bare `#N`), so the zone must be wide enough to catch issue references that appear below YAML frontmatter + H1 + summary (often lines 15–30) but narrow enough to exclude body discussion. 50 lines covers the typical plan preamble (frontmatter + header + problem statement) while excluding implementation details where cross-references to other issues are common.

**False-positive discipline:** The 50-line zone is the primary guard. A plan that mentions `#1617` in its first 50 lines but is actually for a different issue would false-positive. This is acceptable because: (a) plans rarely cross-reference other issues in their header zone, (b) tier 3 only fires when both tiers 1 and 2 miss (the common case is a plan for THIS issue that just wasn't named right), and (c) `sort -r` returns the most-recent file first, which is almost always the correct one for a just-dispatched groom.

### Tier priority

Tiers are evaluated in strict order: 1 → 2 → 3. First match wins. This preserves the existing behavior for plans that match tier 1 or 2 — tier 3 is additive, never overrides.

## Files to change

### 1. `skills/bundled/_shared/dispatch-lib.sh` — `_find_issue_plan()`

**Add tier-3 scan after the existing tier-2 loop (after line ~1469):**

```bash
# Tier 3: broad issue-number reference in header zone (first 50 lines).
# Catches plan shapes where the issue number appears in a non-standard
# format — parenthesized in H1, bare `#N` in summary, YAML `number: N`,
# etc. Wider zone (50 lines vs tier-2's 20) to cover preamble sections
# that sit below frontmatter. (mika#1617, N=5 founding incidents.)
while IFS= read -r candidate; do
    [ -r "$candidate" ] || continue
    if head -n 50 "$candidate" 2>/dev/null \
        | grep -qE "(#${ISSUE_NUM}\b|(issue|ticket|number|id):[[:space:]]*${ISSUE_NUM}\b)"; then
        printf '%s' "$candidate"
        return 0
    fi
done < <(find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | sort -r)
```

**Update the function's doc comment** to document the three-tier hierarchy.

### 2. `skills/bundled/_shared/dispatch-lib.sh` — error messages (lines ~1089, ~1094)

Update the `PIPELINE FAILURE` messages to mention all three tiers instead of just "no filename match ... and no header-line match in first 20 lines":

```
no filename match *-${ISSUE_NUM}-*-plan.md, no anchored header match (first 20 lines),
and no broad issue-number reference (first 50 lines)
```

### 3. `skills/bundled/_shared/tests/test_find_issue_plan.sh` — new test cases

Add tests for tier-3 discovery:

1. **Positive: bare `#N` in H1 (line 1–5)** — plan with `# Fix for (#1617)` in H1, no `**Ticket:**` prefix, no issue number in filename. Tier 3 should match.

2. **Positive: YAML `number: N` without `#`** — plan with `number: 1617` in frontmatter. Tiers 1–2 miss; tier 3 matches.

3. **Positive: `Closes #N` in summary (line ~10)** — plan with `Closes #1617` in the problem summary. Tier 3 catches.

4. **Negative: `#N` in body past line 50** — plan where the only mention of the issue number is on line 60+. Tier 3's 50-line zone must reject it (same discipline as tier 2's 20-line zone for the mika#1421 regression).

5. **Priority: tier 1 wins over tier 3** — plan with issue number in filename AND bare `#N` in body. Tier 1 returns; tier 3 never runs.

6. **Priority: tier 2 wins over tier 3** — plan with `**Ticket:** mika issue#N` in first 20 lines AND bare `#N` on line 40. Tier 2 returns; tier 3 never runs.

### 4. `skills/bundled/_shared/test-dispatch-lib.sh` — update Test 16 comment

Update the Test 16 header comment to mention tier 3 if it tests `_find_issue_plan` shapes.

## Out of scope

- Standardizing `/ce:plan` filename output (out of our control — marketplace plugin)
- Changing the 500-byte size threshold
- Changing tier-1 or tier-2 behavior (additive only)
- Adding a tier-4 for whole-file scan (too high false-positive risk)

## Risk assessment

- **Low risk.** Tier 3 is additive — tiers 1 and 2 are unchanged. Existing plans that match tier 1 or 2 continue to work identically.
- **False-positive risk** is bounded by the 50-line header zone. The only scenario: a plan for issue X mentions `#Y` in its first 50 lines, and we're searching for issue Y, and no plan actually matches Y via tiers 1–2. This is rare (plans don't cross-reference other issues in their preamble) and the most-recent-file ordering further reduces it.
- **Test coverage** is extended to validate all three tiers plus zone-boundary negative cases.

## Acceptance criteria

1. `_find_issue_plan()` locates plan files using bare `#N` references or YAML `number: N` keys within the first 50 lines, when tiers 1 and 2 miss.
2. Tier 3 only fires after tiers 1 and 2 have been exhausted — first match wins.
3. Plans mentioning the issue number only past line 50 are NOT matched by tier 3.
4. All existing tier-1 and tier-2 tests continue to pass unchanged.
5. Test 16 in `test-dispatch-lib.sh` passes with updated tier-3 coverage.

## Verification

1. Run the updated test suite: `bash skills/bundled/_shared/tests/test_find_issue_plan.sh` — all assertions pass.
2. Run the dispatch-lib integration tests: `bash skills/bundled/_shared/test-dispatch-lib.sh` — Test 16 passes.
3. Manual verification: create a plan file matching each of the N=5 incident shapes and confirm `_find_issue_plan` discovers it.
