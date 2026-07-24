# Plan — fix(gateway): route `pull_request.ready_for_review` to mika-qa

**Ticket:** mika#1822
**Type:** fix (bug — loop substrate, p1-important)
**Component:** mika-gateway
**Branch:** `fix/1822/gateway-route-pull-request-ready-for`

## Problem

`mika-gateway`'s `route_event()` does not map the `pull_request.ready_for_review`
action to any agent. When a draft PR is un-drafted, GitHub fires
`pull_request.ready_for_review`; the gateway matches no arm, returns `None`
("not routable, dropping"), and replies 200 OK. As a result the draft→ready
transition never triggers an autonomous `mika-qa` review.

The `check_suite.completed(success)` → mika-qa fan-out (mika#1711) cannot rescue
this case: `check_suite` fires when checks finish (typically while the PR is still
a draft), and no fresh `check_suite` event is emitted when a PR is un-drafted.

**Founding incident (2026-07-23):** PR mika#1821 was un-drafted at
`14:14:40Z` with all 12 checks already green; zero reviews were ever produced,
and the PR sat unmerged, blocking downstream re-kicks. This is a systematic
throughput blocker because dispatch-lib's wip-rescue path routinely promotes
draft PRs to ready.

## Root cause

`crates/mika-gateway/src/github.rs:321` — the `pull_request` routing arm lists
`opened | synchronize | review_requested` but omits `ready_for_review`:

```rust
("pull_request", Some("opened" | "synchronize" | "review_requested")) => Some("mika-qa"),
```

The exact pitfall is already documented for `.github/workflows/*.yml` triggers in
`docs/solutions/best-practices/gha-pr-workflow-event-pitfalls-2026-04-29.md`
(lines 38–53), but the guidance was never ported to the gateway routing table.
Confirmed by grep: `ready_for_review` has zero occurrences in any Rust source
under `crates/`, and the unit tests at `github.rs` cover `opened`, `synchronize`,
`closed`, `review_requested` — never `ready_for_review`.

## Requirements

1. `route_event("pull_request", Some("ready_for_review"), None)` returns
   `Some("mika-qa")`.
2. `ready_for_review` is a **primary-only** route — no secondary fan-out
   (`secondary_targets` returns empty for it).
3. Unit tests assert both of the above in `mod tests`.
4. `crates/mika-gateway/CLAUDE.md` documents the new route alongside
   `opened/synchronize/review_requested`.

## Non-goals (out of scope)

- Extending the fix to `pull_request.reopened` (separate consideration; file a
  follow-up if needed).
- Backfilling qa-review for PRs already stuck ready-but-unreviewed (operator can
  re-request the QA reviewer via `gh api .../requested_reviewers` to hit the
  existing `review_requested` path).
- A poller/reconciler for PRs missing review (separate ticket — this fix is
  preventive, not reconciliative).

## Design notes

- The change is additive to the existing `Some("opened" | "synchronize" |
  "review_requested")` or-pattern — append `| "ready_for_review"`.
- **Reviewer-filter interaction (mika#1655):** the `is_suppressed_review_request`
  guard (`github.rs:305`) is scoped to `action == Some("review_requested")` only.
  Adding `ready_for_review` to the route does **not** subject it to that guard —
  a draft→ready toggle carries no `requested_reviewer` and must not be
  suppressed. This is the desired behavior: `ready_for_review` should route to
  mika-qa unconditionally, exactly like `opened`/`synchronize`. No change to the
  suppression guard is required, and a test should confirm the route is not
  accidentally gated.
- `ready_for_review` deliberately does **not** appear in `secondary_targets`
  (which currently only fans `check_suite.completed(success)` out to mika-qa).
  Its primary target *is* mika-qa; a secondary entry would double-dispatch.

## Implementation steps

1. **`crates/mika-gateway/src/github.rs:321`** — add `ready_for_review` to the
   `pull_request` → mika-qa or-pattern:
   ```rust
   ("pull_request", Some("opened" | "synchronize" | "review_requested" | "ready_for_review")) => Some("mika-qa"),
   ```

2. **`crates/mika-gateway/src/github.rs` `mod tests`** (near the existing
   `test_route_event_pr_*` tests ~line 1701) — add two tests:
   ```rust
   #[test]
   fn ready_for_review_routes_to_mika_qa() {
       assert_eq!(
           route_event("pull_request", Some("ready_for_review"), None),
           Some("mika-qa")
       );
   }

   #[test]
   fn ready_for_review_no_fan_out() {
       assert_eq!(
           secondary_targets("pull_request", Some("ready_for_review"), None),
           &[] as &[&str]
       );
   }
   ```
   (Optional defensive test: confirm `is_suppressed_review_request(Some("ready_for_review"), None)` returns `false` — the guard is `review_requested`-scoped, so `ready_for_review` is never suppressed.)

3. **`crates/mika-gateway/CLAUDE.md`** § "GitHub Webhook Integration" — update the
   route bullet from:
   > `pull_request.opened/synchronize/review_requested` -> mika-qa

   to include `ready_for_review`:
   > `pull_request.opened/synchronize/review_requested/ready_for_review` -> mika-qa

## Verification contract

- `cargo test -p mika-gateway` — the two new tests pass; existing routing tests
  unchanged (regression-free).
- `cargo clippy -p mika-gateway` — clean.
- `cargo fmt --check` — clean.
- Manual grep confirms `ready_for_review` now present in the Rust routing arm and
  the CLAUDE.md route list.

## Definition of Done

- `route_event` maps `pull_request.ready_for_review` → `Some("mika-qa")`.
- Two unit tests assert the route and the primary-only (no-fan-out) property, and
  pass under `cargo test -p mika-gateway`.
- `crates/mika-gateway/CLAUDE.md` documents the route.
- `cargo clippy` / `cargo fmt` clean; existing tests still pass.
- PR body notes the post-deploy manual verification (AC4) as a deploy-time
  follow-up (not gated in CI — requires a live draft→ready toggle).

## Acceptance criteria

- **AC1** — `route_event("pull_request", Some("ready_for_review"), None)` returns
  `Some("mika-qa")`.
- **AC2** — Unit test in `mod tests` asserts AC1 and that `secondary_targets`
  returns empty for `ready_for_review` (primary-only, no fan-out).
- **AC3** — `crates/mika-gateway/CLAUDE.md` documents the `ready_for_review` route
  (grouped with `opened/synchronize/review_requested`).
- **AC4** — Post-deploy: manual verification by toggling a test PR draft→ready
  produces a mika-qa session (grep gateway log for delivery of
  `pull_request.ready_for_review` to mika-qa; grep mika-qa log for the resulting
  review). Deploy-time follow-up, not a CI gate.

## References

- PR senara-solutions/mika#1821 (founding incident, 2026-07-23)
- `docs/solutions/best-practices/gha-pr-workflow-event-pitfalls-2026-04-29.md`
  (documents the exact pitfall for workflow triggers)
- mika#1711 (`check_suite.completed(success)` → mika-qa fan-out — related but
  insufficient: doesn't refire on draft→ready)
- mika#1655 (`review_requested` reviewer-filter guard — scoped to
  `review_requested`, does not affect `ready_for_review`)
