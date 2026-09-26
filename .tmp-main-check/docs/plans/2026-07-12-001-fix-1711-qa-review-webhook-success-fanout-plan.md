---
issue: 1711
type: fix
scope: gateway, skills, mika-qa identity
title: fan out check_suite.completed(success) to mika-qa (Option A)
---

# Plan — mika#1711: qa-review-webhook-success fan-out

## Problem

mika-qa's autonomous qa-review dispatch on CI-complete PR events silently no-ops. Confirmed 14-hour dead window on 2026-07-01 (last activity 02:53Z until manual mika-spirit restart at 17:15Z). Multiple loop PRs (#1706, #1709, #1700) landed READY+GREEN with zero qa attempts and required admin-merge.

Root cause named in the AC1 diagnosis comment on mika#1711 (posted 2026-07-12 by orchestrator-CC before this PR):

- `crates/mika-gateway/src/github.rs:317-329` (`route_event`) — `check_suite.completed(success)` routes only to `mika-dev`.
- mika-qa's `qa-review` skill has no webhook trigger; it fires on keyword-match against user messages.
- mika-dev's `self-dev-webhook-ci` skill (`description = "Webhook handler for CI check_suite failures"`) handles FAILURES only. There is no `success` counterpart routed to mika-qa.
- Consequence: mika-qa only reviews when explicitly reviewer-requested (`is_suppressed_review_request` filter). The autonomous-loop PR flow never emits `review_requested` for the QA bot, so mika-qa gets zero dispatch signals in a green-CI window.

## Verification result

Verified the diagnosis by reading `route_event`, `qa-review/skill.toml`, `self-dev-webhook-ci/skill.toml`, and the webhook dispatch flow at `github.rs:740-975`. All three surfaces confirm the trigger gap. See the ticket comment for full evidence.

## Scope

### Option A — chosen

New `qa-review-webhook-success` skill on mika-qa + gateway fan-out of `check_suite.completed(success)` to both mika-dev (primary; existing merge-readiness path) and mika-qa (secondary; new autonomous review path). Rejected alternatives:

- **Option B (gateway fan-out only)**: modify `route_event` to return `Vec<&'static str>`. Rejected because it forces every caller through vector allocation on the hot path and breaks the additive contract with existing tests.
- **Option C (cross-agent dispatch from mika-dev)**: have `self-dev-webhook-ci` notify mika-qa via `run_a2a` or an internal channel. Rejected because it tangles two agents' state and breaks the wrapper-doctrine (mika-dev's role is merge readiness, not qa dispatch coordination).

### In scope for v1 (this PR)

- New helper `secondary_targets(event_type, action, check_conclusion) -> &'static [&'static str]` alongside `route_event`. Returns `["mika-qa"]` for `check_suite.completed(success)`, empty otherwise. Additive; existing `route_event` contract unchanged.
- Fan-out dispatch in webhook handler after primary spawn. Each secondary target acquires its own semaphore permit + delivery slot. Log-and-skip on slot exhaustion (secondary must not fail the primary; DLQ stays authoritative).
- New bundled skill `skills/bundled/qa-review-webhook-success/`:
  - `skill.toml` — `always_on = false`, `dependencies = ["qa-review"]`, `[triggers].keywords = ["check_suite", "check suite", "ci success", "check_suite success"]`.
  - `system_prompt.md` — correlate to PR (via `run_gh pr list --head <branch>` for open PRs) → skip if not-our-scope (author, draft, repo checks) → skip if already-reviewed-at-SHA (`review.commit_id == pr.headRefOid`) → dispatch qa-review. Never dispatches claude-pilot; never merges.
- Add `qa-review-webhook-success` to `MIKA_QA_IDENTITY.allowlist` in `well_known_agents.rs`. Bumps counter test assertion 17 → 18.
- 8 new unit tests for `secondary_targets` including a structural invariant test (`secondary ∩ primary = ∅` for all routable events).

### Deferred to follow-ups

- **AC3** (audit_event on silent-no-op) — mika#1774 (filed).
- **AC4** (integration test EvalHarness + MockLlmProvider webhook path) — filed inline in PR body; separate scope due to test-harness scaffolding.
- **AC5** (dashboard mika-qa qa-fire rate signal) — mika#1775 (filed).

## Acceptance criteria

- [ ] **AC1** — Diagnosis of the trigger gap named with file-level location. Posted as ticket comment 2026-07-12 (before this PR).
- [ ] **AC2** — When a PR reaches all-checks-green (`check_suite.completed(success)`) on a repo in the routable allowlist, mika-qa's `qa-review-webhook-success` skill receives the webhook within 60s and dispatches `qa-review`. Verified structurally by the new `secondary_targets` fan-out logic + the `qa-review-webhook-success` skill's dependency + allowlist inclusion. Audit_event kind=`qa_dispatch_fired` is emitted by the existing qa-review dispatch path once the fan-out delivers.
- [ ] **AC3** — Audit_event on silent-no-op: **tracked in mika#1774**, not in this PR.
- [ ] **AC4** — Integration test: **tracked as follow-up**, not in this PR.
- [ ] **AC5** — Dashboard signal: **tracked in mika#1775**, not in this PR.
- [ ] **AC6 (structural invariant)** — `secondary_targets` must never intersect with `route_event`'s primary for any event tuple. Enforced by unit test `test_secondary_targets_no_intersection_with_primary`.
- [ ] **AC7 (fan-out failure isolation)** — A secondary target that can't acquire a semaphore permit or delivery slot must log-and-skip; the primary dispatch continues unaffected. Enforced by the code shape (secondary acquires happen after primary is fully spawned).

## Definition of Done

- All acceptance criteria satisfied where marked in-scope for this PR (AC1, AC2, AC6, AC7). Out-of-scope ACs tracked in follow-up tickets and referenced in the PR body via `Tracked in:` lines.
- Unit tests: 8/8 `secondary_targets` tests pass; 63/63 `well_known_agents` tests pass (allowlist count assertion updated).
- Structural gate: `make verify-bundled-skills` — 5/5 checks pass on the new skill.
- Build + lint: `cargo build -p mika-gateway -p mika-agent`, `cargo fmt`, `cargo clippy` — all clean.

## Out of scope

- Full end-to-end integration test against a live mika-spirit / mika-gateway pair. Uses mocked webhook payload + EvalHarness on the agent side — non-trivial harness, split to AC4 follow-up.
- Route change for `check_suite.completed(failure|timed_out)` — those stay mika-dev-only (self-dev-webhook-ci handles failures).
- Dashboard UI surface for qa-dispatch fire-rate — mika#1775.
- Alerting mechanism for zero-fires-for-N-hours — mika#1775 subscope.

## Files involved

- `crates/mika-gateway/src/github.rs` — new `secondary_targets` helper + fan-out dispatch loop + 8 new tests
- `crates/mika-agent/src/well_known_agents.rs` — `MIKA_QA_IDENTITY` allowlist + counter test assertion
- `skills/bundled/qa-review-webhook-success/skill.toml` (new)
- `skills/bundled/qa-review-webhook-success/system_prompt.md` (new)

## Verification

```bash
cargo test -p mika-gateway github::tests::test_secondary   # 8/8
cargo test -p mika-agent --lib well_known_agents           # 63/63
make verify-bundled-skills                                 # 5/5
cargo build -p mika-gateway -p mika-agent                  # clean
cargo fmt --package mika-gateway                           # clean
cargo clippy                                                # clean
```

## References

- mika#1711 — parent ticket (5 ACs)
- AC1 diagnosis comment on mika#1711 — the pre-implementation walk of `route_event`, `qa-review/skill.toml`, `self-dev-webhook-ci/skill.toml`
- `crates/mika-gateway/src/github.rs:317-329` — `route_event`
- `crates/mika-gateway/CLAUDE.md` § Review-requested reviewer filter (mika#1655)
- mika#1655 — reviewer filter precedent (`is_suppressed_review_request`)
- mika#1774 — AC3 follow-up (audit_event on silent no-op)
- mika#1775 — AC5 follow-up (dashboard fire-rate signal)
- Vincent-directed 2026-07-12 work order (via samidarko-claude): loop-health priority 1, drain-clog #2 restoration
