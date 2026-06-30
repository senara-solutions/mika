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

## References

- mika#841 — `ready` label autonomous-consent design (parent design context)
- mika#1645 — qa-review cross-artifact equivalence-claim grounding (sibling)
- Today's session: orchestrator-CC `bba3bcac` at 15:00–15:32Z (incident timestamps)
