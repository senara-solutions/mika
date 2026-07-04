---
issue: 1725
type: fix
scope: loop-substrate
title: auto-groom convergence failure + grooming-marker gate rigidity
---

# Plan — mika#1725: auto-groom convergence failure + grooming-marker gate rigidity

## Why

Two mechanisms both bite the autonomous loop's dispatch path today. Ticket route-around cost accumulates on every fresh grooming.

**Note on the ticket body's diagnosis:** the body claims the grooming-marker gate uses `Plan: docs/plans/` literal-substring check. Reading the actual code at `crates/mika-agent/src/skills/executor.rs:803-817` (`check_grooming_markers()`) — the check for the plan marker is just `docs/plans/` (no `Plan:` prefix required). The failure mode I actually hit on mika#1723 dispatch was the SECOND check — `second-pass (GROOMED)` requires literal `(GROOMED)` immediately followed by `)`. My initial body carried `second-pass (GROOMED, session fd4c1a14)` — the `,` after `GROOMED` broke the substring match. So mechanism 2 is real, but on the GROOMED verdict marker, not the Plan marker. This plan corrects that scope.

## Mechanism 1: auto-groom architect convergence failure (dev-groom)

**Symptom.** `mika#1723` dispatch at 17:58Z produced parent task `e781006e-ade5-445d-83a3-dc1d005e8288` and callback child `f490c32f-6aa0-4942-b10f-c3e13204f75e`. Callback delivered with `PIPELINE FAILURE: architect convergence did not complete (_iterate_groom_loop returned non-zero). Plan exists on branch but architect verdict is missing.` Parent transitioned to `blocked` at 18:03Z.

**Investigation.** Root cause candidates (mutually non-exclusive):
1. mika-arch first-pass returned ITERATE, second-pass ran before revisions applied
2. mika-arch first-pass returned a paraphrased Disposition the loop didn't parse (per `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`)
3. mika-arch second-pass hit a timeout mid-turn
4. Intermediate step (plan-write, plan-read, session-continuation) failed silently

**Investigation step 1:** grep `/var/log/mika/server.log` for `task_id=e781006e-ade5-445d-83a3-dc1d005e8288` or `f490c32f-6aa0-4942-b10f-c3e13204f75e` OR the `_iterate_groom_loop` return value site. Establish which of the four candidates fired.

**Investigation step 2:** read `skills/bundled/dev-groom/handler.sh` (the `_iterate_groom_loop` implementation) to understand the convergence contract. It expects mika-arch first-pass → apply revisions → mika-arch second-pass → GROOMED. The non-zero return could be from ITERATE without applied revisions, ESCALATE from first-pass, or a session-continuity break.

**Fix shape.** Depends on investigation outcome. Three plausible directions:

- **A. Session-continuity fix.** If the second-pass session doesn't correctly reference the first-pass session, mika-arch has no context for "what changed." Fix: pass `session_id` explicitly on the second-pass invocation.
- **B. Paraphrase tolerance in `_iterate_groom_loop`.** Match arch's own known paraphrase set (`docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`): "Proceed" ≈ READY, "Ratify" ≈ READY, etc.
- **C. Escalate on convergence failure with actionable diagnostic.** Instead of setting parent `blocked` with an opaque `PIPELINE FAILURE`, capture WHICH step failed + WHAT the last arch response was. Enables faster orchestrator-manual fallback.

**Recommendation:** all three at different layers. A + B are code-level fixes; C is defensive observability that speeds recovery on future convergence classes we haven't seen yet.

## Mechanism 2: grooming-marker gate rigidity on parameterized GROOMED verdict

**Actual failure shape (verified against `check_grooming_markers()` at `crates/mika-agent/src/skills/executor.rs:811-812`):**

```rust
let has_groomed_marker = issue_body.contains("second-pass (GROOMED)")
    || issue_body.contains("second-pass (READY, paraphrased GROOMED");
```

The check requires literal `second-pass (GROOMED)` OR literal `second-pass (READY, paraphrased GROOMED` — the first form requires `)` immediately after `GROOMED`. When orchestrator-CC produces `second-pass (GROOMED, session fd4c1a14)`, the `,` breaks the substring match — dispatch rejects with `dispatch_no_grooming_marker` + `missing_signals: ["groomed_verdict"]`.

**Fix shape (structural widening):**

Replace the substring match with a regex tolerating parameterized forms:

```rust
static GROOMED_VERDICT_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match `second-pass (GROOMED)` OR `second-pass (GROOMED, ...)` OR
    // `second-pass (GROOMED — ...)` — the parameter/annotation is optional and
    // any character after GROOMED except a word-continuation is acceptable.
    Regex::new(r"second-pass \(GROOMED[\s\)\.,;:—-]").unwrap()
});
static PARAPHRASED_GROOMED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"second-pass \(READY, paraphrased GROOMED").unwrap()
});

pub fn check_grooming_markers(issue_body: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !issue_body.contains("> - **Branch:**") {
        missing.push("branch_callout");
    }
    if !issue_body.contains("docs/plans/") {
        missing.push("plan_callout");
    }
    let has_groomed_marker = GROOMED_VERDICT_RE.is_match(issue_body)
        || PARAPHRASED_GROOMED_RE.is_match(issue_body);
    if !has_groomed_marker {
        missing.push("groomed_verdict");
    }
    missing
}
```

**Same widening applied to `auto_pull.rs:49` regex:**

```rust
Regex::new(r"(?m)^> - \*\*Grooming history:\*\*.+second-pass \(GROOMED[\s\)\.,;:—-]")
```

**Why regex not more permissive substring:** naked `GROOMED` in the body without the `second-pass (` context would false-positive on prose like "the ticket was groomed" or "GROOMED status pending." The regex anchors to the `second-pass (` prefix + a delimiter after `GROOMED`, which structurally distinguishes verdict-line from prose.

## Acceptance criteria

- **AC1 (Mechanism 1 investigation):** The plan is amended (during implementation) with named root cause from `server.log` for e781006e / f490c32f. The candidate list narrows to ONE of A/B/C/other.
- **AC2 (Mechanism 1 fix):** Whichever of A/B/C applies, `dev-groom`'s `_iterate_groom_loop` no longer returns non-zero on the class of failure that happened at 17:58Z. Regression test uses a fixture that reproduces the specific step failure identified in AC1.
- **AC3 (Mechanism 2 fix — executor.rs):** `check_grooming_markers` accepts `second-pass (GROOMED)`, `second-pass (GROOMED, session fd4c1a14)`, `second-pass (GROOMED — session-id: fd4c1a14)`, `second-pass (GROOMED. Full ratification.)` — verified by unit test with each form.
- **AC4 (Mechanism 2 fix — auto_pull.rs):** Same widening applied at the sibling regex site. Same test set.
- **AC5 (regression — no false positive):** Prose like `"the ticket was GROOMED yesterday"` or `"GROOMED status pending"` does NOT satisfy the check. Unit test asserts absence-of-`second-pass (` context is rejected.
- **AC6 (existing test coverage preserved):** The existing tests at `crates/mika-agent/src/skills/executor.rs:5557+` (`test_grooming_markers_*`) continue to pass without modification. Only new tests are added.
- **AC7 (in-flight tickets unaffected):** Tickets whose body carries the strict canonical `second-pass (GROOMED)` form (no comma parameter) continue to dispatch identically. No regression on cm#56/57/58/59/60 whose grooming-history callouts use the strict form.

## Out of scope

- Renaming `_iterate_groom_loop` or restructuring dev-groom's handler flow (bigger refactor).
- Changing the writer-side canonical form (this is a structural gate fix — writers can produce any of the accepted forms, and orchestrator-CC's current `— session-id:` shape is one accepted form).
- Adding a `parse_paraphrased_disposition()` mika-arch helper library (future ticket if the paraphrase set grows).

## Dependencies

**Blocked by:** none. mika#1719 fix (PR#1726) is UNRELATED — this is a separate substrate class.

**Blocks:** nothing directly. Loop-quality improvement.

**Order relative to Prime #446:** Prime task #446 blocks new autonomous dispatches until mika#1719 fix ships. This ticket is GROOMED now and dispatchable once #446 lifts.

## Verification

```bash
# Unit tests
cargo test -p mika-agent --lib skills::executor::tests::grooming_markers

# Integration: reproduce the mika#1723 dispatch shape with the current form
cargo test -p mika-agent --lib server::ready_label_handler::tests

# Manual: verify the fix accepts orchestrator-CC's actual body format
echo '> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: fd4c1a14' | \
  cargo run --bin grooming-marker-check
```

## References

- `crates/mika-agent/src/skills/executor.rs:803-817` (`check_grooming_markers`)
- `crates/mika-agent/src/auto_pull.rs:43-56` (sibling regex check)
- `crates/mika-agent/src/skills/executor.rs:793` (comment noting canonical shape)
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` (arch paraphrase drift context — mechanism 1 candidate B)
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` (canonical grooming callout discipline)
- mika#919, mika#1108 (the gate origin + observability additions)
- mika#1725 body (original filing with diagnosis correction noted above)
- `feedback_prompt_enforcement_fragile` — the doctrine motivating a structural gate widening over writer-side discipline
