# Canonical body-callout writer on GROOMED (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`; v1 scope `yes`.
**Sub-PR sequence:** **Sixth sub-PR of mika#1271.**
  - PRs #1273 (`f2bef21`), #1274 (`1eb5a03`), #1275 (`30917d7`), #1276 (`96fd2281`), #1277 (`9c20096e`), companion `mika-platform e9c7060`.
  - **This PR**: canonical body-callout writer wired on both GROOMED success paths.
  - **Sub-PR 7 (terminal)**: retire Class D shim + remove feature flag.

## Goal

Add `_write_canonical_callout <stage> <session_id>` to `dispatch-lib.sh`. Wire it into both GROOMED success paths in `_iterate_groom_loop` (READY → GROOMED and ITERATE → GROOMED) so that — once the existing pilot-owns-architect path retires in sub-PR 7 — dispatch-lib still produces a body callout that passes the Pin B / `check_grooming_markers` dispatch gate.

Distinct from `_verify_and_write_body_callout` (mika#1123) which writes a RECOVERY callout that deliberately does NOT carry a `second-pass (GROOMED)` marker because it cannot verify the architect's verdict from branch state alone. This new forward-path writer DOES carry the verdict — it runs only after `_iterate_groom_loop` has personally invoked the architect's second-pass and read back `Verdict: GROOMED`.

## Stage labels

Two stage labels drive distinct Grooming-history line shapes. Both forms include `second-pass (GROOMED)` to satisfy the dispatch-gate `has_verdict` regex.

| Stage | First-pass disposition | Grooming-history line |
|-------|------------------------|------------------------|
| `ready-to-groomed` | `READY` | `first-pass (READY) → second-pass (GROOMED) — session-id: <id>` |
| `iterate-to-groomed` | `ITERATE` (then revised) | `first-pass (ITERATE) → revised → second-pass (GROOMED) — session-id: <id>` |

The history line carries forensic detail (which path produced the verdict + the architect session-id for log correlation). The other two callout lines (Branch + Plan-committed-at-SHA) are identical to the recovery shim's output for downstream parser compatibility.

## Implementation

### New helper: `_write_canonical_callout <stage> <session_id>`

Defined between `_escalate_groom` and `_iterate_groom_loop` in `dispatch-lib.sh`. Behavior:

1. **Validate inputs.** WORKTREE_DIR must exist; REPO/ISSUE_NUM/BRANCH must be set. Unknown stage label → return 1.
2. **Find issue-scoped plan file.** Same pattern as `_iterate_groom_loop` (issue-number-scoped, > 500 bytes, most-recent first).
3. **Idempotency check.** Reuse the three-signal pattern from `_verify_and_write_body_callout` — if `has_branch + has_plan + has_verdict` all > 0 in the existing body, the dispatch gate already passes; skip writing.
4. **Compose callout block.** Three markdown blockquote lines: `> - **Branch:** ...`, `> - **Plan:** ... (committed on branch @ <sha>)`, and the stage-specific Grooming-history line.
5. **Write.** Fetch current body, prepend the callout block + blank line, `gh issue edit --body-file`. Idempotent on retry because the second invocation will hit the idempotency check.

### Two call sites in `_iterate_groom_loop`

```bash
# READY → second-pass → GROOMED branch
GROOMED)
    echo "iterate_groom_loop: converged on GROOMED for $REPO#$ISSUE_NUM (session $session_id)" >&2
    _write_canonical_callout "ready-to-groomed" "$session_id" || \
        echo "info: canonical callout write non-fatal failure — Class D recovery (mika#1123) still runs downstream" >&2
    _cleanup_iterate_findings
    return 0
    ;;
```

Same shape on the ITERATE → revised → second-pass → GROOMED branch with stage `iterate-to-groomed`. Both call sites use `|| echo ...` so callout-write failure is non-fatal — the Class D recovery shim still runs in the downstream `_run_claude_pilot` post-flight path, so a missing canonical callout falls through to the existing recovery surface rather than failing the dispatch.

### Why non-fatal

Per the dual-write doc on `_verify_and_write_body_callout` (sub-PR 5 inherited it): body callouts can be written by two paths — (1) the organic LLM writer in dev-groom step 18, (2) structural recovery via the Class D shim. This new forward-path writer is a THIRD source — but as long as ANY of the three writes succeeds, the dispatch gate passes. Making the canonical writer fatal would convert a transient `gh issue edit` failure into a dispatch halt, which is a regression vs. the current state. Sub-PR 7 retires path (1); paths (2) and (3) remain. If sub-PR 7 also retires path (2), this writer's failure mode must be promoted to fatal — but that is sub-PR 7's call to make, not this PR's.

## Acceptance criteria

- [ ] **AC1:** `_write_canonical_callout <stage> <session_id>` is defined in `skills/bundled/_shared/dispatch-lib.sh`.
- [ ] **AC2:** Two stage labels supported: `ready-to-groomed` and `iterate-to-groomed`. Unknown stage labels return non-zero.
- [ ] **AC3:** Required env validation — missing `WORKTREE_DIR`, `REPO`, `ISSUE_NUM`, or `BRANCH` returns non-zero without writing.
- [ ] **AC4:** Idempotency — the writer's source includes the same three-signal check (`has_branch`/`has_plan`/`has_verdict`) used by `_verify_and_write_body_callout`. Skips writing when all three are already present.
- [ ] **AC5:** Two call sites in `_iterate_groom_loop` — one in each GROOMED branch. Zero call sites on any ESCALATE branch.
- [ ] **AC6:** Non-fatal contract — both call sites use `|| echo ...` so callout-write failure does NOT halt the iterate loop. The downstream `_run_claude_pilot` Class D recovery shim catches the gap.
- [ ] **AC7:** Source-shape — writer emits the canonical 3-line callout block including the literal `second-pass (GROOMED)` marker so the Pin B / `check_grooming_markers` regex in `executor.rs` matches.
- [ ] **AC8:** Pre-existing test failure count (6) unchanged. 17 new structural assertions pass.

## Verified invariants from prior sub-PRs (still hold)

- `_escalate_groom` call count = 3 (one per ESCALATE branch), unchanged.
- `_cleanup_iterate_findings` call count = 2 (GROOMED-only preservation invariant), unchanged.
- ESCALATE branches still write structured `PIPELINE FAILURE` markers per sub-PR 5.
- No plan-on-origin coupling between architect passes (verified in sub-PR 4).
- `.iterate/` directory ownership exclusive to dispatch-lib.

## What does NOT ship in this sub-PR

- **Class D body-callout shim retire** (`_verify_and_write_body_callout`) — sub-PR 7.
- **Feature flag removal** (`MIKA_DISPATCH_USE_ITERATE_LOOP`) — sub-PR 7.
- **mika#1272** (paraphrased dispositions) — separate ticket.
- **Format changes** — the canonical writer mirrors the existing recovery shim's 3-line shape for downstream parser compatibility. Any format-level evolution (e.g., adding an architect-model field, adding a timestamp) is out-of-scope.

## Test plan

17 new structural assertions in `test-dispatch-lib.sh`:

- Writer definition exists (`declare -f _write_canonical_callout`)
- 2 call sites in `_iterate_groom_loop` (grep count)
- Stage labels `ready-to-groomed` and `iterate-to-groomed` present at the two call sites
- Non-fatal marker `canonical callout write non-fatal failure` present at call sites
- Unknown stage label returns non-zero
- Missing `WORKTREE_DIR` returns non-zero
- Missing `REPO` returns non-zero
- Writer source contains `> - **Branch:**` line
- Writer source contains `committed on branch @` line
- Writer source contains `second-pass (GROOMED)` marker
- Writer source includes `session-id: ${session_id}` in history line
- READY-to-GROOMED history line shape (`first-pass (READY) → second-pass (GROOMED)`)
- ITERATE-to-GROOMED history line shape (`first-pass (ITERATE) → revised → second-pass (GROOMED)`)
- Idempotency signals: `has_branch`, `has_plan`, `has_verdict` all present in source

## Provenance

- mika#1271 parent ticket, milestone#26.
- Sequence: PRs #1273 → #1274 → #1275 → #1276 → #1277 → **this**.
- Architect contract: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (`flip` / `(i) Retire` / `yes`).
- mika#1123 — Class D drift recovery; the three-signal idempotency check pattern reused.
- Pin B in `executor.rs::check_grooming_markers` — the dispatch gate this writer satisfies.
- Friend-peer sharpenings from sub-PR 4 (preserve-on-ESCALATE, session-id symmetry, `.iterate/` ownership) honored unchanged.
