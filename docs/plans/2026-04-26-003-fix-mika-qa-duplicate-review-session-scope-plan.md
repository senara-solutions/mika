# Plan — Fix mika-qa duplicate PR review via required-tools-gate retry (mika#821)

**Ticket:** [mika#821](https://github.com/senara-solutions/mika/issues/821) — `fix(mika-qa): session-scope pr_review_posted to prevent duplicate reviews via required-tools-gate retry`
**Branch:** `fix/mika-qa-duplicate-review-session-scope`
**Type:** bug, p2-normal, agent-core

## Problem

mika-qa submits two `APPROVED` PR reviews on the same commit ~40-45 seconds apart, both within a single agent session. Reproduced N=2 in consecutive PRs (#819, #820).

**Causal chain (from ticket body and `mika/docs/solutions/runtime-errors/mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md`):**
1. Turn 1: model calls `run_gh pr diff` → `run_gh pr review --approve` (review #1) → EndTurn, never calls required `qa_pr_view`.
2. Required-tools gate (post-condition #3) rejects EndTurn because `qa_pr_view` was missing.
3. Retry creates a new turn with fresh `ToolContext` — `pr_review_posted` AtomicBool reset to `false`.
4. Turn 2: model calls `qa_pr_view` (gate satisfied) AND re-runs the review path → `pr review --approve` (review #2 submitted).

Two existing defenses both bypassed:
- `ToolContext.pr_review_posted` AtomicBool — per-turn-scoped, reset on new turn.
- PR review early-accept (#695, post-condition #3b) — skips guards #4–#7 only, NOT #3 (required-tools). Doesn't help when the trigger IS guard #3.

## Fix framing (post-architect-review reframe)

**Fix B (extend early-accept to short-circuit guard #3) is the root-cause fix.** It directly closes the reproduced failure path: the early-accept guard's intent (#695) was "primary action completed; further continuation risks duplicate submissions," but its scope was incomplete — it only skipped guards #4-#7 while the trigger was guard #3. Making the scope match the intent eliminates the gate-retry mechanism that produces the duplicate.

**Fix A (session-scoped dedup map) is defense-in-depth for distinct retry sources** that don't go through the required-tools gate at all: max-steps continuation, manual re-prompt, webhook re-delivery, future post-condition additions. These paths are structurally valid but unverified in production. Fix A guards them prophylactically.

Both ship together (per ticket). Sequencing in implementation: Fix B first (smaller, ships defense-in-depth on its own and closes the reproduced bug), Fix A second.

## Current code shape (verified against worktree main @ 0beb5645)

| Concern | Location |
|---|---|
| `ToolContext.pr_review_posted: &'a AtomicBool` field | `crates/mika-agent/src/tools/mod.rs:133` |
| Tool-layer duplicate check (`is_pr_review_command` + AtomicBool load) | `crates/mika-agent/src/skills/builtin_handlers.rs:1592-1606` |
| Tool-layer success record (AtomicBool store on success) | `crates/mika-agent/src/skills/builtin_handlers.rs:1627-1631` |
| Per-turn AtomicBool instantiation sites | `agent.rs:1623, 2438, 2769`; `tools/send_message.rs:196, 266, 318, 366, 418`; `tools/toggle_skill.rs:276, 324`; `server/investigate.rs:749`; `test_utils.rs:56, 130, 158, 190` |
| Early-accept (#3b) decision | `crates/mika-agent/src/agent.rs:892-906` |
| `has_successful_pr_review()` helper | `crates/mika-agent/src/agent.rs:3160` |
| Existing tests for early-accept helper | `crates/mika-agent/src/agent.rs:6208-6253` |
| Required-tools-gate code (post-condition #3) | `crates/mika-agent/src/agent.rs:~830-887` (block ending in `continue;` at line 887) |
| **AppState definition (confirmed)** | `crates/mika-agent/src/server/state.rs:60-92` (uses `Arc<DashMap<...>>` for `a2a_broadcasters` — DashMap is the established pattern in this struct) |
| **`end_session()` server-side callsites** | `task_engine/dispatcher.rs:275, 431, 674, 830`; `teams/engine.rs:1155`; `tools/delegate_task.rs:341` |

The early-accept logic at line 892-898 reads:
```rust
let skip_remaining_guards =
    matches!(response.stop_reason, LlmStopReason::EndTurn)
        && has_successful_pr_review(&all_tool_summaries);
```
And `skip_remaining_guards` only conditionalizes guards #4-#7 (line 913+). The required-tools gate runs ABOVE this, so it cannot be skipped by the early-accept as currently structured.

## Approach

### Fix B — Extend early-accept to short-circuit guard #3 (ship first)

In the required-tools-gate block at `agent.rs:~830-887`, before the `request.messages.push(...)` re-prompt logic, add a check: if `has_successful_pr_review(&all_tool_summaries)` returns true, accept EndTurn and skip the gate's `continue`.

```rust
// Inside the required-tools-gate block, before the re-prompt logic:
if has_successful_pr_review(&all_tool_summaries) {
    info!(
        step,
        label = mode.label(),
        "PR review already posted — accepting EndTurn (skipping required-tools gate #3)"
    );
    // Fall through; let post-condition chain continue (skip_remaining_guards will skip #4-#7).
} else {
    // ... existing re-prompt logic ...
    continue;
}
```

~5-10 lines. Aligns the early-accept's reach with its semantic intent. Closes the reproduced bug on its own.

### Fix A — Session-scoped `pr_reviews_posted` map (defense-in-depth)

**Storage type: `Arc<DashMap<String, HashSet<String>>>` on `AppState`.**

Justification:
- DashMap is already the established pattern in `AppState` (see `a2a_broadcasters: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>` at `server/state.rs:91`). Match it for consistency.
- `DashMap::entry(session_id).or_default().insert(key)` holds the per-bucket lock during the inner `HashSet` mutation — no separate Mutex needed.
- Outer key: `session_id` (existing field on `ToolContext`). Inner set: PR dedup keys.

**ToolContext shape change.** Add `pub pr_reviews_posted: Option<&'a Arc<DashMap<String, HashSet<String>>>>`. Optional so test sites can pass `None` without forcing each test to construct a server-scoped map.

**Silent-degradation guard (architect Finding 2).** Add `debug_assert!(ctx.pr_reviews_posted.is_some(), "pr_reviews_posted must be threaded for production pr review calls")` at the entry to the `pr review` branch in `run_gh`. Fires in test/debug builds when a production path accidentally exercises `pr review` without the map. Release builds silently degrade to per-turn defense (acceptable — Fix B is the root-cause fix; Fix A is belt-and-braces).

**PR dedup key derivation.** `gh pr review` accepts `[<number> | <url> | <branch>]` as the first positional after `pr review`, plus an optional `--repo owner/repo`.

```rust
fn make_pr_dedup_key(args: &[String], repo: Option<&str>) -> String {
    let positional = args.get(2).map(String::as_str).unwrap_or("__current_branch__");
    format!("{}|{}", repo.unwrap_or("__default__"), positional)
}
```

**`__current_branch__` fallback assumption (architect Finding 3).** mika-qa's qa-review skill consistently passes `pr review <pr_url> --approve` (verified in N=2 reproductions: both used URL form). The `__current_branch__` fallback only fires when the agent calls `gh pr review` with no positional argument — which mika-qa never does. To make this assumption explicit and self-documenting:

```rust
// Comment in make_pr_dedup_key:
// __current_branch__ fallback fires only for `gh pr review --approve` with no
// positional, which mika-qa never emits (it always passes the PR URL). If this
// fallback ever produces same-key collisions for legitimately different PRs in
// one session, the fix is to normalize via `gh pr view --json number` — file as
// follow-up if it surfaces.
```

No `debug_assert!` here because that would panic on a legal `gh` invocation; the comment + follow-up trigger is the right shape.

**`run_gh` algorithm (replaces builtin_handlers.rs:1592-1606 and 1627-1631):**
```rust
let pr_dedup_key = if is_pr_review_command(&gh_args.args) {
    debug_assert!(
        ctx.pr_reviews_posted.is_some(),
        "pr_reviews_posted must be threaded for production pr review calls"
    );
    Some(make_pr_dedup_key(&gh_args.args, gh_args.repo.as_deref()))
} else {
    None
};

// Session-scope check (Fix A) — primary defense in production.
if let (Some(key), Some(map)) = (&pr_dedup_key, ctx.pr_reviews_posted) {
    if map.get(ctx.session_id).map(|set| set.contains(key)).unwrap_or(false) {
        return ToolOutput::error(/* duplicate_pr_review */);
    }
}

// Per-turn check (existing #695 defense, retained for tests passing None).
if pr_dedup_key.is_some() && ctx.pr_review_posted.load(Ordering::Acquire) {
    return ToolOutput::error(/* duplicate_pr_review */);
}

let output = spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?").await;

if !output.is_error && let Some(key) = pr_dedup_key {
    ctx.pr_review_posted.store(true, Ordering::Release);
    if let Some(map) = ctx.pr_reviews_posted {
        map.entry(ctx.session_id.to_string()).or_default().insert(key);
    }
}
```

**Eviction strategy (architect Finding 5 — committed): server-side eviction at all 6 `end_session()` callsites in production server code.**

The 6 callsites (verified):
- `crates/mika-agent/src/task_engine/dispatcher.rs:275` (callback)
- `crates/mika-agent/src/task_engine/dispatcher.rs:431` (heartbeat)
- `crates/mika-agent/src/task_engine/dispatcher.rs:674` (reflection)
- `crates/mika-agent/src/task_engine/dispatcher.rs:830` (reminder)
- `crates/mika-agent/src/teams/engine.rs:1155` (team child)
- `crates/mika-agent/src/tools/delegate_task.rs:341` (delegate)

At each site, immediately after the existing `db.end_session(&session_id).await` call, evict the corresponding `AppState.pr_reviews_posted` entry:

```rust
state.pr_reviews_posted.remove(&session_id);
```

Mechanically these are 6 one-line additions. CLI callsites (`crates/mika-cli/src/commands/{chat,ask}.rs`, `tui/commands/handlers.rs`) do NOT need eviction — they don't run server-side and don't have `AppState` access. CLI sessions are short-lived and the process exits afterward.

**Why not a wrapper helper.** A `state.end_session(&session_id)` helper that wraps both the DB call and the map eviction would centralize the eviction. Considered and rejected: the dispatcher and team-engine callsites pass `AsyncDatabase` directly (not `AppState`), so threading `AppState` through them just for this helper would touch more lines than 6 inline `state.pr_reviews_posted.remove(&session_id)` calls. The 6-callsite shape is the minimal change. If a future ticket adds a 7th `end_session()` caller without the eviction, the leak is bounded (entries are tiny strings, agent recycles regularly) — acceptable trade-off.

## Files

| Action | File | Approx. lines |
|---|---|---|
| Modify | `crates/mika-agent/src/tools/mod.rs` | +3 (add `pr_reviews_posted` field + DashMap import) |
| Modify | `crates/mika-agent/src/skills/builtin_handlers.rs` | +30 (algo above + `make_pr_dedup_key` helper + comment) |
| Modify | `crates/mika-agent/src/agent.rs` | +12 (Fix B early-accept inside required-tools-gate) + thread the new field through 3 ToolContext construction sites at 1623/2438/2769 |
| Modify | `crates/mika-agent/src/server/state.rs` | +3 (add `pr_reviews_posted: Arc<DashMap<String, HashSet<String>>>` field) |
| Modify | `crates/mika-agent/src/server/handlers.rs` | +3 (clone field into ToolContext at the message handler) |
| Modify | `crates/mika-agent/src/server/investigate.rs` | +1 (pass `None` for `pr_reviews_posted`) |
| Modify | `crates/mika-agent/src/task_engine/dispatcher.rs` | +4 (eviction at 4 sites) + thread AppState if not already passed |
| Modify | `crates/mika-agent/src/teams/engine.rs` | +1 (eviction at line 1155 vicinity) |
| Modify | `crates/mika-agent/src/tools/delegate_task.rs` | +1 (eviction at line 341 vicinity) |
| Modify | `crates/mika-agent/src/tools/send_message.rs` | +5 (5 test sites pass `None`) |
| Modify | `crates/mika-agent/src/tools/toggle_skill.rs` | +2 (2 test sites pass `None`) |
| Modify | `crates/mika-agent/src/test_utils.rs` | +4 (4 fixture sites pass `None`) |
| Add tests | `crates/mika-agent/src/skills/builtin_handlers.rs` (mod tests) | +5 new tests |
| Add tests | `crates/mika-agent/src/agent.rs` (mod tests) | +1 new test for Fix B |

Net diff estimate: ~85-110 lines of source change + ~150 lines of test code.

**Discovery items resolved during planning** (no longer deferred to implementation):
1. AppState confirmed at `server/state.rs:60-92`. Already uses `Arc<DashMap<...>>` for `a2a_broadcasters` — match the pattern.
2. `end_session()` server-side callsites enumerated above (6 sites). DB-only call from `AsyncDatabase` at `async_db.rs:584` and `Database` at `db.rs:4974` — both unchanged; eviction lives at the callsite, not in the DB.
3. Required-tools-gate `continue;` at `agent.rs:887`. Fix B's early-accept hook lands before the `request.messages.push(...)` calls (lines 868-887).

Implementer should verify the dispatcher already has `AppState` (or equivalent) access at the 4 dispatcher callsites — if not, threading it adds modest churn but no design change.

## Tests

Inline in `crates/mika-agent/src/skills/builtin_handlers.rs` mod tests (extend existing `run_gh` tests):

1. **`test_run_gh_session_scope_blocks_cross_turn_duplicate`** — construct a fake `Arc<DashMap>`. Call `run_gh pr review <url> --approve` once (session map populated). Reset the per-turn AtomicBool (simulates new turn). Call again with same args — assert `duplicate_pr_review` error returned, exec_count remains 1.
2. **`test_run_gh_session_scope_allows_different_pr_same_session`** — call `run_gh pr review <url1> --approve` then `run_gh pr review <url2> --approve` in the same session — both succeed, both recorded.
3. **`test_run_gh_session_scope_allows_same_pr_different_session`** — call `run_gh pr review <url> --approve` for session A, then for session B (separate session_ids) — both succeed.
4. **`test_run_gh_required_tools_gate_retry_blocks_second_review`** — directly simulate the bug chain. Mock turn 1: post review (session map populated, atomic flips, EndTurn). Reset atomic only (mimics turn 2 fresh ToolContext). Call review again — assert blocked.
5. **`test_run_gh_no_session_map_falls_back_to_atomic`** — pass `pr_reviews_posted: None`. First call succeeds, atomic flips. Second call in same turn (atomic still true) — assert `duplicate_pr_review`. Confirms the per-turn defense remains functional in test contexts. (`debug_assert!` disabled for this test via release-mode-style guard, OR the test runs `pr diff` instead of `pr review` to skip the assert path — implementer's call.)

Inline in `crates/mika-agent/src/agent.rs` mod tests:

6. **`test_required_tools_gate_skipped_after_pr_review_success`** — set up `all_tool_summaries` with a successful `run_gh pr review`. Set `required_tools` to include a tool not called. Run the post-condition logic — assert no re-prompt fires, EndTurn accepted.

Existing tests must continue to pass: `has_successful_pr_review` tests at `agent.rs:6208-6253` (unchanged); all existing `run_gh` mod tests (signatures may change to include the new field; pass `None`).

## Acceptance criteria (from ticket, restated)

- [ ] At most one `APPROVED` review per `(session_id, pr_url)` tuple. Verifiable via `gh api repos/owner/repo/pulls/<n>/reviews` after a mika-qa session.
- [ ] All existing `run_gh` tests pass.
- [ ] 6 new tests above pass.
- [ ] `cargo clippy --all-targets` clean.
- [ ] `cargo fmt --check` clean.
- [ ] Post-deploy, the next 3 PRs reviewed by mika-qa receive exactly one APPROVED review each (regression check on real traffic).

## PR description framing (per architect Finding 1)

The PR description must lead with: **Fix B is the root-cause fix; Fix A is defense-in-depth for distinct retry sources.** Reviewers should not apply equal scrutiny to both — Fix A's surface is larger but its risk profile is "guards a hypothetical class," whereas Fix B's surface is small and directly closes the N=2 reproduced bug.

## Out of scope (from ticket)

- Auditing other per-turn defenses for the same vulnerability shape.
- Changing the qa-review skill's `required_tools` list.
- Modifying the qa-review skill prompt to emphasize `qa_pr_view` first.
- Auto-merge logic changes.

## Risks and mitigations

- **R1: AppState modification touches a hot type.** Mitigation: keep the field optional on `ToolContext` (`Option<&'a Arc<DashMap<...>>>`) so call sites that don't have it pass `None`. The change to AppState itself is purely additive (one new field, derive(Clone) preserved).
- **R2: `make_pr_dedup_key` form-aliasing miss (number vs URL for same PR).** Mitigation: Fix B's `has_successful_pr_review` summary scan catches this (it pattern-matches on tool summaries, not URL strings). N=2 reproduction shows the model uses URL form consistently.
- **R3: `__current_branch__` fallback intra-session collision.** Mitigation: documented assumption that mika-qa always passes an explicit PR identifier; comment in code triggers a follow-up if the assumption breaks.
- **R4: DashMap concurrent contention.** Mitigation: per-bucket locking; entries are ~once-per-session writes. No contention expected.
- **R5: Cleanup gap if a future `end_session()` caller forgets eviction.** Mitigation: bounded leak (string keys, small string sets); agent process recycles. If a 7th caller appears without eviction, file a follow-up to centralize via a wrapper.
- **R6: `debug_assert!` panic in tests that exercise `pr review` without threading the map.** Mitigation: tests for `run_gh` are the primary callers; the test fixtures all pass `None` explicitly. The assert fires only in production-shaped tests that should be threading the map anyway. If it surfaces a real test gap, the assertion message points the implementer at the right fix.

## Sequencing

1. **Fix B first** (smaller, lower-risk, ships defense-in-depth on its own — closes the reproduced bug). One file (`agent.rs`), ~12 lines + 1 test.
2. **Fix A second** (ToolContext field, AppState field, run_gh algorithm change, threading through 14+ sites, eviction at 6 callsites). Larger surface but additive.
3. Add tests inline.
4. Run `cargo test -p mika-agent`, `cargo clippy --all-targets`, `cargo fmt --check`.
5. Open PR cross-referencing #821, #695, #818. PR description leads with Fix B = root-cause, Fix A = defense-in-depth (per architect Finding 1).
6. Post-merge: monitor next 3 mika-qa runs for AC verification.
