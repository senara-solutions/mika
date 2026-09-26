# Review-Anchor Attestation: Clean Plan With Numbered Questions (READY)

This fixture measures the anti-vacuity direction of mika#2037: a plan with no blocking
concern must still produce an *attested* READY, not a short acknowledgement. A model that
cannot quote the brief it just reviewed will trip the engine's review-anchor guard on every
clean plan, burning a corrective re-prompt per grooming round.

## Plan Under Review: docs/plans/2026-08-27-001-manager-token-renewal-plan.md

### Summary

The milestone-manager cycle token is resolved once at spawn and forwarded to `gh` forever.
A GitHub App installation token has roughly a one-hour lifetime, so the manager cycles 401
until the process restarts. Re-resolve the token before every cycle instead of freezing it.

### Design

- The cycle token is re-resolved before every cycle through a `TokenResolver` trait, never
  frozen at spawn time.
- `SettingsTokenResolver` is the production implementation: PAT first per ADR-008, GitHub App
  installation token as the fallback.
- A change in the resolved value emits `manager_token_refreshed` at INFO, carrying presence
  booleans only — never token material.
- `AuthFailureTracker` counts the duration of an unbroken 401 run, not a count of cycles:
  `poll_interval` is operator-configurable, so N cycles has no stable temporal meaning.
- Past a 30-minute threshold the tracker emits `manager_auth_persistent_failure` at ERROR and
  escalates, re-announcing at most hourly.

### Error Handling

- A failed re-resolution keeps the previous token rather than overwriting it with `None`.
  `reader.rs` only sets `GH_TOKEN` when the value is `Some`, so overwriting would silently
  drop the cycle onto the host's ambient credentials.
- The refresh is bounded by a 15-second timeout: it sits outside the `select!` on the
  cancellation token, and `GitHubApp` holds its cache write-lock across an un-timed HTTP call.
- Any successful cycle clears the failure window. Non-401 failures neither advance nor clear
  it — a network blip is not proof of recovery, nor of auth failure.

### Test Plan

- Unit: resolver returns a changed value, unchanged value, and an error; the tracker advances,
  clears, and re-announces on schedule.
- Integration: a cycle running against a resolver whose token rotates mid-run.

### Scope Boundaries

- `AuthClass::Forbidden` (403) is out of scope. A 403 is a permission-shape failure, not a
  credential-expiry one, and the alarm targets the latter.

### Acceptance criteria

- [ ] The token is re-resolved before every cycle, never frozen at spawn.
- [ ] A persistent 401 run past 30 minutes escalates, and re-announces at most hourly.
- [ ] A failed re-resolution never downgrades the cycle to ambient credentials.

---

## Questions for the architect

1. Where does the correction belong — in the spawn path, or in the cycle body?
2. Is the trace exhaustive: should every refresh be journaled, or only a changed value?
3. I have not fixed N for the repeated authentication failure threshold. What should it be?
4. Is putting the 403 response class out of scope safe for this milestone?
