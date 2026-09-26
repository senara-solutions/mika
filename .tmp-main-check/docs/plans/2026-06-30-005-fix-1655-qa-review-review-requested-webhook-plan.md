---
issue: 1655
type: fix
date: 2026-06-30
---

# Plan — fix(qa-review): auto-fire on `pull_request.review_requested` webhook (mika#1655)

## Problem

`qa-review` bundled skill auto-fires on `pull_request.opened` and `pull_request.synchronize` webhook actions only. The `pull_request.review_requested` action with `requested_reviewer.login: mika-platform-qa` is NOT routed to qa-review. Operator-driven re-requests via `gh api .../requested_reviewers` sit in REVIEW_REQUIRED indefinitely until either a fresh commit (synchronize) or manual `mika ask --agent mika-qa` invocation.

Hard evidence n=2 from 2026-06-29 (PR #1637 + PR #1644 both delayed ~25min each waiting on manual fallback).

## Architectural lineage

- `skills/bundled/self-dev-webhook-qa/` — existing handler for `pull_request.opened` + `synchronize` (closest analog)
- `crates/mika-gateway/src/github/webhook.rs` (or equivalent) — webhook router that dispatches by event+action
- mika#841 — `ready` label autonomous-consent design + qa-review skill family

## Prerequisite (implementer first task — architect F2)

**Verify mika-gateway forwards `pull_request.review_requested` events to mika-dev** before committing the Layer 1 fix. GitHub sends `review_requested` by default if the webhook subscribes to `pull_request` events, but mika-gateway may filter actions internally. Grep:

```bash
grep -rn 'review_requested\|"action"' crates/mika-gateway/src/github/
```

If gateway forwards the action → Layer 1 only. If gateway filters out `review_requested` → Layer 1 + Layer 2 (gateway forward fix).

## Fix shape (architect to ratify Layer-1-or-Layer-2 placement)

The trigger has to surface SOMEWHERE between webhook arrival and qa-review skill invocation. Two candidate placements:

### Layer 1 — extend `self-dev-webhook-qa/` skill triggers

Modify the existing skill's `[triggers]` keyword list to include `review_requested:mika-platform-qa` (or similar). The gateway already routes `pull_request` events to mika-dev; the skill itself filters by action. Lowest-surface change.

### Layer 2 — gateway-side action filter expansion

Modify `crates/mika-gateway/src/github/webhook.rs`'s action dispatch table to add `review_requested` → mika-dev message, with the action+reviewer carried in the message envelope. Skill picks it up via keyword match.

### Proposed default: **Layer 1 with reviewer-login filter (architect F1, BLOCKING)**

The gateway dispatches `pull_request` events to mika-dev regardless of action (verified per F2 prerequisite). The skill's `[triggers]` keywords + system_prompt logic discriminate. Adding `review_requested` as a trigger keyword is the minimal change — **but must include the reviewer-login filter** to avoid firing on every human-reviewer request:

```
Activation condition: action == "review_requested" AND requested_reviewer.login == "mika-platform-qa"
```

Human reviewers (`vincent`, etc.) MUST NOT trigger qa-review.

## Implementation outline (Layer 1 default)

1. **Identify the skill trigger shape.** Read `skills/bundled/self-dev-webhook-qa/skill.toml` to see current `[triggers].keywords`. Add the new keyword that the gateway message body contains for `review_requested` events. Verify what the gateway puts in the message body for this action — likely something like `[review_requested] PR #N: mika-platform-qa requested for review`.

2. **Update the skill's `system_prompt.md` to recognize the new event shape.** Add a section/clause that says "On `review_requested` action with reviewer = mika-platform-qa, treat as a fresh qa-review request and proceed with the normal qa-review flow."

3. **Verify gateway forwards the event.** If the gateway today filters out `review_requested` actions (i.e., doesn't forward to mika-dev), this becomes a Layer-2 fix. Implementer first task: confirm gateway behavior via `grep -rn review_requested crates/mika-gateway/`.

4. **Integration test (AC2):** webhook fixture for `pull_request.review_requested` action with `requested_reviewer.login: mika-platform-qa`. Assert that the qa-review skill is invoked + produces expected pre-digest in the message body.

5. **Regression test (AC3):** existing `opened` + `synchronize` paths still fire qa-review normally. No regression on the non-mika-platform-qa reviewer paths (other reviewers don't get qa-review triggered).

## Acceptance criteria

- **AC1** — Webhook routing: `POST /github/webhook` with `X-GitHub-Event: pull_request, action: review_requested, requested_reviewer.login: mika-platform-qa` invokes the qa-review skill against that PR.
- **AC2** — Integration test: webhook fixture exercising the new path produces the expected qa-review pre-digest. Fixture MUST also assert the **negative case** — a `review_requested` event with `requested_reviewer.login != mika-platform-qa` (e.g., a human reviewer login) does NOT invoke qa-review (architect F1 + F3 conjunction-sharpening).
- **AC3** — No regression on existing `synchronize`/`opened` qa-review fires.

## Out of scope

- Reviewers other than `mika-platform-qa` (different agents may want different handlers; explicit in ticket body).
- Re-requesting via UI checkbox — works via the same webhook event, covered by AC1.

## Files involved (architect first-pass to confirm)

Layer 1 default:
- `skills/bundled/self-dev-webhook-qa/skill.toml` — add trigger keyword
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — recognize new event shape

If gateway also needs changes (architect determines):
- `crates/mika-gateway/src/github/webhook.rs` — action dispatch filter

Tests:
- Webhook integration test fixture (location TBD per architect — likely `crates/mika-gateway/tests/` or skill integration test in mika-agent)

## Verification

- Implementer first task: confirm gateway already forwards `review_requested` events to mika-dev. If yes → Layer 1 only. If no → Layer 1 + Layer 2.
- Regression: existing `opened`+`synchronize` qa-review tests stay green.

## Implementation note (discovered during /ce:work)

The F2 prerequisite investigation found that the `review_requested` path was **already wired end-to-end on `main`**, contrary to the plan's premise:

1. **Gateway routing** — `crates/mika-gateway/src/github.rs` `route_event()` already maps `("pull_request", "review_requested") → "mika-qa"` (added 2026-04-20, #506/#707).
2. **Event formatting** — `format_event_text()` already emits `Requested reviewer: @<login>` and the `GitHubWebhookEvent` struct already parses `requested_reviewer`.
3. **Skill prompt** — `skills/bundled/qa-review/system_prompt.md:5` already lists `pull_request.review_requested` as a trigger; the skill is `always_on`.
4. **Engine passthrough** — `handlers.rs::handle_message` intercepts only `pull_request_review.submitted` / `check_suite.*`; a `pull_request.review_requested` message passes straight through to the always-on qa-review turn.

So "add a webhook handler" (ticket framing) and "extend skill triggers" (Layer 1 framing) were both already satisfied. The genuine, testable gap matching architect **F1 (BLOCKING — reviewer-login filter)** is **reviewer discrimination**: `route_event` routed *every* `review_requested` to mika-qa regardless of reviewer, so requesting any human reviewer would wrongly spin up a full qa-review (failing AC2's negative case).

**Chosen placement — gateway post-route guard, not skill prompt.** F1 sketched the filter living in the skill's prompt/keywords, but an LLM-prompt filter cannot be deterministically tested (AC2 demands "integration test exercising the path"), and reviewer-login routing is a structural concern, not a judgment one. The filter is implemented as `is_suppressed_review_request(action, requested_reviewer)` — a pure predicate consumed by a handler guard placed alongside the existing skill-denylist (#845) and synchronize-no-diff (#886) guards. `QA_REVIEWER_LOGIN = "mika-platform-qa"`. Fail-closed: missing/unresolvable reviewer (e.g. team requests carrying `requested_team`) is suppressed.

**AC mapping:** AC1 → `test_review_request_for_qa_bot_is_dispatched` (+ existing `test_format_event_text_pr_review_requested_with_reviewer`); AC2 negative → `test_review_request_for_human_reviewer_is_suppressed` + `test_review_request_with_no_reviewer_is_suppressed`; AC3 → `test_non_review_request_actions_are_never_suppressed` + existing `test_route_event_pr_*` routing tests stay green.

## References

- mika#841 — `ready` label autonomous-consent design (parent design context)
- mika#1645 — qa-review cross-artifact equivalence-claim grounding (sibling)
- Today's session: orchestrator-CC `bba3bcac` at 15:00–15:32Z (incident timestamps)
