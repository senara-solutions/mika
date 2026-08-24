# Plan — fix(manager,auth): verify_gh_auth milestone-scoped probe

**Status:** DRAFT (nuit wave 2026-08-23)
**Date:** 2026-08-23
**Ticket:** mika#1974
**Parent:** mika#1968 (PR#1973 shipped; A4 P1 deferred)
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** post-ship hardening — fidelity gap in boot-time GitHub auth verifier

## Why

mika#1968 shipped `verify_gh_auth` calling `gh api /rate_limit` at boot. Adversarial review of PR#1973 flagged **A4 P1**: `/rate_limit` succeeds on any authenticated PAT even when the token belongs to a **wrong user** with no access to the target milestone repo. The verifier's job — surface auth gaps loudly at boot before cycle-error spam accumulates — is exactly the failure mode `/rate_limit` cannot catch. Every cycle then 403s on the actual milestone read, which is the class of silent-degradation the boot-time check exists to prevent.

**Evidence (verbatim):** verdict comment on PR#1973 — "A4 P1: verify_gh_auth should probe milestone-scoped endpoint (`gh api /repos/{repo}/milestones/{n}`) not just /rate_limit — catches wrong-user PAT class".

**Fidelity boundary docstring already flags the gap** — `crates/mika-agent/src/milestone_manager/spawn.rs:427-433` explicitly documents that `/rate_limit` "does NOT confirm the token has scope/reachability for the specific target milestone repo". This plan closes that gap.

## Codebase reality (verified, not inferred)

- `verify_gh_auth` signature: `pub async fn verify_gh_auth<R: GhRunner>(runner: &R) -> Result<u64, GhAuthError>` (`spawn.rs:434`). Currently probes `runner.run(&["api", "/rate_limit"])`.
- Caller: `spawn_manager_cycle_task` at `spawn.rs:251` — takes success return as `rate_limit_remaining` and logs it in `manager_gh_auth_check_ok`. On failure logs `manager_gh_auth_check_failed` with fixed hint about MIKA_GITHUB_TOKEN.
- `AuthClass` enum: `Unauthorized` (401), `Forbidden` (403), `Network`, `Other` (`spawn.rs:352-373`).
- `classify_cycle_error` string-classifies errors from `gh` stderr; no 404 branch — 404s currently fall through to `Other`.
- `GhRunner` trait + `ProcessGhRunner`: `crates/mika-agent/src/milestone_manager/reader.rs:31-66`. `ProcessGhRunner.run` returns `Err(anyhow!("gh {} failed: {}", args.join(" "), stderr))` on non-zero exit.
- `MilestoneRef { repo: String, number: u32 }`: `types.rs`. Already threaded through `ManagerConfig.target`.
- Existing tests in `spawn.rs:1074-1179`: 401 classify, success-with-remaining, malformed-body, 403/network classify.
- LECTURE-SEULE invariant: `no_dispatch_test.rs` FORBIDDEN_TOKENS includes `"\"PATCH\""` etc. — our new probe is a GET (`gh api <path>` defaults to GET), no conflict.

## What

Replace `/rate_limit` probe by milestone-scoped `/repos/{owner}/{repo}/milestones/{number}` probe. **REPLACE (not compose)** — the milestone GET is a superset: success proves both token validity AND repo/milestone access; failure gives us stronger discrimination (404/401/403). Composing `/rate_limit` + milestone adds one API call for zero fidelity gain.

### 1. `AuthClass::MilestoneNotFound` variant

**File:** `crates/mika-agent/src/milestone_manager/spawn.rs`.

Add `MilestoneNotFound` variant to `AuthClass` enum. `as_str()` returns `"404_milestone_not_found"` — the `_milestone_not_found` suffix distinguishes it from generic 404s in `manager_cycle_error` grep (`auth_class=404_milestone_not_found` vs a hypothetical `auth_class=404` from a future non-milestone probe).

**Rationale:** the enum is the structural axis for operator grep. `classify_cycle_error` returns `Other` on 404 today (no branch matches); leaving 404 as `Other` in the milestone-probe context would collapse the 404-vs-500-vs-other-server-error signal that AC2 specifically requires. Adding a variant forces exhaustive-match callsites to acknowledge the new class.

### 2. `classify_milestone_probe_error` classifier

**File:** `crates/mika-agent/src/milestone_manager/spawn.rs`.

New function `fn classify_milestone_probe_error(err_text: &str) -> AuthClass` — checks for 404 first, else delegates to existing `classify_cycle_error`. Keeps the cycle-error classifier unchanged (404 during a cycle body — e.g., an issue that got deleted — is legitimately `Other`; the milestone probe is the sole context where 404 means "target milestone gone / wrong repo").

### 3. `verify_gh_auth` rewrite

**File:** `crates/mika-agent/src/milestone_manager/spawn.rs`.

New signature: `pub async fn verify_gh_auth<R: GhRunner>(runner: &R, target: &MilestoneRef) -> Result<(), GhAuthError>`.

Probe path: `format!("/repos/{}/milestones/{}", target.repo, target.number)`.

Parse-or-fail success discipline mirrors current `A2 P1` shape: a successful HTTP response with body missing `number` (or with `number != target.number` — proves URL was rewritten by GitHub) is a fidelity gap that MUST surface as `AuthClass::Other` with `parse_failure:` prefix on `stderr_head`. The verifier's `/rate_limit → Ok(0)` regression class is the exact regression this discipline prevents.

Return type drops the `u64` — `rate_limit_remaining` was a diagnostic side-quest that only made sense while `/rate_limit` was the probe. The new success signal is binary ("milestone endpoint reachable with matching number"); further diagnostics live in the caller's log line.

### 4. Caller update — per-class operator hints

**File:** `crates/mika-agent/src/milestone_manager/spawn.rs` (in `spawn_manager_cycle_task`).

Update `verify_gh_auth(&runner, &cfg.target)` call site. Success arm: log `manager_gh_auth_check_ok` without `rate_limit_remaining` field (add `milestone = %cfg.target.as_display()` which is already there — no schema change to the field set beyond removing the u64). Failure arm: switch from single fixed hint to per-class hint via `match auth_class`:

- `Unauthorized` (401): "token missing/invalid/expired — check `tr '\\0' '\\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_GITHUB_TOKEN`"
- `Forbidden` (403): "token authenticated but lacks scope for {repo} — check GitHub App installation or PAT org access"
- `MilestoneNotFound` (404): "milestone {repo}#{number} not found — check `MIKA_MANAGER_TARGET_MILESTONE` value or milestone existence on GitHub"
- `Network`: "gh cannot reach GitHub — check network/DNS/TLS from daemon host"
- `Other`: "unexpected failure — see stderr_head"

Each hint is a `match` arm inline in the `error!` macro invocation. Named class discrimination via structured `auth_class` field remains the primary greppable signal; hints are the operator-facing human touch.

### 5. Regression test — wrong-user PAT scenario (AC3)

**File:** `crates/mika-agent/src/milestone_manager/spawn.rs` `#[cfg(test)] mod tests`.

Introduce arg-aware mock `ArgAwareRunner` that returns different responses based on the probe path (`/rate_limit` vs `/repos/.../milestones/N`). Regression test `verify_gh_auth_catches_wrong_user_pat_class` asserts:

1. Mock returns success on `/rate_limit` (would pass old check).
2. Mock returns 403 on milestone endpoint.
3. `verify_gh_auth(&mock, &target)` returns `Err(GhAuthError { auth_class: AuthClass::Forbidden, .. })`.

This locks the founding-incident class: **any regression that reverts to `/rate_limit`-only will fail this test**. Also add:
- `verify_gh_auth_404_returns_err_milestone_not_found` — 404 on milestone endpoint → `MilestoneNotFound`.
- `verify_gh_auth_403_returns_err_forbidden` — 403 on milestone endpoint → `Forbidden`.
- `verify_gh_auth_success_returns_ok` — valid milestone JSON body → `Ok(())`.

Update existing tests to new signature:
- `verify_gh_auth_401_returns_err_unauthorized` — pass `&target`.
- `verify_gh_auth_malformed_body_returns_err_other` — case bodies rewritten for milestone-endpoint context (missing `number` field, empty JSON, mismatched number).

## How — implementation steps

1. Add `MilestoneNotFound` variant to `AuthClass` + `as_str()` arm.
2. Add `classify_milestone_probe_error` function.
3. Rewrite `verify_gh_auth` signature and body per §3.
4. Update `verify_gh_auth` caller in `spawn_manager_cycle_task` per §4.
5. Update existing tests + add 4 new tests per §5.
6. `cargo test -p mika-agent milestone_manager::spawn` — must pass.
7. `cargo clippy -p mika-agent --all-targets` — no new warnings.
8. `cargo test -p mika-agent milestone_manager` — full module suite passes (includes `no_dispatch_test` — verifies our GET-only change respects LECTURE-SEULE).

## Definition of Done

- `verify_gh_auth` probes milestone-scoped endpoint (verifiable via git-stat and by test that asserts the arg-list).
- `AuthClass::MilestoneNotFound` variant exists; `classify_milestone_probe_error` catches 404s.
- Caller log discriminates all four failure classes (401/403/404/network) with operator-actionable hints.
- Regression test for wrong-user PAT class passes.
- Existing tests updated to new signature; no test deleted without justification.
- `cargo test -p mika-agent milestone_manager` green.
- `cargo clippy -p mika-agent --all-targets` clean.
- `no_dispatch_test` still passes (GET-only probe respects LECTURE-SEULE).

## Acceptance criteria

- [ ] **AC1**: `verify_gh_auth` probes `gh api /repos/{repo}/milestones/{n}` where `n = MIKA_MANAGER_TARGET_MILESTONE` number (verified by test asserting the arg-list passed to `GhRunner.run`).
- [ ] **AC2**: log discriminates milestone-not-found (404) vs auth-failed (401) vs forbidden-milestone (403) — each with operator hint (verified by inspection of `spawn_manager_cycle_task` error arm + `manager_gh_auth_check_failed` field set).
- [ ] **AC3**: regression test — mock PAT valid on `/rate_limit` but invalid on milestone endpoint → `verify_gh_auth` catches (test `verify_gh_auth_catches_wrong_user_pat_class`).

## Non-goals

- Composing with `/rate_limit` (replaced entirely — see § What).
- Changing `classify_cycle_error` (cycle-body 404s remain `Other` — orthogonal concern).
- App token TTL refresh (mika#1968 A3 P1 — separate deferred ticket).
- Rate-limit remaining diagnostic (was a nice-to-have on `/rate_limit`; loses its home with the probe change and is not worth adding a second API call).
