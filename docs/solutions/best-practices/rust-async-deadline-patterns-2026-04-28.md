---
module: rust
tags: [rust, async, tokio, testing, deadline-patterns, grooming, review]
problem_type: best_practice
category: best-practices
ticket: mika#848
date: 2026-04-28
---

# Rust async-deadline patterns + grooming/review lessons from mika#848

## Context

Four meta-lessons surfaced during the mika#848 pipeline (plan → groom → implement → review) that aren't already documented elsewhere. Two are Rust gotchas other agents will repeatedly hit; two are process patterns that the grooming and review steps must explicitly check for. The technical fix itself is documented at `docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md`; this doc compounds the *meta*.

## Guidance

### 1. Use `tokio::time::Instant` (not `std::time::Instant`) for any deadline that tests need to fast-forward through

`std::time::Instant::now()` reads the OS monotonic clock. `tokio::time::pause()` + `tokio::time::advance()` only manipulate tokio's *virtual* clock — they have no effect on `std::time::Instant`.

This matters whenever you write:
1. A deadline check using `Instant::now() >= deadline`, AND
2. A test that wants to drive virtual time forward instead of waiting wall-clock seconds.

Always import the deadline `Instant` from tokio:

```rust
use std::time::Duration;
use tokio::time::Instant;  // <- not std::time::Instant

let deadline = Instant::now() + Duration::from_secs(300);
// ... later ...
if Instant::now() >= deadline { /* deadline gate */ }
```

The eval test pattern that pairs with this:

```rust
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn deadline_during_slow_call() {
    use mika_common::llm::mock::*;
    // MockResponse::Delayed uses tokio::time::sleep so virtual time advances.
    let responses = vec![delayed_response(360_000, text_response("..."))];
    // ... run the agent with a short deadline; the runtime auto-advances ...
}
```

If the agent code uses `std::time::Instant`, the deadline check sees real wall-clock time (which never advances under `pause()`), and the test either hangs or asserts the wrong thing. The first iteration of the mika#848 eval test failed this way before the agent code was switched to `tokio::time::Instant`.

**Where this applies:** any new deadline-shaped pattern in the codebase. Existing precedent: `crates/mika-agent/src/task_engine/engine.rs:735` (`tokio::time::Instant::now() + Duration::from_secs(5)`); `crates/mika-agent/src/teams/engine.rs:1181`. mika#848 added the agent-loop deadline to this list.

### 2. `cfg(any(test, feature = "test-utils"))` doesn't expose items to integration tests

A common Rust idiom for "this function is for tests only":

```rust
#[cfg(any(test, feature = "test-utils"))]
pub fn my_test_helper() { ... }
```

This works for **unit tests inside the same crate** because `cargo test` enables `cfg(test)` on the lib. But **integration tests** in `tests/*.rs` are a *separate crate* that links against the lib's normal compiled output. They do NOT see the lib's `cfg(test)`. So a `cfg(any(test, feature = "test-utils"))`-gated function is invisible to integration tests unless the integration test's crate also opts into `feature = "test-utils"` — which Cargo doesn't allow you to do via a self-dependency.

**The naming-convention contract is the practical alternative.** Make the function `pub` without `cfg` gating, name it explicitly (e.g., `run_agent_with_deadline` next to the production `run_agent`), and document it as test-only in the docstring. Mention that the gate was rejected because integration tests can't see it. Trust naming + docstring for the production-bypass contract; don't pretend you have machine enforcement.

When you actually need machine enforcement, the alternatives are:

- A per-PR CI grep guard (e.g., reject `*_with_deadline` callsites outside `tests/`)
- A separate `*-test-utils` crate that the test crate depends on (heavy)
- A trait-with-private-blanket-impl pattern (only works for some shapes)

mika#848 chose naming + docstring because the test-only function set is small and reviewable.

### 3. Grooming must verify the actual data shape of types named in the plan, not the assumed shape

The mika#848 plan was groomed by mika-arch through two architect passes. The plan stated: "`LoopResult` gains a `DeadlineExceeded` variant" and required no `#[non_exhaustive]` so the compiler enforces match-exhaustiveness across the three outer handlers.

Reality: `LoopResult` was a **struct** with `max_steps_exceeded: bool`, not an enum. The architect didn't grep the actual file; the plan author didn't either. The implementation step caught it (the conflict was unsatisfiable as written) and halted to ask the operator. Operator chose Option A (convert to enum), which was the architect's intent — but the path through that question was friction that grooming should have removed.

**How to apply during grooming:** for every type or function named in a plan that the implementation will modify, the grooming step should `cat` or `gh_read` the current definition before approving. Type-shape mismatches are silent in plans (everyone reads "enum" and pictures one) but loud at implementation time.

The architect already had a `gh_read` tool with `file_view` op (#811, #817). The grooming skill prompt should require its use for any type-name in the plan.

### 4. The same bug class often co-exists at smaller scale near the fix site — review must check the neighborhood, not just the diff

mika#848 fixed the silent-drop variant of an in-flight LLM cancellation bug at the outer 5-min `tokio::time::timeout` wrappers. Adjacent code in the same file (`attempt_continuation_turn`, line ~437 pre-fix) had a 60s `tokio::time::timeout` wrapping a single LLM call — **the same in-flight-cancel bug at smaller scale, plus a pre-existing silent-drop**: that path called `llm.send_message` directly without ever persisting an `llm_calls` row.

The architect's groomed plan included a clause to fix it ("F3c: replace continuation's own `tokio::time::timeout` with deadline-aware wrapper... so in-flight LLM calls during continuation also persist their `llm_calls` row"). The first implementation pass added the deadline clamp but missed the row-persistence — `attempt_continuation_turn` still didn't call `save_llm_call`. The adversarial reviewer caught it.

**How to apply during review:** when a fix targets a specific bug class (silent drops, race conditions, leaking handles), the reviewer should grep for the same class within the surrounding code, not only inside the diff. Reviewer prompts that ask "did the diff fix the bug?" miss "did the same bug exist next door?" Adversarial-style prompts that ask "construct failure scenarios near this code" catch it.

The compounding rule: **every documented bug class in `docs/solutions/runtime-errors/` is a candidate review-grep pattern**. When the bug is "in-flight future cancellation drops persistence," the review pass should grep for `tokio::time::timeout` near `db.save_*` and `await` callsites.

## Why This Matters

- (1) and (2) are concrete Rust gotchas that will repeat. Capturing them once means the next deadline-shaped feature isn't a debug detour.
- (3) and (4) are process gaps that quietly burn time. (3) added one halt-and-ask cycle; (4) added one review-fix cycle. Each is small individually but they compound across PRs.
- The grooming and review skills can each absorb a small change to address (3) and (4) — they're cheap to encode, expensive to keep rediscovering.

## When to Apply

- **Lesson 1 (`tokio::time::Instant`):** any new code that computes a deadline. Default to tokio's variant unless the deadline is genuinely outside any tokio runtime.
- **Lesson 2 (test-only entry points):** any "I want this `pub fn` only for tests" situation. Decide upfront: naming-convention or CI grep guard or separate test-utils crate.
- **Lesson 3 (grooming type-shape verification):** every grooming pass that names a type or function the implementation will modify. Add to `mika-arch-groom-ticket` skill prompt as a required pre-check.
- **Lesson 4 (review-grep adjacent code):** every review pass that touches a documented bug class. Adversarial-reviewer-style prompts already encourage this; surface the explicit grep-pattern derivation step.

## Examples

### Lesson 1: deadline in agent.rs

Before:
```rust
use std::time::{Duration, Instant};
let deadline = Instant::now() + Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS);
```

After (test-compatible):
```rust
use std::time::Duration;
use tokio::time::Instant;
let deadline = Instant::now() + Duration::from_secs(AGENT_TOTAL_TIMEOUT_SECS);
```

Test then can use `start_paused = true` and the runtime auto-advances when the only pending future is a sleep.

### Lesson 2: test-only entry point

Don't:
```rust
#[cfg(any(test, feature = "test-utils"))]
pub async fn run_agent_with_deadline(params: &AgentParams<'_>, deadline: Instant) -> Result<...> { ... }
```

(Invisible to integration tests in `tests/*.rs`.)

Do:
```rust
/// **Test-only entry point** that exposes the `deadline: Instant` parameter.
/// Production callers use [`run_agent`]; this exists only for the eval harness.
/// The `cfg(any(test, feature = "test-utils"))` gate was rejected because
/// Rust integration tests don't see the lib's `cfg(test)`.
pub async fn run_agent_with_deadline(params: &AgentParams<'_>, deadline: Instant) -> Result<...> { ... }
```

### Lesson 3: grooming type-shape check

mika-arch first-pass review on a plan that mentions `LoopResult` should run:

```bash
gh_read file_view "crates/mika-agent/src/agent.rs"
# search for "LoopResult" definition and confirm enum vs struct, fields, derive macros
```

before approving the plan's prescription. If the plan says "add variant X to enum Y" but Y is a struct, the disposition is `ITERATE` with a note to convert (decision goes to operator).

### Lesson 4: review-grep for bug class

mika#848 fixes silent drop of in-flight HTTP requests. Reviewer grep:

```bash
git diff main -- crates/mika-agent/src/agent.rs | grep -B5 -A5 "tokio::time::timeout"
# look for any other tokio::time::timeout wrapping LLM calls or DB writes
# in the same file — the bug class likely co-locates
```

Found: `attempt_continuation_turn` line 437 had the same pattern. The fix scope expanded to cover it.

## Related

- `docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` — the technical fix this doc compounds the meta-lessons of
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — sister doc on grooming-step rigor
- `docs/solutions/best-practices/eval-harness-test-defaults-and-di-pattern.md` — eval harness conventions that pair with Lesson 2
- `crates/mika-agent/CLAUDE.md` § Agent Loop — implementation-level documentation
