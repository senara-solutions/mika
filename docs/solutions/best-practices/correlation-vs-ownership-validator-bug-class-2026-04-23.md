---
module: mika-agent
date: 2026-04-23
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - Writing or reviewing task/entity lookup calls in agent-tool code
  - Introducing a new agent that forwards messages on behalf of another (relay, proxy, coordinator)
  - Routing decisions that change which agent calls an existing helper
  - Code review on helpers whose name describes existence but whose implementation enforces ownership
tags:
  - validation
  - agent-scope
  - correlation
  - ownership
  - relay
  - latent-bug
  - structural-vs-documentation
  - post-mortem
---

# Correlation-vs-ownership: a latent validator bug class

## Context

On 2026-04-23 the `#339` golden-dataset dispatch under milestone #16 produced 65+ relay denials (`Error: Task 'X' not found`) in the claude-pilot log at `/var/log/claude-pilot/19049f32-d8a9-46ff-86f4-09d5f73875fc.log`. The PR still shipped (tier-1 auto-approval covered enough safe-pattern Bash commands), but non-tier-1 Bash calls were silently auto-denied for 42 minutes. Milestone blocked.

Root cause, verified by code inspection: `crates/mika-cli/src/commands/ask.rs:183` called `ctx.async_db.get_task(tid).await` (the **agent-scoped** lookup) inside a code path the comment immediately above at line 182 explicitly labels `// Correlation-only path`. The intent was correlation (existence check for observability metadata); the implementation was ownership (agent-scoped validation that rejects if the task belongs to a different agent).

Worked by accident for months because the caller of `mika ask --task-id` was always the task owner — claude-pilot's permission relay routed to `mika-dev`, which owned every task it dispatched. The bug was structurally present but operationally invisible. Last night's routing fix (four direct commits across mika-platform / mika-cloud / mika-skills / claude-pilot-py swapping `.claude/claude-pilot.json` args from `mika-dev` to `mika-relay` per mika#721's original intent) changed the caller from the task owner to a non-owner agent. The latent validator mismatch surfaced instantly.

The comment was right. The call was wrong. The name was the only thing agreeing with the comment (`validate_task_exists` — sounds like existence, enforces ownership).

## Guidance

When a helper's name describes one semantic (existence, lookup, correlation) but its implementation enforces a stricter semantic (ownership, agent-scope, permission), it is a latent bug waiting for a caller-context change. Three principles for preventing and catching this class:

**1. Name the semantic, not the surface.** `validate_task_exists` describes existence; it actually checks ownership. Rename or, better, introduce a type-level guard:

```rust
// Option A — rename to match enforced semantic:
pub async fn validate_task_owned_by_caller(...) -> Result<Task, ToolOutput>

// Option B — type-level guard, strongest:
pub struct AgentScopedTaskId(String);
impl AgentScopedTaskId {
    // Only constructible from within tool-context code paths
    pub(crate) fn new_from_tool_context(id: String, _ctx: &ToolContext) -> Self { ... }
}
pub async fn validate_task_exists(db: &AsyncDatabase, id: AgentScopedTaskId) -> ... { ... }
```

CLI correlation call sites cannot construct `AgentScopedTaskId` (no `ToolContext`) so they must call an unscoped primitive directly. Wrong-function selection becomes a compile-time error, not a doc-comment-skippable choice.

**2. When you can't rename yet, bidirectional-cross-link the docstrings.** Both the ownership-enforcing helper and the correlation-only primitive should name each other by fully-qualified path in their doc comments. A developer reading either function sees the other exists. Still prompt-level (skippable), but meaningfully better than one-sided.

```rust
/// Agent-scoped task lookup — for intra-agent state mutation only.
///
/// For correlation-only / observability existence checks that cross agent
/// boundaries, use [`crate::db::Database::get_task_unscoped`] instead.
pub(crate) async fn validate_task_exists(...) { ... }

/// Cross-agent task lookup — for correlation and observability only.
///
/// Does NOT enforce agent ownership. For state-mutating agent tools that
/// require the caller own the task, use [`crate::tools::validate_task_exists`].
pub fn get_task_unscoped(&self, id: &str) -> Result<Option<Task>> { ... }
```

**3. When introducing an agent that relays on behalf of another, add an integration test exercising the relay → validator path end-to-end.** Task owned by agent A, invoked as agent B, assert no `task_not_found`. This is the operational analog of the request-side well-formedness test pattern from mika#338 D9 — lock in the regression at the pipeline layer, not just the unit layer.

## Why This Matters

The cost of missing this class is asymmetric:

- **Mostly invisible.** Because the bug only fires when caller-context changes (new agent, new routing config, new delegation path), it passes unit tests, integration tests, and staging review. The failure mode is "works perfectly until it catastrophically doesn't."
- **Degrades silently under partial failure.** Tier-1 auto-approval in relay-style pipelines means some fraction of commands get through. The failure looks like "mostly working" rather than "broken" — operator trust survives, but review quality doesn't.
- **Recurs structurally, not accidentally.** Every long-lived codebase accumulates helpers whose names mean one thing and whose implementations enforce another. The mismatch is discovered by routing changes, refactors, or new agent introductions — events that happen on predictable cadence.

The doc-comment half-measure is a tripwire, not a fix. The type-level guard (option B above) is the structural answer. Files as tracked follow-up (mika#755 in this codebase) when the shipping moment calls for a one-liner.

## When to Apply

- Reviewing any new agent that delegates to another (permission relays, coordinators, proxy agents) — before merging, confirm the delegation path doesn't assume agent ownership where existence semantics are documented
- Naming a new validation helper — make the name match the strictest semantic it enforces, not the loosest
- Receiving a PR that introduces an `Err(task_not_found)` or equivalent — verify whether the lookup should be ownership-scoped or correlation-only for each caller
- During a refactor that changes which agent calls an existing helper — grep for the helper's call sites and verify the ownership semantic is still correct at each one

## Examples

### The incident that prompted this (mika#752, 2026-04-23)

```rust
// crates/mika-cli/src/commands/ask.rs, line 182-206 (buggy form):

// Correlation-only path: validate task exists, emit deprecation warning if needed
match ctx.async_db.get_task(tid).await {            // ← agent-scoped! wrong for correlation
    Ok(Some(task)) => { /* ... */ }
    Ok(None) => { anyhow::bail!("Task '{}' not found.", tid); }
    Err(e) => { /* ... */ }
}
```

The fix (one-liner):

```rust
match ctx.async_db.get_task_unscoped(tid).await {   // ← correct: correlation-only primitive
    // ...
}
```

### A different example of the same class (illustrative)

```python
# Buggy: function named "exists" but filters by user_id
def issue_exists(issue_id: str, user: User) -> bool:
    return db.query(Issue).filter(id=issue_id, assignee=user).exists()

# A correlation caller (webhook payload, notification system, search indexer)
# will get False when the issue exists but was assigned to a different user.
# Same class: name says existence, implementation enforces ownership.
```

Fix: split into `issue_exists(id)` and `user_owns_issue(id, user)` — name each semantic.

## Related

- **mika#752** — the bug this learning derives from (ask.rs correlation branch used agent-scoped validator). Primary fix: one-line swap to `get_task_unscoped` + bidirectional cross-linked doc comments + regression test.
- **mika#753** — integration test on mika#721's surface (cross-agent relay → task-id pipeline test). Prevention at the operational layer.
- **mika#754** — mika-qa received `#751` PR-opened webhook and produced zero tool calls; unclear whether same class or different failure mode. Investigation ticket.
- **mika#755** — structural naming tripwire (p3): rename `validate_task_exists` or introduce `AgentScopedTaskId` newtype if this bug class recurs. Filed as explicit tripwire, not active work.
- **mika#721** — introduced the mika-relay agent whose routing change exposed this latent bug. The ticket did not exercise the cross-agent task-id correlation path before merge; mika#753 adds that integration test.
- `feedback_prompt_enforcement_fragile.md` (auto memory [claude]) — structural > prompt-level enforcement. The bidirectional doc comments are prompt-level; the type-level guard is structural. This learning sits at the seam.
- `feedback_verify_before_claiming.md` (auto memory [claude]) — I violated this twice during this diagnosis (claimed "log should show X" without checking, framed surgical fix as "pragmatism over clarity" without principle-checking). The friend review corrected both.
- `docs/plans/CONVENTIONS.md` — SHA-pinning amendment protocol; relevant because the decomposition friend prescribed (direct DB call, no new helper, bidirectional docstrings) was chosen against the tempting refactor (new helper or enum parameter).
