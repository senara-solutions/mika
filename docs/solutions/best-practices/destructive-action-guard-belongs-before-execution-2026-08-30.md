---
module: agent-core
tags: [destructive-action, guard-placement, idempotency, fail-closed, endturn-guard, pre-execution-gate, grounding]
problem_type: structural-guard
category: best-practices
related_issues: [mika#1646, mika#1331, mika#1645, mika#1682, mika#1798]
---

# A guard against a destructive action has to run before the action, and fail toward not doing it

## Problem

mika-dev closed PR #1644 twice in nine minutes on the same fabricated rationale
("duplicate of mika#1638 — content identical"). Between the two closes, a human
had reopened the PR and posted the file diff disproving the claim. The second
close replayed the first one's text verbatim.

The obvious framing — "the agent ignored evidence" — is not the useful one. Two
structural facts made the replay possible, and each has a general shape.

**The second close came from a different execution context.** It was triggered
by a deferred webhook replay at 11:12:11Z, not by a continuation of the turn
that produced the first close at 11:08:54Z. Nothing in memory connected them.

**A close that succeeds looks exactly like a close that was wanted.** There is
no error, no retry, no alert. The system returns a plausible state and the
overwritten intent leaves no trace anyone thinks to look for.

## What we reached for first, and why it was wrong

The ticket's groomed plan placed the fix as an **EndTurn guard**, by analogy
with its two siblings: assert-grounded (mika#1331) and equivalence-claim
(mika#1645). Both live in `evidence/guards.rs`, are consulted from the EndTurn
arm of the agent loop, and work by rejecting the assistant's text and
re-prompting.

The analogy holds right up to the point where it matters. Those guards catch a
**sentence**. This defect is a **tool call**. `gh pr close 1644` leaves at step
3 of the tool loop; the EndTurn arm runs after the whole loop. By then the PR is
closed, and the guard's only remaining power is to comment on an accomplished
fact — which is precisely the failure being fixed.

**The generalizable rule: a guard's placement is set by when the damage lands,
not by which existing guard it most resembles.** Similar predicates can need
completely different enforcement points. Copying the sibling's wiring because
the sibling's *shape* matches is how a guard ends up structurally unable to do
its job while passing every test written for it.

The correct site was already in the codebase and already documented. `run_gh`
carries a chain of pre-subprocess gates (mika#1196, mika#1682, mika#1167), and
the Layer 4 comment in `tool_execution/dispatch.rs` states the routing rule
outright: builtins like `run_gh` have no `data_grade`, so *their gate lives in
the handler*. `validate_pr_ready_undraft_scope` (mika#1682) was a line-for-line
patron — async, takes `&ToolContext`, returns `Result<(), ToolOutput>`, logs a
refusal event, and refuses before any side effect.

## Idempotence by intention, not by accident

"Check the state before acting" is the reflex answer, and it is not enough.
Between the read and the write, someone can reopen. The window is small and the
founding incident lived in it.

What is actually needed is that **the second execution knows it is a second
execution**. That is a property of the record, not of the process:

```rust
// Scoped to the AGENT — not the turn, not the session.
db.find_recent_destructive_actions(agent_id, noun, number, window_secs)
```

Scoping this to the session would have found nothing on 2026-06-29 and waved
the second close through, because the replay ran in its own session. Any repeat
detection built on in-memory turn state is defeated by exactly the mechanism
that produces the most dangerous repeats: retries, replays, and restarts —
contexts that share no memory with the first attempt, which is *why* they repeat.

## Which way to fail, and the fact that it is two directions

The guard fails in **opposite directions** at two stages, and collapsing them
into one policy breaks it either way:

| Stage | Direction | Why |
|---|---|---|
| **Detection** — is this argv a destructive close? | fail-**open** | A gate that cannot tell what it is looking at must not become a blanket refusal of `gh`. Scope stays bounded to `pr close` / `issue close`. |
| **Grounding** — is this recognized close founded? | fail-**closed** | Missing read, unreadable history, persistence disabled, DB error — every one is a refusal. |

The fail-closed half **inverts** the policy of its own sibling: `assert_grounded`
is deliberately "lean-narrow fail-open" (no resource ref extracted → no fire).
That is right for a fabrication guard, where a false positive costs a spurious
re-prompt. It is wrong here, and the asymmetry of costs is the whole reason:

> A ticket left open in error is visible and gets corrected. A ticket closed in
> error drops out of the count and nobody goes looking for it.

The default follows the cost asymmetry. When one error mode is self-correcting
and the other is silent, the guard leans toward the self-correcting one — even
when that means refusing work that was probably fine.

A fail-closed refusal is only tolerable if it is **actionable**, so every
refusal body carries `{error, doctrine, target, reason, remedy}` and names the
unblocking gesture. The `turn_calls.is_empty()` branch exists for the same
reason: "no read of the target" and "no tool calls recorded at all" have the
same symptom and different remedies, and reporting the first when the second is
true sends the agent into a loop re-reading a target whose read can never be
observed.

## Two things that quietly defeated the gate, found by re-reading the diff

Both were fail-**open** holes — the gate stopped firing rather than firing
wrongly, which is the direction that leaves no evidence:

**A boolean flag in the value-flag list.** `--delete-branch` was listed among
flags that consume a following argument. `gh pr close --delete-branch 1644`
therefore skipped past `1644`, found no target number, and returned `None` —
the action was not recognized at all, so the gate never ran. A parsing table
that mixes boolean and value-taking flags fails silently in exactly the argv
shapes an author does not think to test.

**A substring number match.** `input.contains("1644")` is also satisfied by
`16440`. The SQL side had been anchored on the JSON quoting; the in-memory
predicate had not. Anchor both on the same thing (`"1644"` with quotes) or they
drift.

Neither breaks a test written from the happy path. Both were found by asking of
each predicate: *what input makes this return the permissive answer when it
shouldn't?*

## Checks worth stealing

- **Does the guard run before the damage?** If a re-prompt is the enforcement
  mechanism, the action has already happened.
- **Can the guard's own subject satisfy it?** `gh pr close 1644` must not ground
  `gh pr close 1644`. The satisfaction predicate needs a state-reading verb, not
  just a matching reference.
- **Does repeat detection survive a restart?** If not, it does not detect the
  repeats that matter.
- **Is naming a cause the same as showing one?** "Duplicate of #1638" is a
  citation-shaped sentence with nothing checkable in it. Requiring evidence means
  requiring something a reviewer can verify against the artifact — a file list, a
  diff, a read-back state. Admitting a bare `#N` reference would have made the
  guard accept the exact comment that caused the incident.

## Files

- `crates/mika-agent/src/evidence/guards.rs` — pure predicates (detection,
  grounding, evidence citation, repeat acknowledgment, window parsing)
- `crates/mika-agent/src/skills/builtin_handlers.rs` —
  `validate_destructive_action_grounding`, in `run_gh`'s pre-subprocess chain
- `crates/mika-agent/src/db.rs` — `find_recent_destructive_actions` (agent-scoped,
  windowed, anchored on the serialized argv)
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-dev/destructive_action_thread_reground.md`
  — PR #1644 timeline replay

## Related

- mika#1331 — assert-grounded (the sibling whose fail-open policy this inverts)
- mika#1645 — equivalence-claim grounding, the emission-side half of this incident
- mika#1682 — `validate_pr_ready_undraft_scope`, the placement patron
- mika#1798 — Layer 4 guardrail, which states the builtin-gates-live-in-the-handler rule
