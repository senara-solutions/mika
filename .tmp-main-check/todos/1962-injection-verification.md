# Injection verification — mika#1962

Per `feedback_verify_pipeline_passes_without_the_fix`: a test that passes with the
fix removed proves nothing. Each inversion below was **actually applied and run**,
not described. Results are transcribed from the runs, and the tree was restored to
baseline afterwards (verified: `server::` 378 passed, `home::` 37 passed).

Baseline before any inversion: `server::tier_guard` **9 passed, 0 failed**.

---

## I1 — Detection hardcoded to false

**What was changed.** Both detection axes short-circuited in
`server/tier_guard.rs::assert_family_tier_env_consistency`:

```rust
let evidence = if false && soul_has_family_marker(&agent_home)? {
} else if false && identity_allowlist_matches_family(&agent_home)? {
```

**Expected.** Every test asserting the guard *catches* drift fails; every test
asserting a *clean* start still passes (they assert `Ok`, which a guard that never
fires trivially satisfies).

**Observed — `FAILED. 4 passed; 5 failed`:**

| Test | Result |
|---|---|
| `refuses_start_when_family_provisioned_agent_runs_under_default_tier` | FAILED |
| `detects_pre_marker_family_agent_via_identity_allowlist_alone` | FAILED |
| `detects_family_agent_via_soul_marker_alone` | FAILED |
| `names_every_drifted_agent_not_just_the_first` | FAILED |
| `malformed_identity_stops_startup` | FAILED |
| `starts_clean_when_family_provisioning_matches_family_tier` | ok |
| `starts_clean_for_operator_provisioning_under_default_tier` | ok |
| `no_agents_is_a_clean_start` | ok |
| `agent_without_persona_files_is_a_clean_start` | ok |

**Verdict.** Exactly the predicted partition. The detection half of the guard is
load-bearing for all five catch-tests, and the clean-start tests are correctly
insensitive to it (they are not the tests that prove detection works — I1 is why
we know that).

---

## I2 — Tier comparison inverted

**What was changed.** The early return in the same function:

```rust
if tier != AgentTier::Family {   // was: ==
```

**Expected.** A different failure signature from I1 — the guard now short-circuits
on the *operator* tier (so every catch-test loses its bail) **and** evaluates on
the *family* tier (so the family-coherent clean-start test now wrongly bails).

**Observed — `FAILED. 3 passed; 6 failed`:**

The five I1 failures, **plus** `starts_clean_when_family_provisioning_matches_family_tier`
(FAILED — a correctly-configured family container would now refuse to start).

**Verdict.** Distinct signature from I1, and it is the *sixth* failure that
matters: it proves `starts_clean_when_family_provisioning_matches_family_tier` is
a real test of the tier comparison, not decoration. Had both inversions produced
the same five failures, that test would have been provably inert.

---

## I3 — One `ToolContext` site reverted to `from_env()`

**What was changed.** The first of the three `agent_loop/mod.rs` sites reverted
from `tier: params.tier` to `tier: mika_common::home::AgentTier::from_env()`.

**Expected.** No unit test fails — and that is the point. `AgentState.tier` is
still cached, so `agent_state_tier_survives_env_drift` still passes; the
regression is that *one consumer stopped reading the cache*. This class is not
reachable by a unit test at a sane cost (it would need a live agent turn under a
mutated process env), so the proof is **structural**, not behavioral.

**Observed.** `grep -rn "AgentTier::from_env()" crates/mika-agent/src/` gained a
hit the baseline does not have:

```
crates/mika-agent/src/agent_loop/mod.rs:3353:        tier: mika_common::home::AgentTier::from_env(),
```

**Verdict.** The DoD grep gate catches it. Baseline production hits are exactly
two, both in `server/mod.rs`, and both authorized:

| Line | Site | Why it is allowed |
|---|---|---|
| `server/mod.rs:514` | `init_agent` | The single read whose value populates `AgentState.tier` and `TaskDispatcher.tier`. |
| `server/mod.rs:709` | `run_server` guard callsite | The guard's whole job is to compare the env against disk; it must read the env. |

Two further hits (`server/mod.rs:1787`, `:1795`) are inside the
`agent_state_tier_survives_env_drift` test itself, where reading the env is the
subject under test.

**Precise gate wording** (the plan's original "zero hits outside the single
`init_agent` callsite" was too strict — it did not account for the guard callsite
the same ticket introduces):

> `grep -rn 'AgentTier::from_env()' crates/mika-agent/src/` must return no
> production hit outside `server/mod.rs`. Any hit in `agent_loop/`, `teams/`,
> `task_engine/`, or `server/investigate.rs` is a regression.

---

## What this exercise surfaced

I3 is the one that earned its keep. It showed the DoD grep as I had written it
during the plan-deepen would have **failed on the correct implementation** —
`server/mod.rs:709` is a legitimate second read that the ticket itself adds. A
gate that fires on correct code gets disabled the first time it fires, so the
wording was corrected here and in the plan's Definition of Done rather than left
to be discovered by whoever next touches this file.


---

## Post-review re-run (2026-08-27)

`/ce:review` produced six findings; five were fixed in-ticket (see the plan's
§ Review findings and resolutions). Those fixes changed the guard's body — new
enumeration (`servable_agent_names`), a narrowed malformed-TOML path, a derived
error message, and a new per-agent entry point — so I1 was re-run against the
corrected code rather than assumed to still hold.

**I1 re-run, same inversion (both detection axes short-circuited to `false`):**

| | Before fixes | After fixes |
|---|---|---|
| Baseline | 9 passed, 0 failed | **16 passed, 0 failed** |
| Detection disabled | 4 passed, 5 failed | **8 passed, 8 failed** |

Eight tests now fail where five did before, because the tests added for F1, F3
and F4 also depend on detection actually firing. The tree was restored and
re-verified at 16/16 green.

**Gate wording, corrected a second time.** The authorized-read set grew again
with the F1 fix (`server/state.rs`, the lazy-resolve callsite) and the F5 fix
(`server/tier_guard.rs`, the message-coherence comparison). The gate is now
stated by *role* rather than by file list, so it stops needing an amendment
every time the tier-resolution surface legitimately gains a member:

> No production `AgentTier::from_env()` hit outside the tier-resolution surface
> — `server/mod.rs` (init + boot guard), `server/state.rs` (lazy-resolve guard),
> `server/tier_guard.rs` (message coherence). A hit in `agent_loop/`, `teams/`,
> `task_engine/`, or `server/investigate.rs` is a regression: those are the
> per-turn consumers that must read the cache.

That this gate needed correcting twice, in the same ticket, is the finding worth
carrying forward — a structural gate written as a file whitelist ages badly
against its own feature. Written as "which *role* may read this", it does not.
