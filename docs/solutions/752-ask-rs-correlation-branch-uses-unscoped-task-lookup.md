---
module: crates/mika-cli/src/commands/ask.rs
tags: [validation, agent-scope, correlation, ownership, relay, latent-bug, task-lookup]
problem_type: correctness-bug
date: 2026-04-23
issue: 752
---

# ask.rs correlation branch uses unscoped task lookup

## Problem

`mika ask --task-id <uuid>` has two branches: `--task-complete` (mutates the task: marks a callback complete) and the correlation-only path (existence check for observability, then falls through to the normal agent loop with the task ID in session metadata). The correlation path validated the task via `ctx.async_db.get_task(tid)` — the **agent-scoped** primitive whose SQL is `WHERE id = ? AND agent_id = ?`.

That worked for months only because the caller of `mika ask --task-id` was always the task owner. Claude-pilot's permission relay routed through `mika-dev`, which owned every task it dispatched, so the agent filter was a no-op. Commit `8b6df69` (the fix for mika#721) correctly rerouted permission forwarding to `mika-relay` — and from that moment forward, the caller was no longer the owner. The agent filter started rejecting real cross-agent correlation attempts with `Error: Task 'X' not found`. Production observed 65+ denials across the golden-dataset dispatch under milestone #16; tier-1 auto-approval covered enough safe Bash patterns that the PR still shipped, but non-tier-1 commands were silently auto-denied for 42 minutes. Milestone #16 blocked.

The call site even had a comment directly above it — `// Correlation-only path` — contradicting the primitive it called. The comment was right, the call was wrong, and nothing in the type system, linter, or review process could tell them apart because both primitives return `Result<Option<Task>>` with identical shapes. See `docs/solutions/best-practices/correlation-vs-ownership-validator-bug-class-2026-04-23.md` for the bug-class writeup that derives the prevention principles.

## Solution

One-line swap plus bidirectional documentation, no new abstractions.

**Fix:** `crates/mika-cli/src/commands/ask.rs:185` now calls `ctx.async_db.get_task_unscoped(tid)` — the existing unscoped primitive on `AsyncDatabase` that delegates to `Database::get_task_unscoped` (SQL: `WHERE id = ?`, no agent filter). The `--task-complete` branch at line 129 continues to use agent-scoped `get_task` — completion is a state mutation and ownership is load-bearing there.

**Bidirectional doc comments** on `Database::get_task_unscoped` (`crates/mika-agent/src/db.rs`) and `validate_task_exists` (`crates/mika-agent/src/tools/mod.rs`) cross-link the two and spell out the ownership-vs-correlation distinction, including the constraint that tool-path consumers must use the scoped validator while CLI correlation/observability sites must use the unscoped primitive. The doc comments are a tripwire for the next reviewer facing this same choice.

**Regression tests** in `crates/mika-agent/tests/ask_correlation.rs` — placed in `mika-agent` because `mika_agent::test_utils` is `#[cfg(test)]`-gated and not reachable from `mika-cli`'s dev build. The tests use one shared in-memory `Database` wrapped by two `AsyncDatabase` handles (`new_with_agent("agent-a")` plus `.with_agent("agent-b")`) to exercise the actual cross-agent condition, not two disjoint in-memory DBs (which was the semantic bug in the original test draft — same agent on both sides would have trivially returned `Ok(None)` and hidden the real behavior). Three scenarios: unscoped lookup finds the cross-agent task (happy path, proves the fix), scoped lookup rejects it (contrast, proves the fix was necessary), unscoped lookup of a non-existent UUID returns `Ok(None)` (edge, confirms unscoped doesn't suppress real misses).

## Key Decisions

- **Use the existing `get_task_unscoped` primitive; do not introduce a new helper or an `enum AgentScope { Scoped(AgentId), Unscoped }` parameter.** Both alternatives were tempting. Both expand the surface area of the fix from 1 line to ~30 lines and introduce a fresh abstraction for a bug that is structurally about *which primitive to call*, not about *how the primitive is shaped*. The better structural answer (a type-level `AgentScopedTaskId` newtype so correlation call sites can't construct it) is filed as mika#755 and deliberately not in scope here — the relay was live when this was diagnosed and the shipping goal was unblock-first.

- **Keep the completion branch agent-scoped.** Line 129's `get_task` is correct: completing a callback task is a privileged state mutation, and callers from other agents should not be able to complete tasks they don't own. The fix is targeted at the correlation path alone.

- **Place regression tests in `mika-agent/tests/`, not `mika-cli`.** `mika-agent::test_utils::test_helpers` is `#[cfg(test)]`-gated so it's invisible to mika-cli's dev build and invisible to mika-agent's integration-test build. Pulling it into a `test-utils` cargo feature would touch the dependency graph for a two-file fix and contradict "no new abstractions, no new features." A self-contained integration test in `mika-agent/tests/ask_correlation.rs` using only public API is the minimum-blast-radius placement.

- **Amend the fix commit, do not split into fix + test-move.** The original commit message already describes "fix + regression test" as one logical change; the broken test code was never meant to land independently. The local branch had not been pushed, so amend was safe.

## Testing

```
$ cargo test -p mika-agent --test ask_correlation
running 3 tests
test unscoped_lookup_returns_none_for_nonexistent_task ... ok
test scoped_lookup_rejects_cross_agent_task ... ok
test unscoped_lookup_finds_cross_agent_task ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

$ cargo test -p mika-cli
test result: ok. 252 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s

$ cargo clippy -p mika-cli -p mika-agent --all-targets -- -D warnings
(clean)

$ cargo fmt --all -- --check
(clean)
```

## Generalizable Lesson

Any codebase that exposes both a scoped and an unscoped variant of the same lookup (tasks, sessions, messages, facts, audit events) is one routing change away from a latent-bug incident at every call site that didn't document which variant it needed and why. Two practical rules:

1. **Each caller must carry an explicit rationale tied to state-mutation-vs-observation.** Tools and handlers that change state: scoped. CLI correlation, observability, dashboard lookups, external-webhook correlation: unscoped. The rationale belongs in a comment at the call site *or* — stronger — encoded in the primitive's name so a reviewer can't miss it.

2. **Bidirectional doc-comment cross-links on the primitives are structural, not decorative.** Prompt-level conventions ("always use the scoped variant by default") rot; docstrings that name each other by fully qualified path survive renames, moves, and IDE search because they show up in both directions of navigation. This is the same seam as the `feedback_prompt_enforcement_fragile` pattern: structural beats prompt-level when the cost of getting it wrong is asymmetric.

The strongest fix (not taken in this PR) is a type-level newtype — `AgentScopedTaskId` constructible only from within a `ToolContext` — that turns wrong-primitive-selection into a compile-time error. Filed as mika#755. Doc-comment cross-links are the tripwire; the newtype is the guard.

## Follow-up Audit

Grep for other `async_db.get_task(` (and `db.get_task(`) call sites and for each one confirm whether the path mutates state (scoped is correct) or merely observes / correlates (scoped is wrong). Not in scope for this PR. Starting pointers from the existing codebase, to be triaged in a separate ticket:

- `crates/mika-cli/src/commands/ask.rs:129` — `--task-complete` branch. **State-mutating, scoped is correct.**
- `crates/mika-agent/src/tools/mod.rs:328` — `validate_task_exists`, consumed by state-mutating tools. **Scoped is correct for its consumers.**
- `crates/mika-agent/src/skills/executor.rs:570` — skill executor lookup path. Review whether the caller is always the owning agent or whether this is reachable from a relay-style dispatch.
- `crates/mika-agent/src/server/mod.rs:1744` — server handler. Review whether webhook-driven paths ever reach this with non-owner callers.
- `crates/mika-agent/src/task_engine/{engine,dispatcher}.rs` — task engine callers. Review the callback/resume path, which is the most likely second instance of this class.

Test-only call sites (`*_tests`, `#[cfg(test)]` modules) can be ignored for the audit — they construct their own task owner and aren't reachable from relay routing.

## Related

- **`docs/solutions/best-practices/correlation-vs-ownership-validator-bug-class-2026-04-23.md`** — the bug-class knowledge-track doc derived from this incident, with the three prevention principles (name the semantic, bidirectional docstrings, integration test at the pipeline layer) and the `AgentScopedTaskId` newtype proposal.
- **mika#721** — introduced the `mika-relay` agent and rerouted claude-pilot permission forwarding. The routing change was correct but did not exercise the cross-agent task-id correlation path before merge, so this bug activated on production dispatch rather than in review.
- **mika#753** — pipeline-level integration test that exercises relay → task-id correlation end-to-end on the mika#721 surface. Operational-layer prevention.
- **mika#754** — mika-qa produced zero tool calls on a `#751` PR-opened webhook during the same incident window. Investigation ticket; unclear whether same class or different failure mode.
- **mika#755** — the structural tripwire (newtype `AgentScopedTaskId` or rename `validate_task_exists` to match its enforced semantic). Not active work; fires if this class recurs.
- Commit `8b6df69` (claude-pilot routing fix for mika#721) — the caller-context change that activated the latent bug.
