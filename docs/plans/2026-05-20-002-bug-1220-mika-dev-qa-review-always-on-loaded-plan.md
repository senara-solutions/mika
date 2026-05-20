---
ticket: mika#1220
type: bug
status: draft
created: 2026-05-20
branch: bug/1220/mika-dev-qa-review-skill-loaded-as
priority: p0-critical
---

# mika#1220 — qa-review skill loaded as always_on on mika-dev (and likely mika-qa/mika-relay)

## Summary

mika-dev's `run_gh` calls are being blocked by `validate_qa_review_gh_scope`, scoping the tool to qa-review's narrow allowlist (`pr review`, `pr diff`, `pr list`, `issue view`). This breaks every autonomous-loop step that needs `gh issue edit`, `gh pr create`, `gh pr merge`, `gh issue list`, etc. — the loop is structurally broken.

The ticket's initial framing (qa-review in `MIKA_DEV_IDENTITY` allowlist) is **wrong by inspection** — the static const at `crates/mika-agent/src/well_known_agents.rs:108-143` does NOT contain `qa-review`. The actual root cause is one layer deeper.

## Root cause

`provision_well_known_agents()` (line 437 of `well_known_agents.rs`) short-circuits on `mika_common::agent::agent_exists(home_dir, spec.name)` and skips the agent entirely (line 448-453: `continue`). Once a well-known agent has been provisioned on this host, its on-disk `identity.toml` is **frozen** — subsequent changes to the static `MIKA_*_IDENTITY` templates (e.g., the addition of `[skills].allowlist` in #815) never reach the file.

Empirical state on the affected host (verified via `cat ~/.mika/agents/<name>/identity.toml`):

| Agent | On-disk `[skills].allowlist` | Spec wants it? |
|-------|------------------------------|----------------|
| mika-dev | **missing** | YES (26-skill allowlist) |
| mika-qa | **missing** | YES (17-skill allowlist) |
| mika-relay | **missing** | YES (1-skill allowlist: `permission-policy`) |
| mika-arch | present (but missing `mika-arch-groom-milestone` + `[context.summary] inject = false`) | YES |

Consequence: in `server/mod.rs:402`, `if let Some(ref allowlist) = identity.skills.allowlist` evaluates to `None` for mika-dev → `apply_identity_allowlist()` is **not called** → the full bundled-skill set stays in the registry → `qa-review.skill.toml` has `always_on = true` → it matches every conversation-mode turn (including webhook events that arrive via `/message`) → `active_skill_paths` includes `qa-review` → `validate_qa_review_gh_scope` fires on every `run_gh` call.

The session in the ticket evidence (`6afe7739-6783-4a12-8fcb-e2aea32dfaf2`) is exactly this shape: `[GitHub] Issue labeled ready on senara-solutions/mika#1205` arrives via webhook, mika-dev's conversation turn runs with `qa-review` keyword/always-on matched, the engine tries `gh issue edit 1205 --remove-label ready`, scope check rejects.

The same wedge silently affects mika-qa and mika-relay — they happen not to need rejected subcommands, so it hides until an autonomous-loop dispatch tries one.

This is a **drift class**: anything code-owned in the identity template (`[skills].allowlist`, `[tools].disabled`, `[context.summary]`) does not propagate to existing well-known agents. Same shape as the disabled-skills drift that mika#1041 fixed for DB-backed `skill_overrides`.

## Fix design

Add a reconciliation step that runs on every server startup (same gate as `provision_well_known_agents`) and writes the security-critical sections of each well-known agent's identity.toml to match the static spec.

### Reconciled sections (code-owned)

| Section | Why code-owned |
|---------|----------------|
| `[skills].allowlist` | Security contract — `crates/mika-agent/CLAUDE.md` "Adding a New Bundled Skill" step 4 names the static const as the source of truth; operators add skills via PR, not by editing identity.toml |
| `[tools].disabled` | Same — defense-in-depth for read-only agent invariant (mika-arch); operators do not edit |
| `[context.summary]` (`inject` + `max_tokens`) | mika#1009 leak protection; code-owned per mika-arch's spec |

### Preserved sections (operator-owned)

`name`, `emoji`, `[reflection]`, `[kg]`. Operators legitimately customize these per host (e.g., mika-dev sets `[kg].enabled = true` on this host where the spec defaults to `false`; mika-dev has `name = "Mika Dev"` with the Sparkles emoji where the spec says `name = "Dev"`, `emoji = "🛠"`).

### Algorithm

For each agent in `WELL_KNOWN_AGENTS` where `agent_exists(home_dir, spec.name)`:

1. Render the **expected** identity content via `render_identity_content(spec, settings)` (already exists).
2. Parse the **on-disk** identity content via `toml::from_str::<toml::Value>`.
3. Parse the expected content the same way.
4. For each reconciled section above:
   - If expected has the section AND on-disk differs (missing OR not equal):
     - Replace the on-disk section with the expected one in the parsed `toml::Value` tree
     - Mark `changed = true`
5. If `changed`, serialize the merged value with `toml::to_string` and write to identity.toml.
6. Emit `info!` per agent with the list of reconciled sections and `warn!` per skipped agent (parse failure, write failure).

### Failure isolation

- Parse failure (on-disk file is malformed): log `warn!`, skip THIS agent, continue with the others. The existing fail-closed parse path in `prompt::load_identity()` (#811) protects the running agent from a malformed file.
- Render failure on the spec side (e.g., mika-arch when `MIKA_KG_DOCS_ROOTS` is unset): log `warn!`, skip THIS agent, continue. Same shape as `provision_well_known_agents()`.
- Write failure: log `warn!`, leave the file as-is, continue. The agent will start with stale identity but the existing #815 + #811 protections (fail-closed parse, identity allowlist no-op when None) prevent worse-than-current-state outcomes.

### Disable gate

Same `MIKA_DISABLE_AGENT_PROVISIONING` env var. When set, reconciliation is skipped with a `warn!` log — operators iterating on identity files locally retain control.

## Placement

Insert reconciliation **into** `provision_well_known_agents()` at the existing `agent_exists` branch (line 448-453). The "skip" branch becomes a "reconcile" branch. Single function, single call site, lifecycle stays obvious.

```rust
if mika_common::agent::agent_exists(home_dir, spec.name) {
    reconcile_well_known_identity(home_dir, spec, settings);  // NEW
    continue;
}
```

The new function lives in the same module (`well_known_agents.rs`).

## Acceptance criteria

**AC1.** After a server restart on the affected host, `cat ~/.mika/agents/mika-dev/identity.toml` shows a `[skills]` block with an `allowlist` array containing `self-dev`, `dev-pilot`, `dev-groom`, `run_gh`-using skills, etc. — matching `MIKA_DEV_IDENTITY` line-for-line on the `[skills]` section. Same for `mika-qa` and `mika-relay`.

**AC2.** On the running server, `validate_qa_review_gh_scope` returns `Ok(())` on mika-dev turns (verified by sending a webhook `ready`-label event and observing `gh issue edit` succeed — or, equivalent, by a unit test that builds a `SkillRegistry` post-reconciliation and asserts `qa-review` is NOT in the registry's `skills` vec for the mika-dev agent_id).

**AC3.** Operator-customized sections (`name`, `emoji`, `[reflection]`, `[kg]`) on the existing on-disk identity files are **preserved verbatim** after reconciliation — verified by diffing before/after the first reconciler run.

**AC4.** mika-arch's identity is reconciled to include `mika-arch-groom-milestone` in the allowlist AND `[context.summary] inject = false` (currently missing on this host).

**AC5.** Second startup after deploy is a no-op (idempotency): the reconciler sees no drift, writes nothing, logs a single info line per agent ("identity in sync").

**AC6.** Regression test: `tests/eval/test_qa_review_run_gh_scope_validator.rs` covers the existing positive case (validator fires when qa-review IS active). A new test verifies the validator does NOT fire when mika-dev's reconciled identity allowlist excludes qa-review (i.e., the post-fix happy path).

## Test plan

### Unit tests in `well_known_agents.rs`

Add tests in the existing `#[cfg(test)] mod tests` block:

1. `test_reconcile_adds_missing_allowlist_for_mika_dev` — provision an agent with a pre-#815-shape identity.toml (no `[skills]` block), run the reconciler, assert the file now contains `[skills].allowlist` matching the static const.

2. `test_reconcile_preserves_operator_kg_and_reflection` — pre-seed identity.toml with operator-customized `[kg].docs_root` and `[reflection]` blocks plus the missing `[skills]` block, run the reconciler, assert `[kg]`/`[reflection]` are byte-identical to pre-state AND `[skills]` is now present.

3. `test_reconcile_overwrites_drifted_allowlist` — pre-seed with `[skills].allowlist = ["only-self-dev"]`, run reconciler, assert the allowlist matches the static const (drift in code-owned section is reset; operator does not get to weaken security via identity edit).

4. `test_reconcile_idempotent` — run reconciler twice in a row, assert second call writes nothing (check via file mtime or via a `changed: bool` return value).

5. `test_reconcile_adds_mika_arch_missing_groom_milestone` — pre-seed mika-arch identity missing `mika-arch-groom-milestone` from allowlist, run reconciler, assert it's added.

6. `test_reconcile_adds_mika_arch_missing_context_summary` — pre-seed mika-arch identity without `[context.summary]`, run reconciler, assert `inject = false` is now present.

7. `test_reconcile_skips_user_defined_agent` — provision a non-well-known agent name (e.g., "operator-custom-agent"), run reconciler against the global home, assert that agent's identity.toml is not touched (the reconciler only iterates `WELL_KNOWN_AGENTS`).

8. `test_reconcile_disabled_via_env` — set `disable_agent_provisioning = true`, run reconciler, assert no writes (matches `provision_well_known_agents()` behavior).

9. `test_reconcile_handles_malformed_identity` — write garbage to identity.toml, run reconciler, assert it logs warn and continues (no panic, no partial write).

### Integration test

In `tests/eval/test_qa_review_run_gh_scope_validator.rs`, add a new scenario `test_validator_skipped_when_mika_dev_post_reconcile`:
- Construct a `SkillRegistry` from the bundled skills directory
- Apply `MIKA_DEV_IDENTITY`'s allowlist via `apply_identity_allowlist`
- Build a `ToolContext` with `active_skill_paths` derived from the post-allowlist registry
- Call `validate_qa_review_gh_scope` with `["issue", "edit", "1205"]`
- Assert it returns `Ok(())` (qa-review is not in the post-reconciliation active set)

### Deploy-side smoke test

After the binary lands on the host:
1. `cat ~/.mika/agents/mika-dev/identity.toml` → expect `[skills].allowlist` present
2. Trigger a webhook event (`gh issue edit <test-ticket> --add-label ready`) and watch mika-dev process it
3. Confirm the previously-blocked `gh issue edit ... --remove-label ready` call now succeeds in the agent loop
4. `grep validate_qa_review_gh_scope $MIKA_SERVER_LOG_FILE` returns no rejection events for mika-dev sessions post-deploy

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Reconciler corrupts identity.toml mid-write, leaving a partial file | Write to `identity.toml.tmp` then `std::fs::rename` (atomic on Linux). Use `mika_common::fs::write_atomic` if it exists; else inline the pattern. |
| Operator was relying on the broken-allowlist behavior (e.g., had silently disabled mika-dev's restrictive scope) | Mitigated by changelog note + the existing `MIKA_DISABLE_AGENT_PROVISIONING` env var. Operator can opt out. The "fix" returns mika-dev to its documented behavior (#815) — staying broken is not an option since the autonomous loop is wedged. |
| TOML serialization drops comments | Identity templates contain no comments today (verified via `cat`). Add a static-string comment-preservation note in the function docstring; if templates gain comments later, switch to `toml_edit` (currently overkill). |
| Reconciler runs in `dev_mode = true` only (gated by the surrounding `if settings.dev_mode { }` block in `run_server`) | Production hosts run `dev_mode = false` and rely on operator-curated identity files. That's the right scoping — same as `provision_well_known_agents()` itself. No change needed. |
| Drift reconciliation triggers a `skills_dirty` flush mid-turn | Reconciler runs in `run_server` BEFORE per-agent init / skill loading. No mid-turn concern. |

## Sequencing

Single PR, single commit (or a small commit chain — reconciler + tests + compound doc).

1. Add `reconcile_well_known_identity()` function in `well_known_agents.rs`
2. Wire into the existing `agent_exists` branch of `provision_well_known_agents()`
3. Add the 9 unit tests
4. Add the integration test in `tests/eval/test_qa_review_run_gh_scope_validator.rs`
5. `cargo test -p mika-agent` and `cargo clippy` pass
6. `make deploy` → cat the identity files → confirm reconciliation took effect
7. Trigger a real webhook on a test issue and confirm `gh issue edit` succeeds
8. Compound doc in `docs/solutions/best-practices/identity-toml-drift-from-static-spec-2026-05-20.md`

## Compound notes

This is the **second** time identity-drift has bitten the autonomous loop:

1. mika#1041 — `disabled_skills` drift in DB-backed `skill_overrides` (fixed by reconciler in `seed_well_known_skill_overrides`)
2. mika#1220 (this ticket) — `[skills].allowlist` drift in on-disk identity.toml (this fix)

The compound pattern is: **any code-owned configuration of a long-lived per-agent artifact requires explicit reconciliation, not just first-creation seeding.** The `agent_exists` short-circuit is structurally hostile to evolving the static spec. Future identity-template changes (next year's #815-class restructure) will hit the same shape unless reconciler keeps pace.

Out of scope for this PR but worth filing as a follow-up: a startup-time invariant test that **fails the build** if the spec contains a section that the reconciler doesn't cover. Today the reconciler covers `[skills]`, `[tools]`, `[context.summary]` — if a future spec adds `[security]` and reconciler isn't updated, drift returns silently. A test that iterates over the spec's section names and asserts each is either reconciled or whitelisted-as-operator-owned would catch this at CI time.

## Cross-repo impact

None. mika core only. No mika-cloud, mika-skills, or claude-pilot changes needed. The fix is bounded to `crates/mika-agent/src/well_known_agents.rs` + tests.

## Reference

- Static spec: `crates/mika-agent/src/well_known_agents.rs:78-227` (MIKA_DEV, MIKA_DEV_IDENTITY, MIKA_QA, MIKA_QA_IDENTITY, MIKA_RELAY, MIKA_RELAY_IDENTITY)
- Allowlist application: `crates/mika-agent/src/skills/mod.rs:390-426` (apply_identity_allowlist)
- Loader: `crates/mika-agent/src/server/mod.rs:374-410` (load_identity → apply_identity_allowlist → apply_overrides)
- Validator that fires: `crates/mika-agent/src/skills/builtin_handlers.rs:1806-1833` (validate_qa_review_gh_scope)
- Related precedents:
  - mika#815 — identity-driven skill allowlist introduced
  - mika#811 — fail-closed identity parse for well-known agents
  - mika#1009 — `[context.summary] inject = false` introduced for mika-arch
  - mika#1041 — disabled_skills drift reconciler in `seed_well_known_skill_overrides`
  - mika#1196 — `validate_qa_review_gh_scope` introduced
