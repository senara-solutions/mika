---
module: mika-agent/milestone_manager
tags: [auth, github-app, token-refresh, silent-failure, observability, cadence-loop]
problem_type: logic_error
category: logic-errors
created: 2026-08-29
ticket: mika#2013
---

# A renewable credential resolved once at spawn is a frozen credential

## Problem

Mika Manager cycled `manager_cycle_error auth_class=401` sixteen times in a
single night and could not read its milestone. `verify_gh_auth` had passed at
boot. Nothing in the process had changed.

The token was resolved **once**, at spawn:

```
spawn.rs:157    settings.resolve_github_token(github_app).await   <- ONCE
       |        stored in ManagerConfig.github_token: Option<String>
cadence.rs:405  Reader::new(cfg.github_token.clone())             <- EVERY cycle, same value
       |
reader.rs       cmd.env("GH_TOKEN", t)                            <- EVERY `gh` subprocess
```

`Settings::resolve_github_token` returns a `String`. When no PAT is configured
it falls back to a GitHub App installation token — TTL ~1h. After an hour the
manager 401s until someone restarts the daemon.

**The renewal already existed.** `GitHubApp::installation_token()` has an
in-memory cache with a 5-minute expiry buffer and re-exchanges the JWT when the
token nears expiry. The defect was never a missing refresh mechanism. It was
that we asked for the token once and then held the answer forever.

## The diagnosis trap this ticket walked into

The issue body named the wrong cause with real evidence behind it. Five
per-agent `github_app_token.json` caches were measured stale, so the body
concluded "nothing refreshes them." Reading the code inverted it: every one of
the three sites that touches those files goes through
`installation_token_with_file_cache` (`github_app.rs:254`), which validates
`expires_at`, re-exchanges when stale, and rewrites the file. A `grep` outside
`github_app.rs` returns no raw reader.

The five timestamps sat within **3 seconds** of each other. That is one CLI pass
writing them and nothing reading them since. They were stale because they were
*unused*, not because refresh was missing — and the failing path never reads
them at all (`resolve_github_token` calls `installation_token()`, not the
`_with_file_cache` variant).

The manual mitigation — propagating the global token into the five per-agent
caches — was therefore inert. What actually revived the manager was the process
restart.

**Lesson.** A stale artifact on disk is evidence of *disuse* as readily as of
*broken renewal*. Before concluding "nothing refreshes X," grep for who *reads*
X. Tight timestamp clustering across supposedly independent caches is the tell:
independent renewal produces scattered times, one writer produces a cluster.

## Second lesson: the defect was already written down

`spawn.rs:144-157` carried this comment, left by mika#1968:

> *A3 P1 note - App token lifetime hazard (deferred per plan 5c). [...]
> `ManagerConfig.github_token` is populated ONCE at spawn time and forwarded
> verbatim to `gh` on every cycle. After 1h the manager cycles silently 401
> until the process restarts [...] Follow-up ticket needed.*

The diagnosis predated the incident by one ticket. A known-hazard comment with
no ticket attached is a bug with a scheduled outage date. When deferring a
hazard, file the follow-up in the same breath — the comment is a note to
yourself; the ticket is the thing that actually gets worked.

## Solution

**A. Re-resolve per cycle.** A `TokenResolver` trait is consulted before every
`run_manager_cycle`; `SettingsTokenResolver` delegates to
`Settings::resolve_github_token`. Cost is nil in the common cases — a PAT
returns immediately, and the App path is served by the memory cache until the
token is genuinely near expiry. A changed value emits `manager_token_refreshed`
(INFO, presence booleans only — never token material), so renewal is observable
rather than assumed. The trait is also the seam that makes this testable without
a live GitHub App.

`run_manager_cycle` / `run_manager_cycle_with` keep their signatures; only
`spawn_manager_cycle_task` gained the resolver argument. Minimal blast radius.

**B. The 401 must cry.** `AuthFailureTracker` measures the **duration** of an
unbroken `AuthClass::Unauthorized` run, not a cycle count — `poll_interval` is
operator-configurable, so "N failed cycles" is fifteen minutes at one cadence
and three hours at another. Past 30 minutes it emits
`manager_auth_persistent_failure` (ERROR) and escalates to
`MIKA_MANAGER_ESCALATION_URL`, re-announcing at most hourly so the alarm does
not become the spam it replaces.

Design rules worth keeping:

- **A successful cycle clears the window; a non-401 failure neither advances nor
  clears it.** A network blip is not proof that auth recovered — resetting on it
  would let a real outage evade the threshold forever by interleaving. Nor is it
  proof of auth failure. Only success proves recovery.
- **The escalation carries a dedicated `AuthAlarmBody`, not the normal
  `DeliveryBody`.** `DeliveryBody` requires a full milestone `Assessment` — and
  this alarm fires precisely because the milestone could not be read. Filling
  one in would put fabricated milestone state on the wire. When a schema demands
  data your failure mode denies you, that is a signal to use a different
  payload, not to invent the data.

## Verification

The load-bearing test was mutation-checked: with `refresh_cycle_token` removed
from the loop, `spawn_loop_re_resolves_token_on_every_cycle` fails with
`got 0 call(s)`; restored, it passes. A test that cannot fail without the fix
is not evidence the fix works.

The alarm's clock is injected (`Instant` passed in), so the 30-minute threshold
and 1-hour cooldown are asserted in milliseconds. Anti-vacuity companions cover
each: `Other` at 31 minutes fires nothing, 29 minutes of 401 fires nothing, and
29 + success + 29 fires nothing.

## What the review caught — the fix's own new failure modes

Three of the four review findings were defects **introduced by the fix**, not
pre-existing. Worth naming, because they are the characteristic hazards of
converting a frozen value into a live one:

1. **A refresh can fail, and a failed refresh must not destroy what worked.**
   `resolve_github_token` returns `None` when the App exchange errors — one
   network blip. Assigning that `None` would drop `GH_TOKEN` from the next `gh`
   call entirely (`reader.rs` only sets it when `Some`), silently running the
   cycle under the host's ambient credentials — a different identity than
   ADR-008 mandates — or none. The frozen-token bug could not do this; the fix
   could. Guard: empty resolution keeps the previous value, at WARN.

2. **"Credential missing" does not look like "credential rejected."** With no
   token, `gh` never reaches the API and prints its own onboarding text — no
   401 anywhere. That classified as `Other`, which the new tracker deliberately
   ignores, so the very blindness the alarm exists to end stayed invisible on
   the shape the fix made reachable. `classify_cycle_error` now recognises
   `gh auth login` and `authentication token not found` as `Unauthorized`,
   matching the enum's own docstring ("token missing/invalid/expired").

   A knock-on: `authentication token not found` contains `not found`, and the
   probe classifier tested that substring *first* — reporting a missing
   credential as a missing milestone. Delegating first and applying the 404
   discrimination only to what the base classifier leaves as `Other` fixes it
   without weakening genuine 404 detection.

3. **A new await inside a loop is a new way to hang.** The per-cycle refresh
   sits outside the `select!` on the cancellation token, and `GitHubApp` holds
   its cache write-lock across an HTTP call built with no timeout — shared with
   the tool registry and credential-helper paths. An unbounded stall would hold
   the manager past graceful shutdown and block every other consumer. Bounded at
   15s.

**Generalisable rule.** Turning a once-resolved value into a per-use resolution
adds three failure modes that did not exist before: the resolution can fail, it
can fail in a way the old code's error taxonomy does not describe, and it can
hang. Freezing a value hides all three. Budget for them when you unfreeze.

## Scope discipline under a ratified plan

The review also flagged that a sustained `403` leaves the manager exactly as
blind as the `401` this ticket alarms on. True — and deliberately not fixed
here. The groomed plan scoped volet B to `Unauthorized`, the architect signed
that scope in a second pass, and AC3's anti-vacuity test *asserts* that
`Forbidden` does not fire. Widening the alarm in the same change would have
silently overturned a ratified acceptance criterion.

Carry the intent, route the overturn: the exclusion is now stated in the
`on_failure` docstring with its reason, and the gap is mika#2063. A known gap
with a ticket and a comment is honest engineering; the same gap fixed quietly
against a signed contract is not.

## Related

- mika#1968 — introduced the spawn-time resolution and wrote the hazard comment.
- mika#1781 — the other silent cause of flat throughput, found the same morning.
- RT#009 — "the failure does not shout, so nobody sees it." Volet B is an instance.
- mika#2063 — the same silence on the 403 door, left open on purpose (see above).
