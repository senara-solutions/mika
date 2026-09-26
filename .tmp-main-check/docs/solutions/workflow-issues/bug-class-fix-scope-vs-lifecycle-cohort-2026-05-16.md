---
module: workflow
tags: [scope, follow-up, code-review, lifecycle, cohort-framing, ticket-vocabulary]
problem_type: workflow_pattern
category: workflow-issues
related_issues: [1149, 1150]
---

# Frame Review-Found Adjacent Issues as a Forward-Looking Lifecycle Cohort, Not a Retroactive Patch

## Context

A bug fix's `/ce:review` often surfaces **parallel holes in the surrounding wiring** — paths that are not the panic path the fix closed, but exhibit the same failure class. For mika#1149 (supervised the TUI agent-worker tokio::spawn so panic surfaces as a visible TUI error), the review produced:

- 1× P1: agent-switch failure path silently drops the next message (same bug class, different trigger).
- 4× P2: post-crash send guard, missing Quit on agent-switch, restart+switch identity split, shutdown_initiated encapsulation.

The natural temptation is to either (a) widen mika#1149's scope and ship them all together, or (b) file each P-finding as its own follow-up ticket. Both have failure modes:

- (a) **widening** breaks dispatch-scope discipline ("mika#1149 fixes the panic path"); the PR description drifts away from the issue body's contract; reviewers lose the anchor they signed off on.
- (b) **splitting per-finding** loses the cohort framing. The P1 is the *lead item* of a lifecycle-hardening cohort, not a peer to be groomed in isolation. Reviewing F2 (post-crash send) without F1 (switch-failure dead channel) and F5 (shutdown_initiated encapsulation) produces three smaller fixes that drift apart and re-relitigate the same wiring decisions.

## Guidance

When `/ce:review` surfaces a P1 plus 2+ P2s that compose into a coherent failure pattern around the same lifecycle/state machine the original fix touched:

1. **Land the original PR with its original scope.** Apply only safe_auto cleanups (tests, doc updates, log additions, trivial deletions). Don't widen the diff.
2. **File ONE follow-up ticket bundling P1 + N×P2** as a named **lifecycle-hardening cohort**.
3. **Title forward-looking, not retroactive.** Use "<surface> lifecycle hardening" (e.g., *"tui: harden agent-worker lifecycle wiring around the mika#1149 supervisor"*), not "complete mika#1149" or "follow-ups for #1149". The framing signals: the original fix was correct for its scope; this is the next iteration on a shared concern, not a patch on a half-finished job.
4. **Lead with the cohort thesis in the ticket body.** Open with the shared root pattern, the shared fix surface, and the shared test discipline. Then enumerate the findings as F1..Fn. List F1 as the lead item explicitly; mention that it composes with F2..Fn rather than being a standalone P1 bug.
5. **Keep follow-up scope tight to the cohort's seam.** Other reviewer findings (P3 cleanups, advisory items, adjacent unguarded code in other crates) go in the new ticket's "Out of scope" section as separate-ticket suggestions. Don't pile them into the cohort just because they were found in the same review.

## Why This Matters

Cohort framing preserves three things:

- **Architectural coherence in review.** A reviewer evaluating F1 alone sees a small null-check fix. The same reviewer evaluating F1+F2+F3+F4+F5 as one PR sees the shared state-machine seam (`AgentWorker { agent_tx, shutdown_initiated, worker_crashed }`) and can sign off on the encapsulation (F5) once instead of relitigating it on each per-finding PR.
- **Operator trust in the original PR.** When the next dispatch happens after merge of the original PR and the operator sees mika#1150 in the queue with title *"complete mika#1149"*, they reasonably ask "what's incomplete about it?" — undermining confidence in what was actually correct work. *"TUI worker lifecycle hardening"* answers the implicit question: this is the forward-looking next step, not a retroactive band-aid.
- **Grooming agent reasoning.** mika-arch (the grooming reviewer) sees the cohort framing in the ticket body and recognizes that F1's "fix" should be evaluated against the same encapsulation choice (F5) rather than as a standalone null-check. The grooming pass converges faster.

The retroactive-patch framing also misuses the ticket vocabulary established in CLAUDE.md § Ticket vocabulary: *milestone* (single-repo grouping), *project* (cross-repo or sprint-scoped grouping), *sub-issue* (parent-child link only). A cohort of 5 P-findings is structurally a *grouping*, not a *sub-issue of the parent fix*. Filing as a peer ticket with cohort framing respects the vocabulary; filing as a sub-issue of mika#1149 implies the parent is incomplete.

## When to Apply

Use this pattern when:

- `/ce:review` finds 1× P1 + 2+ P2 that share a state machine, file surface, or lifecycle seam touched by the original fix.
- The findings are not exploitable by the same trigger as the original bug — they're parallel holes in adjacent paths.
- The original PR's scope was anchored to a specific issue body's "Proposed fix" section, and widening would break the dispatch contract.

**Don't apply** when:

- The findings are P0 / actively exploitable — fix in the original PR or block the merge.
- The findings are unrelated to the original bug class (filed as separate tickets per their own surface).
- The original PR was a small standalone fix and the cohort would be 70% of the original scope — at that point, widen the original PR instead.

## Examples

### Bad — retroactive-patch framing

```
Title: complete mika#1149: missed adjacent silent-drop paths
Body:
  CE review on the supervisor fix found 5 additional findings that weren't
  caught in the original PR. This ticket cleans them up.
  - F1: agent-switch dead channel
  - F2: post-crash send guard
  - ...
```

Problems: operator reads it as "the original fix was incomplete"; reviewer evaluates each F-item individually; grooming agent looks for "what was missed" rather than "what coherent improvement comes next"; vocabulary implies parent-child where peer-cohort is correct.

### Good — forward-looking lifecycle cohort

```
Title: tui: harden agent-worker lifecycle wiring around the mika#1149 supervisor

## Why
mika#1149 supervised the TUI agent-worker tokio::spawn, closing the panic-path
silent drop. /ce:review on the PR surfaced a coherent cohort of lifecycle
wiring holes around the supervisor that re-introduce or compose with the same
silent-drop class. They were intentionally deferred to keep mika#1149's scope
tight ("fix the panic path") and to land the supervision primitive cleanly.
This ticket bundles them as a single lifecycle-hardening cohort because they
share the same root pattern (the agent_tx/agent_rx/worker_crashed/
shutdown_initiated state machine), the same fix surface, and the same test
discipline.

## In scope
### F1 — Agent-switch failure path leaves a dead channel with no /restart affordance (P1, lead item)
...
### F2 — Post-crash send_message_with_thinking silently enqueues to dead worker (P2)
...
### F5 — shutdown_initiated has no encapsulation (P2)
...

## Out of scope
- The 6 unguarded tokio::spawn sites in crates/mika-gateway/src/github.rs.
  File separately for server-side gateway supervision; the supervision
  primitive is reusable there.
- Panic-payload secret scrubbing (low severity, narrow trigger).
- Supervisor upper-bound timeout for non-LLM deadlocks (low severity).
```

The cohort framing makes the shared state-machine seam visible (F1 + F2 + F5 all touch the same flag transitions), gives F5's encapsulation choice the chance to be evaluated against all four use sites in one review pass, and signals to operators that mika#1149 is complete and this is the next iteration on a shared concern. Filed as mika#1150 on 2026-05-16.

## Related

- [supervise-tokio-spawn-with-shutdown-flag](../best-practices/supervise-tokio-spawn-with-shutdown-flag-2026-05-16.md) — the technical pattern that mika#1149 introduced and mika#1150 hardens.
- CLAUDE.md § Ticket vocabulary — the milestone/project/sub-issue contract this pattern composes with.
- `feedback_implementation_scope_bundling.md` (auto memory) — "During implementation, file separate ticket for adjacent improvements; never silently fold into in-progress commit." This pattern is the next iteration: when adjacent improvements form a coherent cohort, file ONE follow-up rather than N separate tickets.
