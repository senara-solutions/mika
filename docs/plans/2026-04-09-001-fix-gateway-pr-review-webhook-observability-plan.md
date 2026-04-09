---
title: "fix: gateway PR review webhook observability and verdict contract"
type: fix
status: active
date: 2026-04-09
issue: 487
---

# fix: gateway PR review webhook observability and verdict contract

Issue #487 — mika-qa posted a `state=COMMENTED` PR review on mika-platform#19 with a `VERDICT: hold[review]` token. mika-dev never received the `pull_request_review.submitted` webhook, silently breaking the "QA verdict → mika-dev retry" handoff. The primary hypothesis (H1) is the GitHub App is not subscribed to `Pull request review` events — that's an App-config problem, not code. But the gateway currently cannot distinguish "never arrived" from "arrived but dropped" because unroutable events log at `debug!` and the very first touch of a webhook (pre-dedup/pre-routing) is not logged at all. This plan fixes the observability gap in code, and documents the verdict contract so future code never regresses to gating on `state`.

## Scope

**In scope (code):**
- Add a `debug!` log on every inbound GitHub webhook, emitted immediately after signature validation and before dedup/parse/routing. Fields: `event_type`, `delivery_id`.
- Promote the "event not routable, dropping" log at `crates/mika-gateway/src/github.rs:428-434` from `debug!` to `warn!` so silent drops are visible in prod logs.
- Document the verdict contract in `docs/skills.md` (self-dev/qa-bot section): verdicts are `state=COMMENTED` with a `VERDICT:` token in the body; state is NOT authoritative; any webhook filter or verdict parser that gates on `state` instead of body content is a bug.
- Unit test coverage for the log promotion (assert `route_event` returns `None` for the unroutable cases we already test; add a compile-time regression note) — no behavior change tests needed beyond existing ones.

**Out of scope (operational, documented in PR body for human operator):**
- Criterion A: verifying/adding the App-level `Pull request review` event subscription at https://github.com/organizations/senara-solutions/settings/apps/<app-slug>. Cannot be done from code.
- Criterion C: posting a fresh diagnostic review on PR #19 to confirm end-to-end delivery. Manual smoke test after merge + deploy.
- Changing qa-bot to post `CHANGES_REQUESTED` (explicitly rejected in issue — QA is advisory).
- self-dev prompt fallbacks for missing webhooks.

## Acceptance criteria

- [x] `handle_github_webhook()` emits a `debug!` with `event_type` and `delivery_id` as the first log after signature validation (before ping short-circuit, before dedup, before body parse).
- [x] Unroutable events log at `warn!` (not `debug!`) with `event_type`, `action`, and `delivery_id`, so a silently-dropped `pull_request_review` (or any event) is discoverable via log search.
- [x] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test -p mika-gateway` all pass.
- [x] `docs/skills.md` contains an explicit note on the qa-bot verdict contract, including the BAD pattern ("gate on `state` field") and the GOOD pattern ("parse `VERDICT:` token from body"). The note lives alongside the existing self-dev / skills lifecycle documentation.
- [ ] PR body documents the manual verification steps (criteria A and C) for the human operator to run post-merge.

## Implementation notes

### 1. `crates/mika-gateway/src/github.rs` — inbound trace log

After the `info_span!("github_webhook", ...)` at line 378-383 and before the ping handler at line 386, add:

```rust
debug!(
    event_type,
    delivery_id = %delivery_id,
    "GitHub webhook received (pre-dedup, pre-routing)"
);
```

The `action` field is not available here because body parsing happens at step 7 (line 407). The existing `info!` at line 437 already logs `event_type + action + target_agent + delivery_id` on the success path, so an unrouted event will show the pre-routing `debug!` followed by the new `warn!` (below), while a delivered event shows `debug!` + `info!`. This lets log search distinguish:

- **Never arrived:** no `debug!` at all for that delivery_id → App not subscribed or network/proxy issue.
- **Arrived but dropped (no route):** `debug!` + `warn!` → routing table bug or filter bug.
- **Arrived and delivered:** `debug!` + `info!` → working as intended.

### 2. `crates/mika-gateway/src/github.rs` — route-miss warning

Change lines 428-434:

```rust
// BEFORE
debug!(
    event_type,
    action = ?event.action,
    "GitHub webhook event not routable, dropping"
);

// AFTER
warn!(
    event_type,
    action = ?event.action,
    delivery_id = %delivery_id,
    "GitHub webhook event not routable, dropping (check route_event table and App subscriptions)"
);
```

Rationale: a dropped event is a correctness signal, not debug noise. The issue's root-cause analysis is exactly the situation this warning is designed to flag. The existing `debug!` was explicitly called out by the issue as "currently it silently drops" — that's the bug.

### 3. `docs/skills.md` — verdict contract note

Append (or find the self-dev section and insert) a block like:

```markdown
## QA verdict contract

mika-qa-bot posts PR verdicts as GitHub reviews with:

- `state: COMMENTED` (NOT `APPROVED` or `CHANGES_REQUESTED`)
- Body containing a `VERDICT: <class>[<detail>]` token (e.g. `VERDICT: approve`, `VERDICT: hold[review]`, `VERDICT: reject`)

**The `state` field is NOT authoritative. The `VERDICT:` token in the body is.**

QA is advisory — it never blocks GitHub's native merge button. Using `CHANGES_REQUESTED` would conflate advisory verdicts with GitHub's review-required gate and is explicitly rejected.

### BAD — gating on state

```rust
if review.state == "CHANGES_REQUESTED" { retry() } // ❌ never fires for qa-bot
```

### GOOD — parsing the token

```rust
if body.contains("VERDICT: hold") || body.contains("VERDICT: reject") { retry() } // ✅
```

Any webhook filter, routing rule, or parser that gates on `state` instead of body content is a bug. See issue #487 for the incident that motivated this contract.
```

Check whether `docs/skills.md` already has a self-dev section; if so, insert under it. Otherwise append as a new top-level section. Same content sync rule applies: `docs/` is the single source of truth and `crates/mika-agent/build.rs` embeds it via `include_str!` from `OUT_DIR`. No sync script run is needed unless publishing (per CLAUDE.md doc-sync convention) — but the CI `docs-sync` job enforces `scripts/sync-agent-docs.sh` if the crate-local copy drifts. Run the sync script if `crates/mika-agent/docs/skills.md` exists and is the stale fallback copy.

### 4. Tests

The existing `route_event` tests in `github.rs` already cover the `pull_request_review/submitted` positive case. No new routing tests needed — we are not changing routing behavior.

Add (if ergonomic) a doc-test or assertion-free comment block documenting the three log scenarios (never-arrived, arrived-but-dropped, arrived-and-delivered). This is primarily a prose change.

Run:
- `cargo fmt --all`
- `cargo clippy -p mika-gateway --all-targets -- -D warnings`
- `cargo test -p mika-gateway`

## Manual verification steps (for PR body)

These cannot be automated from the code side and belong in the PR body as a checklist for the human operator:

1. **Criterion A — App subscription:** Open https://github.com/organizations/senara-solutions/settings/apps/<app-slug> → Webhook → Subscribe to events. Confirm `Pull request review` is checked. If missing, check it and save.
2. **Criterion C — end-to-end smoke test:** After deploy, run:
   ```
   gh pr review <fresh-PR> --repo senara-solutions/mika-platform --comment -b "VERDICT: approve — diagnostic test for #487"
   ```
   Then grep mika-gateway logs for the delivery_id. Expect: new `debug!` pre-routing log → `info!` routing to `mika-dev` → agent container 202. Confirm mika-dev's session shows a turn containing the review body.
3. **Verify warn-on-drop:** Trigger any unroutable event (e.g. a `star.created` webhook by starring/unstarring a repo the App is installed on — if subscribed) and confirm it produces the new `warn!` line in gateway logs.

## Risks

- **Log volume:** Adding a `debug!` per webhook is fine (debug-level, filtered in prod by default). The promoted `warn!` for dropped events may be noisy if the App is subscribed to many event types we don't route (e.g. `star`, `watch`, `fork`). Mitigation: the issue explicitly asks for this ("no event should vanish without a log entry"). If noise becomes a problem in practice, trim the App's subscription list — that's the correct fix, not suppressing the log.
- **Doc sync drift:** If `crates/mika-agent/docs/skills.md` is the stale fallback copy, CI `docs-sync` job will fail. Run `scripts/sync-agent-docs.sh` before committing if needed.

## Sources

- Issue: senara-solutions/mika#487
- Code: `crates/mika-gateway/src/github.rs:378-434` (handler), `:141-157` (route_event)
- Incident trail: mika-platform#18 → PR #19 → qa-bot review at 2026-04-08T11:58:32Z
- CLAUDE.md: gateway section (request logging via TraceLayer, GitHub App auth)
- Related memory: `feedback_qa_advisory_ci_gate_on_dev` — QA is advisory, mika-dev enforces merge gate
