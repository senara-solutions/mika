# Plan: fix(spirit) — self-deadlock in `handle_message` rate-limit-trip audit

**Ticket:** mika#1723
**Type:** bug (p1-important)
**Parent substrate ticket:** mika#1719 (stays open, scope narrowed to invariant 4 after this merges)

## Problem

`handle_message` (`crates/mika-agent/src/server/handlers.rs:245-255`) self-deadlocks on
the `rate_limit_audit_last` DashMap shard under a narrow, recurring condition. This is the
root cause of the mika-spirit HTTP wedge fingerprint in mika#1719 (n=3, 2026-07-01 →
2026-07-03).

### Root cause (verified against the Rust Reference)

```rust
let should_emit = match state.rate_limit_audit_last.get(&agent_label) {
    Some(last)
        if now.duration_since(*last.value()) < RATE_LIMIT_TRIP_AUDIT_INTERVAL => false,
    _ => {
        state.rate_limit_audit_last.insert(agent_label.clone(), now);  // ← self-deadlock
        true
    }
};
```

Temporaries in a **match scrutinee** live until the end of the *entire* match expression,
through all arm bodies (Rust Reference — temporary scopes). When `.get()` returns
`Some(Ref)`, the shard **read** guard is alive during the `_ =>` arm. On the
`Some`-but-stale path, the arm's `.insert()` requests an **exclusive** guard on the same
shard the current thread already holds shared. DashMap 6 uses `parking_lot::RwLock`
(non-reentrant `RawRwLock`) per shard — read-held + write-requested on the same thread
blocks forever. This is exactly the pattern `clippy::significant_drop_in_scrutinee` exists
to catch.

`RATE_LIMIT_TRIP_AUDIT_INTERVAL` = `Duration::from_secs(10)` (`handlers.rs:40`);
`rate_limit_audit_last: Arc<DashMap<String, Instant>>` (`state.rs:103`).

### Trigger condition (all four must hold)

1. Agent busy — `try_lock_owned()` on `agent_state.agent_lock` fails (`handlers.rs:228`).
2. Rate-limit trip path entered — throttled audit branch (`handlers.rs:244`).
3. A prior trip for this `agent_label` already seeded the DashMap key.
4. `RATE_LIMIT_TRIP_AUDIT_INTERVAL` has elapsed since that prior trip.

**Not a trigger:** first-ever trip per label — `.get()` returns `None`, no `Ref` exists,
the `_ =>` arm's `.insert()` proceeds clean. The requirement of a *stale existing entry*
revisited under concurrency (not merely a busy agent) explains the n=3 rarity.

### Wedge amplification

One thread self-deadlocks (holds read on shard X, waits for write on shard X — exclusive
request queued). Every subsequent concurrent request for the same `agent_label` hashes to
the same shard, tries `.insert()`, and queues behind the pending exclusive request. The
worker pool saturates at `num_cpus` deep threads (per the mika#1719 invariant table).

## Fix

Extract the value from the `Ref` **before** the match, dropping the read guard at the
`.map()` call. `Instant` is `Copy`, so `.map(|r| *r.value())` yields an owned `Instant`
and the scrutinee holds no guard across any arm body:

```rust
let should_emit = match state
    .rate_limit_audit_last
    .get(&agent_label)
    .map(|r| *r.value())
{
    Some(last) if now.duration_since(last) < RATE_LIMIT_TRIP_AUDIT_INTERVAL => false,
    _ => {
        state.rate_limit_audit_last.insert(agent_label.clone(), now);
        true
    }
};
```

### Accepted benign race — DO NOT "fix"

The fix introduces a TOCTOU seam on `get → insert`: two threads may both observe stale and
both insert. Worst case = **one duplicate `rate_limit_trip` audit event**. This is
explicitly accepted. A reviewer MUST NOT restructure back into a guard-holding shape, nor
switch to DashMap's `.entry()` API — `.entry()` re-serializes the same-shard path and
reintroduces a guard-held-across-mutation hazard. The one-duplicate-row cost is strictly
cheaper than the deadlock it replaces. (See ticket § "Accepted benign race".)

## Approach

### Change 1 — Extract a pure, directly-testable throttle helper

`handle_message` is a full Axum handler; standing up a real `AppState` + agent lock +
DB just to exercise a 10-line throttle decision is disproportionate and slow. Extract the
decision into a free function in `handlers.rs`, keyed only on the DashMap, label, `now`,
and interval:

```rust
/// Decide whether to emit a throttled `rate_limit_trip` audit event, recording
/// `now` as the last-emit instant when it returns `true`.
///
/// Guard-drop discipline (mika#1723): `.get(...).map(|r| *r.value())` extracts the
/// `Copy` `Instant` and releases the shard read guard BEFORE the match, so the
/// `.insert()` in the stale/absent arm never requests a write guard on a shard this
/// thread already holds shared. DashMap shards are non-reentrant `parking_lot::RwLock`;
/// holding a `Ref` across the `.insert()` self-deadlocks.
///
/// Accepted benign race: two threads may both observe stale and both insert, costing at
/// most one duplicate audit row. DO NOT restructure into a guard-holding or `.entry()`
/// shape to close this seam (mika#1723).
fn should_emit_rate_limit_audit(
    last_emitted: &DashMap<String, std::time::Instant>,
    agent_label: &str,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> bool {
    match last_emitted.get(agent_label).map(|r| *r.value()) {
        Some(last) if now.duration_since(last) < interval => false,
        _ => {
            last_emitted.insert(agent_label.to_string(), now);
            true
        }
    }
}
```

Replace the inline `match` at `handlers.rs:245-255` with:

```rust
let should_emit = should_emit_rate_limit_audit(
    &state.rate_limit_audit_last,
    &agent_label,
    now,
    RATE_LIMIT_TRIP_AUDIT_INTERVAL,
);
```

This preserves observable behavior exactly (same throttle semantics), fixes the deadlock,
and makes the fix unit-testable without HTTP/DB scaffolding.

### Change 2 — Add a `#[cfg(test)] mod tests` to `handlers.rs`

`handlers.rs` currently has no inline test module. Add one with:

1. **Guard-drop no-deadlock under concurrency (primary regression test).** Seed the map
   with a stale timestamp for one label (`now - 60s`), then spawn N threads
   (`N = 16`, above typical `num_cpus`) that all call `should_emit_rate_limit_audit` for
   that same label concurrently. Assert **all threads complete within a bounded timeout**
   (e.g. 5s). Under the pre-fix guard-holding shape this hangs (futex_wait); under the fix
   it completes. Use `std::thread` + a `std::sync::mpsc` / `JoinHandle::join` with a
   watchdog, or `tokio::time::timeout` around a `tokio::task::spawn_blocking` fan-out — the
   helper is sync, so `std::thread` is the most faithful reproduction of the shard-contention
   path. Exactly one thread should observe the stale entry as first-past-the-post and return
   `true`; the assertion is on *completion*, not on the count (the accepted race makes the
   count non-deterministic under extreme timing — see test 3).

2. **Throttle semantics preserved.** Fresh map: first call for a label returns `true`
   (absent key → insert). Immediate second call returns `false` (within interval). A call
   with `now` advanced past the interval returns `true` again (stale → re-insert).

3. **Stale-entry revisit returns true (trigger-condition unit).** Seed a stale entry
   (`now - interval - 1s`); a single call returns `true` and updates the stored instant to
   `now`. This is the exact path that self-deadlocked pre-fix, exercised single-threaded.

Note the accepted race in the concurrency test comment so a future maintainer does not
"tighten" it into asserting exactly-one-true, which would be flaky.

### Change 3 (verification-only, not committed to the repo) — 10-line deadlock repro

The ticket's standalone `dashmap` repro is a manual verification aid, not a repo artifact.
The plan runs it out-of-tree during implementation to confirm the pre-fix hang and post-fix
completion (see Verification Contract), but does **not** add it to the crate — the inline
concurrency test (Change 2, test 1) is the committed regression guard.

## Files touched

| File | Change |
|------|--------|
| `crates/mika-agent/src/server/handlers.rs` | Extract `should_emit_rate_limit_audit` helper; replace inline `match`; add `use dashmap::DashMap;` if not already in scope; add `#[cfg(test)] mod tests` with 3 tests. |

Single-file change. No schema change, no public API change, no new dependency (`dashmap`
is already a workspace dep).

## Out of scope (explicitly deferred)

- **Invariant 4 / `/health` code=000 during the wedge.** `handle_health` never touches
  `rate_limit_audit_last`, so this site does not explain the atomic-only handler failing
  to respond while workers were idle. mika#1719 **stays open** after this closes, scope
  narrowed to invariant 4 (accept-stranding via tokio LIFO-slot / driver-side effect /
  other). Do not attempt it here.
- **Clippy structural enforcement** (`significant_drop_in_scrutinee` +
  `await_holding_invalid_types` for DashMap guard types). Separate companion PR per
  `feedback_implementation_scope_bundling` — this PR fixes the one live site; the lint that
  prevents the *class* is its own change. (Referenced in ticket § Related.)
- **Prime task #446 unblock.** Filing/merging this ticket does not itself lift the
  restart-without-fix block; #446 lifts when this fix **merges AND deploys**. Operational
  sequencing, not a code deliverable here.

## Verification Contract

**Automated (committed):**
- `cargo test -p mika-agent --lib server::handlers` — the 3 new tests pass; the concurrency
  test completes well within its timeout (proves no deadlock).
- `cargo build -p mika-agent` — clean.
- `cargo clippy -p mika-agent --all-targets` — clean; specifically no
  `significant_drop_in_scrutinee` at the rewritten site.
- `cargo fmt --check` — clean.

**Manual (during implementation, not committed):**
- Run the ticket's 10-line `dashmap` snippet as-is → confirm it hangs (`strace` shows
  `futex_wait`); confirm the shard self-block reproduces the mechanism.
- Apply the `.map(|r| *r.value())` shape to the snippet → confirm it completes.

**Regression framing:** Pre-fix, test 1 (concurrency) hangs indefinitely and the test run
times out. Post-fix it completes. That delta is the regression guard.

## Definition of Done

- [ ] `should_emit_rate_limit_audit` helper extracted in `handlers.rs`, keyed on
      `&DashMap`, label, `now`, interval; returns `bool`; inserts on the stale/absent arm.
- [ ] Inline `match` at `handlers.rs:245-255` replaced by a call to the helper; observable
      throttle behavior unchanged.
- [ ] Guard-drop discipline (`.get().map(|r| *r.value())`) documented in a doc-comment on
      the helper, including the accepted-race note and the do-NOT-`.entry()` warning.
- [ ] `#[cfg(test)] mod tests` added to `handlers.rs` with the 3 tests (concurrency
      no-deadlock, throttle semantics, stale-entry revisit).
- [ ] `cargo test -p mika-agent --lib`, `cargo clippy -p mika-agent --all-targets`,
      `cargo fmt --check`, `cargo build` all clean.
- [ ] mika#1719 left OPEN; PR body notes the narrowed-scope handoff to invariant 4 and the
      separate clippy-enforcement companion PR.

## Acceptance criteria

Derived from the ticket's § Verification and § Fix shape (the issue has no
`## Acceptance criteria` section):

1. **No self-deadlock under the trigger condition.** With the map pre-seeded with a stale
   timestamp for an agent label, N ≥ 16 concurrent invocations of the throttle decision for
   that same label all complete within a bounded timeout (no futex hang). Enforced by the
   committed concurrency test.
2. **Fix uses guard-drop, not guard-hold.** The rewritten site extracts the `Copy`
   `Instant` via `.get(...).map(|r| *r.value())` before the match; no DashMap `Ref` is held
   across the `.insert()` arm. No `.entry()` API and no guard-holding scrutinee shape.
3. **Throttle semantics preserved.** First trip per label emits (`true`); a second trip
   within `RATE_LIMIT_TRIP_AUDIT_INTERVAL` is suppressed (`false`); a trip after the
   interval elapses emits again (`true`).
4. **Accepted race left intact.** The get→insert TOCTOU seam (worst case: one duplicate
   `rate_limit_trip` audit row) is documented and NOT closed. No reviewer restructuring back
   into a guard-holding or `.entry()` shape.
5. **Scope contained.** Single-file change to `handlers.rs`; mika#1719 stays open (invariant
   4 unaddressed here); clippy structural enforcement deferred to a separate PR.
6. **Clean gates.** `cargo build`, `cargo test -p mika-agent --lib`,
   `cargo clippy -p mika-agent --all-targets`, and `cargo fmt --check` all pass.
