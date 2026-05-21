---
ticket: mika#1224
type: bug
status: draft
created: 2026-05-21
branch: bug/1224/skills-qa-review-skill-loaded-on-mika
priority: p1
---

# mika#1224 — qa-review skill loaded on mika-dev despite identity allowlist exclusion

## Summary

mika-dev's session `6afe7739-6783-4a12-8fcb-e2aea32dfaf2` shows all 9 `llm_calls.prompt_variant`
rows include `"qa-review":"base"` despite `qa-review` NOT being in `MIKA_DEV_IDENTITY.allowlist`.
The contamination caused `run_gh issue edit` to be rejected by `validate_qa_review_gh_scope`, which
created the precondition for the engine correction in mika#1221.

## Investigation — Hypothesis verification

The ticket's hypothesis:

> `apply_identity_allowlist()` runs BEFORE `apply_overrides()`, but an `always_on` flag is being
> applied **before** the allowlist filter — incorrect ordering.

**This hypothesis is incorrect.** The code ordering is correct:

1. `apply_identity_allowlist()` — Phase -1, evicts non-allowlisted skills (`mod.rs:390-426`)
2. `apply_overrides()` — Phase 0 (DB disable) + Phase 1 (always_on/LLM overrides) (`mod.rs:436-513`)

Phase -1 runs first. A skill evicted by the allowlist cannot be resurrected by `always_on` at
Phase 1 — Phase 1 searches only within `self.skills` (the surviving set). The ordering is confirmed
by both the code and existing tests (`test_identity_allowlist_db_disable_wins` at `mod.rs:2741`).

## Root cause — shared with mika#1220

The actual root cause is one layer deeper: **`apply_identity_allowlist()` is never called at all
for mika-dev.**

`provision_well_known_agents()` (`well_known_agents.rs:437-522`) short-circuits on
`agent_exists()` (line 448-453: `continue`). Once a well-known agent has been provisioned, its
on-disk `identity.toml` is frozen — subsequent changes to the static `MIKA_*_IDENTITY` templates
(e.g., the addition of `[skills].allowlist` in #815) never reach the file.

Empirical state on the affected host:

| Agent | On-disk `[skills].allowlist` | Spec expects it? |
|-------|------------------------------|------------------|
| mika-dev | **missing** | YES (26-skill allowlist) |
| mika-qa | **missing** | YES (17-skill allowlist) |
| mika-relay | **missing** | YES (1-skill allowlist) |
| mika-arch | present (partial drift) | YES |

Consequence: in `server/mod.rs:402`, `if let Some(ref allowlist) = identity.skills.allowlist`
evaluates to `None` for mika-dev → `apply_identity_allowlist()` is never called → the full
bundled-skill set stays in the registry → `qa-review.skill.toml` has `always_on = true` → it
matches every conversation-mode turn → `active_skill_paths` includes `qa-review` →
`validate_qa_review_gh_scope` fires on every `run_gh` call.

This is the same root cause documented in the mika#1220 plan at
`docs/plans/2026-05-20-002-bug-1220-mika-dev-qa-review-always-on-loaded-plan.md`.

## Resolution — covered by mika#1220

mika#1220's fix adds a reconciliation step to `provision_well_known_agents()` that overwrites
code-owned sections of identity.toml (including `[skills].allowlist`) on every startup, even for
existing agents. This directly resolves the contamination observed in #1224.

The `desktop` skill mentioned in #1224's scope is not a bundled skill — it is a community/custom
skill that was manually installed on the host. It would also be evicted by the allowlist once the
reconciler runs.

## Test coverage — already exists

The unit test requested by #1224 already exists as Scenario 3 in
`crates/mika-agent/tests/eval/test_qa_review_run_gh_scope_validator.rs`:

```
qa_review_evicted_by_mika_dev_allowlist_after_reconcile
```

This test:
1. Seeds a registry containing qa-review (mirroring the pre-fix on-disk shape)
2. Applies `MIKA_DEV_IDENTITY`'s `[skills].allowlist` via `apply_identity_allowlist()`
3. Asserts qa-review is evicted from the registry
4. Runs `gh issue edit … --remove-label ready` and asserts no scope rejection

This is exactly the test #1224 requests: "feed mika-dev's identity + a bundled set containing
qa-review + assert qa-review is evicted."

## Scope assessment

All of #1224's acceptance criteria are either already verified or will be resolved by #1220:

| #1224 scope item | Status |
|------------------|--------|
| Read `apply_identity_allowlist()` + `apply_overrides()` code path | ✅ Done — ordering is correct |
| Determine why qa-review survives eviction for mika-dev | ✅ Done — allowlist never applied (identity.toml drift) |
| Verify the fix evicts contaminating skills | ✅ Covered by mika#1220 reconciler |
| Add unit test for `apply_identity_allowlist()` + identity-driven skill selection | ✅ Already exists (Scenario 3) |
| `gh issue edit` scope-filter regression check | ✅ Already exists (Scenario 3) |

## Recommendation

Close mika#1224 as duplicate of mika#1220. The root cause, fix, and tests are all shared.
No additional code changes are needed beyond what mika#1220 delivers.

## Critical files

| File | Role |
|------|------|
| `crates/mika-agent/src/well_known_agents.rs:437-522` | `provision_well_known_agents` — the short-circuit that causes the drift |
| `crates/mika-agent/src/well_known_agents.rs:126-161` | `MIKA_DEV_IDENTITY` — the spec with `[skills].allowlist` |
| `crates/mika-agent/src/skills/mod.rs:390-426` | `apply_identity_allowlist` — correctly implemented, just never called |
| `crates/mika-agent/src/server/mod.rs:374-410` | init_agent skill setup — `if let Some(ref allowlist)` evaluates to `None` |
| `skills/bundled/qa-review/skill.toml:5` | `always_on = true` — does NOT bypass the allowlist; irrelevant to root cause |
| `tests/eval/test_qa_review_run_gh_scope_validator.rs:228-300` | Scenario 3 — the test #1224 requests, already exists |
| `docs/plans/2026-05-20-002-bug-1220-*.md` | mika#1220 plan — the reconciler that fixes this |
