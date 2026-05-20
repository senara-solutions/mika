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

## Phase 0 Pin — load-bearing sites

Concrete anchors the implementer must read before writing any code. All paths are relative to the mika repo root.

| # | Site | What it does | Why it matters |
|---|------|--------------|----------------|
| 1 | `crates/mika-agent/src/well_known_agents.rs:437-522` (`provision_well_known_agents`) | Iterates `WELL_KNOWN_AGENTS`; for each, calls `agent_exists` and `continue`s on hit (line 448-453) | This is where the bug lives — the `continue` short-circuits drift propagation. Reconciliation replaces this `continue`. |
| 2 | `crates/mika-agent/src/well_known_agents.rs:108-143` (`MIKA_DEV_IDENTITY`) | Static const template for mika-dev identity.toml — contains `[skills].allowlist` with 26 entries | Source of truth for mika-dev's allowlist. Reconciler reads it via `render_identity_content` (line 410-422). |
| 3 | `crates/mika-agent/src/well_known_agents.rs:168-194` (`MIKA_QA_IDENTITY`) | Same shape for mika-qa — 17-entry allowlist | Same role as #2. |
| 4 | `crates/mika-agent/src/well_known_agents.rs:220-227` (`MIKA_RELAY_IDENTITY`) | mika-relay — 1-entry allowlist (`permission-policy`) | Same role as #2. |
| 5 | `crates/mika-agent/src/well_known_agents.rs:333-383` (`build_mika_arch_identity`) | Computed identity for mika-arch — runtime-resolved `[kg].docs_roots` + `[skills].allowlist` + `[tools].disabled` + `[context.summary] inject = false` | mika-arch's expected identity is COMPUTED, not static. Reconciler invokes `render_identity_content(spec, settings)` (which dispatches to this function) rather than reading a const directly. |
| 6 | `crates/mika-agent/src/well_known_agents.rs:295-325` (`MIKA_ARCH_DISABLED_TOOLS`) | The `[tools].disabled` array for mika-arch — 22 platform-mutational tools | Reconciler must overwrite the on-disk `[tools].disabled` to match this const. |
| 7 | `crates/mika-agent/src/well_known_agents.rs:410-422` (`render_identity_content`) | Resolves the spec's `identity_source` to a string (Static / Computed / default) | Existing function the reconciler calls to get the canonical expected content per agent. |
| 8 | `crates/mika-agent/src/skills/mod.rs:390-426` (`apply_identity_allowlist`) | Phase -1 of the skill registry filter — evicts non-allowlisted skills when `allowlist` is non-empty | The wedge: when on-disk identity has no `[skills]` block, `identity.skills.allowlist` is `None` and this function is never called (per `if let Some(...)` in callers). |
| 9 | `crates/mika-agent/src/server/mod.rs:374-410` (`init_agent` skill setup) | Loads identity, conditionally applies allowlist, applies DB overrides | The downstream consumer that depends on the reconciler having run first. |
| 10 | `crates/mika-agent/src/server/mod.rs:579-597` (`run_server` entry) | Calls `provision_well_known_agents` on dev_mode startup | Where reconciliation lifecycle starts. Reconciliation is internal to #1, so no new call site here. |
| 11 | `crates/mika-agent/src/skills/builtin_handlers.rs:1806-1833` (`validate_qa_review_gh_scope`) | The validator that fires on `run_gh` when qa-review is in `active_skill_paths` | The observable wedge. Post-fix, qa-review is evicted from mika-dev's registry → never reaches `active_skill_paths` → validator no-ops. Direct correctness signal. |
| 12 | `crates/mika-agent/src/well_known_agents.rs:618-786` (`seed_well_known_skill_overrides`) | The DB-side reconciler for `disabled_skills` and `llm_overrides` drift (mika#1041) | **Pattern precedent.** New identity reconciler mirrors this shape (idempotent, reconcile-on-restart, info-log on changes). |
| 13 | `skills/bundled/qa-review/skill.toml` (`always_on = true`) | Marks qa-review as always-on for matching | Confirmed: this is why qa-review fires without keyword match. No change here — fix is in identity allowlist enforcement. |
| 14 | `~/.mika/agents/<name>/identity.toml` (on host) | Empirical state of the four well-known agents | Verified: mika-dev/qa/relay missing `[skills]`; mika-arch missing `mika-arch-groom-milestone` + `[context.summary]`. |

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

| Section path | Why code-owned |
|--------------|----------------|
| `skills.allowlist` | Security contract — `crates/mika-agent/CLAUDE.md` "Adding a New Bundled Skill" step 4 names the static const as the source of truth; operators add skills via PR, not by editing identity.toml |
| `tools.disabled` | Same — defense-in-depth for read-only agent invariant (mika-arch); operators do not edit |
| `context.summary` (both `inject` and `max_tokens` fields) | mika#1009 leak protection; code-owned per mika-arch's spec |

### Preserved sections (operator-owned)

`name`, `emoji`, `[reflection]`, `[kg]`. Operators legitimately customize these per host (e.g., mika-dev sets `[kg].enabled = true` on this host where the spec defaults to `false`; mika-dev has `name = "Mika Dev"` with the Sparkles emoji where the spec says `name = "Dev"`, `emoji = "🛠"`).

### Structural enforcement of the boundary (`const CODE_OWNED_IDENTITY_SECTIONS`)

To prevent prose-only drift on what's code-owned, the reconciler is driven by a single explicit constant in `well_known_agents.rs`:

```rust
/// Identity-toml section paths that the reconciler owns from the static spec.
///
/// **Each entry is a dotted path** from the root of `identity.toml`. The reconciler
/// walks each path in both the expected (spec-rendered) tree and the on-disk tree;
/// when they differ, the expected subtree replaces the on-disk subtree.
///
/// Adding a new code-owned section to `WellKnownAgent`'s identity templates requires
/// adding an entry here AND adding a unit test in `tests` below — the
/// `test_code_owned_sections_have_reconciler_coverage` test iterates this constant
/// and fails the build if any entry is not exercised by a test.
///
/// Sections NOT listed here are preserved verbatim from the on-disk file (operator-owned:
/// `name`, `emoji`, `[reflection]`, `[kg]`).
pub const CODE_OWNED_IDENTITY_SECTIONS: &[&str] = &[
    "skills.allowlist",
    "tools.disabled",
    "context.summary",
];
```

The reconciler iterates this constant rather than open-coding the three paths. Adding a future code-owned section is a one-line append (plus a test) — no logic change needed.

A complementary build-time invariant test (in the same module's `tests` block) asserts that each path in `CODE_OWNED_IDENTITY_SECTIONS` corresponds to a section that at least one well-known agent's rendered identity actually emits — catches typos in the constant. Specifically: for each path `P`, walk the rendered identity of every `WELL_KNOWN_AGENTS` entry; assert that at least one produces a non-empty value at `P`. This catches the case where someone adds `"foo.bar"` to the constant but no spec ever emits `[foo].bar`.

A second test asserts the negative direction: for every dotted path the rendered specs emit beyond `name`/`emoji`/`reflection`/`kg`, the path is present in `CODE_OWNED_IDENTITY_SECTIONS`. This catches the dual case where someone adds `[security]` to `MIKA_ARCH_DISABLED_TOOLS`-shape spec but forgets to add `"security"` to the constant — silent drift returns. Both tests together close the loop. (See "Test plan" below for exact test names.)

### Algorithm

For each agent in `WELL_KNOWN_AGENTS` where `agent_exists(home_dir, spec.name)`:

1. Render the **expected** identity content via `render_identity_content(spec, settings)` (already exists).
2. Parse the **on-disk** identity content via `toml::from_str::<toml::Value>`.
3. Parse the expected content the same way.
4. **For each dotted path in `CODE_OWNED_IDENTITY_SECTIONS`:**
   - Resolve the path in the expected tree (`get_path(&expected, "skills.allowlist")`)
   - Resolve the path in the on-disk tree
   - If expected exists AND (on-disk missing OR on-disk != expected):
     - Set the path in the on-disk tree to the expected value (`set_path(&mut on_disk, "skills.allowlist", value.clone())`)
     - Record the path in a `reconciled_paths: Vec<&str>` for logging
5. If `reconciled_paths` is non-empty, serialize the merged on-disk tree with `toml::to_string` and atomic-write to identity.toml (`.tmp` + rename).
6. Emit one `info!` per agent: `agent=<name> reconciled_paths=[skills.allowlist, ...]` (empty array → in-sync log). Emit `warn!` on parse failure / write failure / spec render failure, skipping THAT agent only.

Helper: `get_path(value: &toml::Value, dotted_path: &str) -> Option<&toml::Value>` and `set_path(value: &mut toml::Value, dotted_path: &str, new: toml::Value)`. Path syntax is dot-separated table keys; e.g., `"context.summary"` resolves `value["context"]["summary"]`. No array indexing needed — none of the code-owned sections are arrays at the top level (they may CONTAIN arrays, but the path itself targets a table or scalar). `set_path` creates intermediate empty tables when the parent doesn't exist on-disk (the missing-section case).

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

Add 11 tests in the existing `#[cfg(test)] mod tests` block (9 reconciliation tests + 2 constant-coverage invariant tests):

1. `test_reconcile_adds_missing_allowlist_for_mika_dev` — provision an agent with a pre-#815-shape identity.toml (no `[skills]` block), run the reconciler, assert the file now contains `[skills].allowlist` matching the static const.

2. `test_reconcile_preserves_operator_kg_and_reflection` — pre-seed identity.toml with operator-customized `[kg].docs_root` and `[reflection]` blocks plus the missing `[skills]` block, run the reconciler, assert `[kg]`/`[reflection]` are byte-identical to pre-state AND `[skills]` is now present.

3. `test_reconcile_overwrites_drifted_allowlist` — pre-seed with `[skills].allowlist = ["only-self-dev"]`, run reconciler, assert the allowlist matches the static const (drift in code-owned section is reset; operator does not get to weaken security via identity edit).

4. `test_reconcile_idempotent` — run reconciler twice in a row, assert second call writes nothing (check via file mtime or via a `changed: bool` return value).

5. `test_reconcile_adds_mika_arch_missing_groom_milestone` — pre-seed mika-arch identity missing `mika-arch-groom-milestone` from allowlist, run reconciler, assert it's added.

6. `test_reconcile_adds_mika_arch_missing_context_summary` — pre-seed mika-arch identity without `[context.summary]`, run reconciler, assert `inject = false` is now present.

7. `test_reconcile_skips_user_defined_agent` — provision a non-well-known agent name (e.g., "operator-custom-agent"), run reconciler against the global home, assert that agent's identity.toml is not touched (the reconciler only iterates `WELL_KNOWN_AGENTS`).

8. `test_reconcile_disabled_via_env` — set `disable_agent_provisioning = true`, run reconciler, assert no writes (matches `provision_well_known_agents()` behavior).

9. `test_reconcile_handles_malformed_identity` — write garbage to identity.toml, run reconciler, assert it logs warn and continues (no panic, no partial write).

10. `test_code_owned_sections_have_reconciler_coverage` — for each path in `CODE_OWNED_IDENTITY_SECTIONS`, assert at least one `WELL_KNOWN_AGENTS` entry's rendered identity (via `render_identity_content` with `test_settings_with_kg_roots`) has a non-None value at that path. Catches typos like adding `"skils.allowlist"` to the constant.

11. `test_no_code_owned_drift_outside_constant` — for each `WELL_KNOWN_AGENTS` entry's rendered identity, walk every dotted path at depth ≤ 2 (top-level tables and one level of nesting). For any path that is NOT one of the operator-owned roots (`name`, `emoji`, `reflection.*`, `kg.*`), assert the path appears in `CODE_OWNED_IDENTITY_SECTIONS` (or has a prefix that does, e.g., `skills.allowlist` covers `skills.allowlist` exactly; `tools.disabled` covers `tools.disabled` exactly; `context.summary` covers both `context.summary.inject` and `context.summary.max_tokens`). Catches the silent-regression case where someone adds `[security]` to a future spec but forgets to add it to the constant. Implementation: a small recursive helper that produces a flat list of dotted paths from a `toml::Value::Table`, then `paths.iter().filter(|p| !is_operator_owned(p)).for_each(|p| assert!(is_under_code_owned(p)))`.

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

1. Add `CODE_OWNED_IDENTITY_SECTIONS` constant in `well_known_agents.rs`
2. Add `get_path` / `set_path` helpers (private)
3. Add `reconcile_well_known_identity(home_dir, spec, settings)` function in `well_known_agents.rs`
4. Wire into the existing `agent_exists` branch of `provision_well_known_agents()`
5. Add the 11 unit tests (9 reconciliation tests + 2 constant-coverage invariant tests)
6. Add the integration test in `tests/eval/test_qa_review_run_gh_scope_validator.rs`
7. `cargo test -p mika-agent` and `cargo clippy` pass
8. `make deploy` → cat the identity files → confirm reconciliation took effect
9. Trigger a real webhook on a test issue and confirm `gh issue edit` succeeds
10. Compound doc in `docs/solutions/best-practices/identity-toml-drift-from-static-spec-2026-05-20.md`

## Compound notes

This is the **second** time identity-drift has bitten the autonomous loop:

1. mika#1041 — `disabled_skills` drift in DB-backed `skill_overrides` (fixed by reconciler in `seed_well_known_skill_overrides`)
2. mika#1220 (this ticket) — `[skills].allowlist` drift in on-disk identity.toml (this fix)

The compound pattern is: **any code-owned configuration of a long-lived per-agent artifact requires explicit reconciliation, not just first-creation seeding.** The `agent_exists` short-circuit is structurally hostile to evolving the static spec. Future identity-template changes (next year's #815-class restructure) will hit the same shape unless reconciler keeps pace.

The structural mechanism added in this PR (`CODE_OWNED_IDENTITY_SECTIONS` + the two coverage-invariant tests `test_code_owned_sections_have_reconciler_coverage` and `test_no_code_owned_drift_outside_constant`) closes the loop: any future spec change that adds a code-owned section either (a) gets added to the constant and reconciled, or (b) trips the negative-direction test at CI time. Operator-owned roots (`name`, `emoji`, `reflection.*`, `kg.*`) stay explicitly excluded — extending the operator-owned set is an explicit code change in the test helper, also visible at review time.

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
