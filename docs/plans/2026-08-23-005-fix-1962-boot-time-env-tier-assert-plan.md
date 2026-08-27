# Plan — fix(agent-core): boot-time assert env-tier consistency for family agents

**Status:** IMPLEMENTATION-READY (Path A gate lifted 2026-08-27)
**Date:** 2026-08-23 (deepened 2026-08-27)
**Ticket:** mika#1962
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Substrate reliability hygiene — env-drift silent policy inversion prevention
**Cross-refs:** mika#1783 (founding fix — PR#1965 **MERGED** 2026-08-24 @ `82245af6`), correctness F2, adversarial F3

## Why

mika#1783 (PR#1965 OPEN) threads `ToolContext.tier` via `AgentTier::from_env()` at every ctx construction. Correctness reviewer F2 (MEDIUM) and adversarial reviewer F3 (MEDIUM) both flagged the same failure mode:

> A container bootstrapped as `family` (persona/soul scrubbed, allowlist narrowed) whose `MIKA_AGENT_TIER` env var goes missing on restart silently downgrades to `Default` at runtime. The `dispatch_substrate_diagnostic` fold-back path then leaks operator-shaped diagnostics through what looks like an operator tier — but the persona, allowlist, and provisioning state still say "family."

**Class:** silent policy inversion — env-drift becomes silent-persona-drift. Real vectors: K8s ConfigMap edit, Helm value change, systemd drop-in change, `docker exec` into a running container, manual `mika-spirit` restart from a shell missing the env var. mika-cloud has form here (per adversarial reviewer's specific callout).

**Verified against current `main` state (re-verified 2026-08-27, post-PR#1965-merge):**
- `AgentTier` enum + `AgentTier::from_env()` are defined in `crates/mika-common/src/home.rs:13-46`.
- `MIKA_AGENT_TIER` env var recognized values: `"default"` (unset/empty/literal → operator persona), `"family"` (case-insensitive). Unknown values fall through to `Default` with a WARN log.
- `FAMILY_AGENT_SKILL_ALLOWLIST` — **now on main** at `crates/mika-common/src/home.rs:461`. `FAMILY_SOUL` — **now on main** at `crates/mika-common/src/home.rs:513`. `FAMILY_IDENTITY` at `home.rs:472`. All landed via PR#1965 (merged 2026-08-24 @ `82245af6`).
- The four production `ToolContext` tier-construction sites are on main: `crates/mika-agent/src/agent_loop/mod.rs:3349`, `:4273`, `:4837`, and `crates/mika-agent/src/server/investigate.rs:778` — each currently `tier: mika_common::home::AgentTier::from_env()`.
- `AgentState` is defined at `crates/mika-agent/src/server/state.rs:27` (**not** `crates/mika-agent/src/agent_state.rs` — that path does not exist; corrected 2026-08-27). It is constructed at `crates/mika-agent/src/server/mod.rs:566` (inside `init_agent`, declared at `:419`) and at `crates/mika-agent/src/server/mod.rs:1761` (test fixture).
- `FAMILY_SOUL_MARKER` — **confirmed absent from main.** PR#1965 did not ship a sentinel. Per this plan's own directive (§1), this ticket adds it as a companion change.

**Priority (from ticket):** p2-normal. Detection: no telemetry today; no alarm; the leak surfaces only via user complaint (the exact shape of mika#1783's founding incident).

## What

Three coordinated changes (Options A + B from ticket; C deferred). Option A fails startup fast on env-drift; Option B guarantees running-agent consistency by caching tier at init. Composed together, they close the failure class from both ends.

### 1. Option A — Boot-time assertion in mika-spirit startup (`family_provisioning_consistency_check`)

**File:** new module `crates/mika-agent/src/server/tier_guard.rs`, invoked from `run_server` (`crates/mika-agent/src/server/mod.rs:682`).

**Callsite decision (pinned 2026-08-27).** The original plan said "after `Settings::load`, before `run_server`" — i.e. in `crates/mika-agent/src/bin/mika-spirit.rs` main. **Superseded:** the check runs *inside* `run_server`, immediately after `home::migrate_to_multi_agent(global_home)?` (`server/mod.rs:691`) and before agent discovery/init. Two reasons, both load-bearing:

1. **Post-migration layout.** `migrate_to_multi_agent` is what establishes `~/.mika/agents/<name>/`. Scanning for family provisioning before it runs would read a pre-migration layout and miss agents entirely — a false-negative in the exact direction the check exists to prevent.
2. **No bypass surface.** `run_server` has exactly one production caller (`bin/mika-spirit.rs:71`) and zero test callers (tests use `test_app`, `server/mod.rs:102`). Putting the guard inside `run_server` means any future binary entry point inherits it, rather than each having to remember to call it. Placing it in `main` would make the guard opt-in per-binary — the structural-vs-prompt distinction from `feedback_prompt_enforcement_fragile` applied to callsites.

Settings are already loaded by the time `run_server` is entered (`mika-spirit.rs:50`), and `load_dotenv` has already run (`mika-spirit.rs:23`), so the env is fully materialized — the ordering constraint the original wording was protecting is satisfied.

**Change shape:** for every agent that shows evidence of family-tier provisioning, assert `AgentTier::from_env() == AgentTier::Family`. On mismatch: hard-fail startup with an actionable error naming the agent, the detected provisioning state, and the missing env var.

**Detection criteria for "family-provisioned" (two independent axes, OR-combined):**
- **Axis 1 — soul marker.** `soul.md` contains the `FAMILY_SOUL_MARKER` sentinel.
- **Axis 2 — identity allowlist.** `identity.toml`'s `[skills].allowlist` matches `FAMILY_AGENT_SKILL_ALLOWLIST` exactly (as a set).

**Marker design + backward-compat consequence (pinned 2026-08-27).** `FAMILY_SOUL_MARKER` does not exist on main; this ticket adds it to `crates/mika-common/src/home.rs` as `<!-- MIKA_FAMILY_SOUL_MARKER -->` and prepends it to the `FAMILY_SOUL` constant. Two consequences to hold explicitly:

- **It is not retroactive.** `write_default_if_missing` never rewrites an existing `soul.md` (contract preserved). Any family agent bootstrapped *before* this ticket ships has a marker-less `soul.md`, so **axis 1 returns false for every already-provisioned family agent**. Axis 2 (identity allowlist) is therefore the load-bearing detector for the installed base, and axis 1 only becomes load-bearing for agents bootstrapped after this ships. This is not a defect — it is why the plan carries two axes — but a single-axis implementation would silently fail to protect exactly the agents that already exist.
- **The marker is inert in the prompt.** `soul.md` is read into the system prompt, so the marker becomes prompt text. An HTML comment is the right shape: it carries no instruction the model would act on. It must also not trip the `family_soul_no_operator_name` invariant test (`home.rs:607`) — it carries no operator-identity token.

**Structural implementation:**

```rust
// crates/mika-agent/src/server/tier_guard.rs, called from run_server()
// after home::migrate_to_multi_agent(), before agent discovery.
pub fn assert_family_tier_env_consistency(
    home_dir: &Path,
    tier: AgentTier,
) -> Result<()> {
    let mut mismatched = Vec::new();
    for agent_name in mika_common::agent::list_agents(home_dir) {
        let agent_home = mika_common::home::resolve_agent_home(home_dir, &agent_name);
        let soul_path = agent_home.join("soul.md");
        let identity_path = agent_home.join("identity.toml");

        let is_family_provisioned =
            soul_starts_with_family_marker(&soul_path)?
                || identity_allowlist_matches_family(&identity_path)?;

        if is_family_provisioned && tier != AgentTier::Family {
            mismatched.push(agent_name);
        }
    }
    if !mismatched.is_empty() {
        anyhow::bail!(
            "family-tier provisioning drift detected for agents: {} — \
             on-disk state indicates family tier, but MIKA_AGENT_TIER={} \
             (expected 'family'). Set MIKA_AGENT_TIER=family in the process \
             environment BEFORE mika-spirit starts, or the being will silently \
             downgrade to Default at runtime (persona/allowlist/dispatch semantics \
             will diverge from the operator's intent). See mika#1783 for the \
             founding incident.",
            mismatched.join(", "),
            std::env::var("MIKA_AGENT_TIER").unwrap_or_else(|_| "<unset>".into()),
        );
    }
    Ok(())
}
```

**Ordering:** runs inside `run_server` after `migrate_to_multi_agent` and before agent discovery/init, so the assertion fails startup rather than surfacing at runtime. Requires the two helpers `soul_has_family_marker` and `identity_allowlist_matches_family` — both simple file reads.

**Missing-file semantics (pinned 2026-08-27).** The two failure modes are NOT symmetric and must not be collapsed:

- **File absent** (`soul.md` or `identity.toml` does not exist) → that axis reports `false`, no error. An agent directory mid-bootstrap legitimately has no `soul.md` yet; treating absence as fatal would make the guard refuse startup on a fresh install, which is a self-inflicted outage with no drift behind it.
- **File present but unreadable** (permissions, IO error, malformed TOML) → propagate via `anyhow::Result`, failing startup. Do NOT silently skip an agent whose files exist but can't be read — that is exactly where a genuine drift could hide.

**FAMILY_SOUL_MARKER const (`crates/mika-common/src/home.rs`):** PR#1965 merged without it (verified 2026-08-27). This ticket adds it, per the disposition this plan already pre-specified. Marker value `<!-- MIKA_FAMILY_SOUL_MARKER -->`, prepended to `FAMILY_SOUL`; detection is `contains`, not `starts_with`, so a hand-edit that adds a leading blank line or title does not defeat axis 1.

### 2. Option B — Cache tier on `AgentState` (defense in depth)

**Files (line numbers verified against main 2026-08-27):**
- `crates/mika-agent/src/server/state.rs:27` — `AgentState` definition (add the field).
- `crates/mika-agent/src/server/mod.rs:566` — `AgentState` construction inside `init_agent` (populate the field once).
- `crates/mika-agent/src/server/mod.rs:1761` — test-fixture `AgentState` construction (must also populate the field or the crate will not compile).
- `crates/mika-agent/src/agent_loop/mod.rs:3349`, `:4273`, `:4837` — three of the four `ToolContext` tier sites.
- `crates/mika-agent/src/server/investigate.rs:778` — the fourth.

**Change shape:** add `tier: AgentTier` field to `AgentState`. Populate once in `init_agent` by calling `AgentTier::from_env()`. Change all 4 `ToolContext` construction sites from `tier: mika_common::home::AgentTier::from_env()` to read the cached `AgentState.tier`.

**Reachability caveat (surfaced 2026-08-27).** The four sites must each be checked for whether an `AgentState` is actually in scope at that point. Where one is not, the tier has to be threaded in through the existing params struct rather than conjured — and a site that cannot reach `AgentState` without a signature change is a finding to record, not a reason to leave `from_env()` in place. `/ce:work` resolves this per-site against the real code; the AC ("all 4 production construction sites, not `AgentTier::from_env()`") is the contract regardless of how the threading lands.

```rust
pub struct AgentState {
    // ... existing fields ...
    /// Agent tier resolved ONCE at init time from `MIKA_AGENT_TIER` env var.
    /// A subsequent env-drift (K8s ConfigMap edit, systemd drop-in change,
    /// manual restart from a shell missing the env var) does NOT affect running
    /// agents — the tier is fixed at boot per container lifetime. Composes with
    /// the mika-spirit startup assertion (§1 above): §1 fails fast at boot on
    /// provisioning-vs-env mismatch; §2 guarantees consistency for running
    /// agents.
    pub tier: AgentTier,
}
```

**Ordering:** field populated at `init_agent` BEFORE first `run_agent()` call. All ctx-construction sites thread from `agent_state.tier`, never from `AgentTier::from_env()` (removes the `from_env()` call from the hot path entirely).

### 3. Option C — Boot-time capability drop (deferred to separate ticket)

**Deferred rationale:** Option C (drop `web-search` from family-tier allowlist when `brave_api_key` is missing) is a narrower, tool-specific gate that composes orthogonally with A+B but adds surface area. This ticket ships A+B; Option C is a separate follow-up when the mika-arch `web_search` interaction pattern warrants it — the ticket's founding incident (mika#1783 «Salut Vincent») was about the *fold-back diagnostic* path, not the tool-configuration path per se. File separate.

### 4. Tests

**Env-var test hygiene (pinned 2026-08-27).** `AgentTier::from_env()` reads process-global state. Rust runs tests multi-threaded within a binary, so any test that sets or unsets `MIKA_AGENT_TIER` races every other test that reads it — including the existing `home.rs:1116-1142` tier tests. Follow whatever serialization the existing `home.rs` tier tests already use (they mutate the same var); do not introduce a second, divergent mechanism. If they rely on being in a separate binary or on a mutex, match it.

**Unit test (`crates/mika-agent/src/server/state.rs` `#[cfg(test)] mod tests`):**

```rust
#[test]
fn agent_state_tier_survives_env_drift() {
    // Simulate env-drift-mid-runtime:
    // 1. Set MIKA_AGENT_TIER=family, create AgentState.
    // 2. Unset MIKA_AGENT_TIER (mid-runtime).
    // 3. Assert AgentState.tier == AgentTier::Family (cached).
    // 4. Assert AgentTier::from_env() == AgentTier::Default (env-drift confirmed).
}
```

**Integration test (`crates/mika-agent/tests/family_tier_env_consistency.rs` new):**

```rust
#[tokio::test]
async fn mika_spirit_refuses_start_when_family_provisioned_agent_missing_env() {
    // Set up a tempdir with an agent that has FAMILY_SOUL marker in soul.md
    // AND MIKA_AGENT_TIER unset.
    // Call the agent-init assertion helper directly (or spawn mika-spirit as
    // a subprocess and assert non-zero exit + stderr contains the error message).
    // Assert error contains the agent name + "family-tier provisioning drift".
}

#[tokio::test]
async fn mika_spirit_starts_clean_when_family_provisioned_agent_matches_env() {
    // Same tempdir setup + MIKA_AGENT_TIER=family.
    // Assertion helper returns Ok(()).
}
```

**Unit test extending `crates/mika-common/src/home.rs` tests:**

```rust
#[test]
fn soul_starts_with_family_marker_detects_marker() { ... }

#[test]
fn identity_allowlist_matches_family_detects_family_allowlist() { ... }
```

### 5. Documentation

**File:** `crates/mika-agent/CLAUDE.md` § Skills System (near `FAMILY_AGENT_SKILL_ALLOWLIST` reference) + workspace `CLAUDE.md` § MIKA_AGENT_TIER env-var section.

**Content:**
- Boot-time assertion behavior: mika-spirit refuses to start on family-vs-env mismatch. Fail-fast, actionable error.
- `AgentState.tier` cached-at-init contract: env-drift-during-runtime does NOT affect running agents.
- Deploy-time discipline: set `MIKA_AGENT_TIER=family` in service EnvironmentFile / K8s ConfigMap BEFORE first startup. Missing env at restart is a hard failure, not a silent downgrade.
- Diagnostic path: on assertion failure, error message names the affected agents + the missing env var. Operator remediation: set the env, restart.

## Dependency on PR#1965 (mika#1783) — RESOLVED

**Status 2026-08-27: gate LIFTED.** PR#1965 merged 2026-08-24T11:38:02Z as `82245af6` (base `main`, head `fix/1783/agent-doctrine-l-tre-scell-demande-de-la`). This branch was rebased onto `origin/main` (`5db56354`) on 2026-08-27, so every reference below is now concrete against the working tree. **Path A is satisfied — implementation proceeds.** Path B (companion branch) is moot and requires no operator consent.

The one carve-out: PR#1965 did NOT ship `FAMILY_SOUL_MARKER`. This plan pre-specified the disposition for that case ("add here as a companion change"), so no re-grooming round is needed — see §1.

PR#1965 supplied:
- `FAMILY_AGENT_SKILL_ALLOWLIST` const (PR#1965 §2)
- `FAMILY_SOUL` template (PR#1965 §1)
- `ToolContext.tier` field (PR#1965 core change)
- 4 production ctx-construction sites (PR#1965 threading changes)
- `dispatch_substrate_diagnostic` fold-back path (PR#1965 gate)

Plan committed to **Path A**; Path A has now happened. No residual dependency.

## Acceptance Criteria (verbatim from ticket)

- [ ] `mika-spirit` startup asserts env-tier consistency for family-provisioned agents; mismatch fails fast with actionable error. **→ Satisfied by § 1.**
- [ ] `AgentState.tier: AgentTier` field populated once at init time; `ToolContext.tier` reads from `AgentState.tier` at all 4 production construction sites (not `AgentTier::from_env()`). **→ Satisfied by § 2.**
- [ ] Unit test simulates env-drift-mid-runtime → cached tier does not change. **→ Satisfied by § 4 unit test.**
- [ ] Integration test: startup with family-provisioned agent + unset env → mika-spirit refuses to start with named-agent error. **→ Satisfied by § 4 integration test.**
- [ ] Documented in `crates/mika-agent/CLAUDE.md` § Skills System (near `FAMILY_AGENT_SKILL_ALLOWLIST` reference) + workspace `CLAUDE.md` § MIKA_AGENT_TIER env-var section. **→ Satisfied by § 5.**

## Definition of Done

- [ ] `crates/mika-agent/src/server/tier_guard.rs`: `assert_family_tier_env_consistency()` implemented; called from `run_server` after `migrate_to_multi_agent`, before agent discovery.
- [ ] `crates/mika-agent/src/server/state.rs`: `tier: AgentTier` field added to `AgentState`; `init_agent` (`server/mod.rs:566`) sets it once; the test fixture at `server/mod.rs:1761` populates it too.
- [ ] All 4 `ToolContext` construction sites (`agent_loop/mod.rs:3349`, `:4273`, `:4837`, `server/investigate.rs:778`) thread `tier` from the cached `AgentState.tier`, not `AgentTier::from_env()`.
- [ ] `crates/mika-common/src/home.rs`: `FAMILY_SOUL_MARKER` const added and prepended to `FAMILY_SOUL`; `soul_has_family_marker()` + `identity_allowlist_matches_family()` helpers.
- [ ] `grep -rn 'AgentTier::from_env()' crates/mika-agent/` returns zero hits outside the single `init_agent` callsite (structural proof the hot path no longer reads env — per `feedback_structural_gate_audit_grep_all_callsites`).
- [ ] Tests per § 4 (1 unit test on `AgentState`, 2 integration tests on assertion helper, 2 unit tests on marker helpers).
- [ ] Docs per § 5.
- [ ] `cargo test --workspace` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] PR body: (a) coordination note with PR#1965, (b) diff footprint verbatim, (c) manual verification recipe (temporarily unset `MIKA_AGENT_TIER` on the local family-provisioned agent, attempt `make deploy`, verify fails with named agent error).

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

Three inversions:

1. **§ 1 fires** — temporarily invert the check `is_family_provisioned && tier != AgentTier::Family` to `is_family_provisioned && tier == AgentTier::Family`; verify integration test `mika_spirit_refuses_start_when_family_provisioned_agent_missing_env` fails (assertion no longer catches the drift); restore.
2. **§ 2 cache** — temporarily change `tier: agent_state.tier` back to `tier: AgentTier::from_env()` at one ctx-construction site; verify unit test `agent_state_tier_survives_env_drift` still passes (it tests AgentState only) BUT add an assertion-in-test that reads a `ToolContext.tier` snapshot after env-drift and confirms it's now `Default` (proving the cache is what fixes this); restore.
3. **§ 4 marker detection** — temporarily hardcode `is_family_provisioned = false` unconditionally; verify integration tests fail (family agent no longer detected); restore.

Document in `todos/1962-injection-verification.md`.

## Out of scope

- **Option C: boot-time capability drop for tools with missing env deps** (`web-search` when `brave_api_key` unset). Composes orthogonally with A+B; file separate ticket keyed on the specific tools when the interaction shape warrants it.
- **Runtime tier-switching** — no support for dynamically changing `AgentTier` after boot. The whole point is that tier is fixed at boot per container lifetime. If an operator wants to switch an agent from family to operator, they restart with a different env — not a hot-swap.
- **Multi-agent multi-tier per container** — v1 assumes all agents in a single mika-spirit process share one tier (from the process env). Per-agent tier declaration in `identity.toml` would be a schema extension; deferred.
- **Auto-remediation** — no automatic env-setting on assertion failure. Fail-fast + operator-fix is the intended shape.
- **Non-family tier assertion** — this ticket only asserts family-provisioned agents match family env. Operator-provisioned agents don't have an equivalent detection sentinel (there's no OPERATOR_SOUL marker); asymmetric coverage is acceptable because operator is the *default* (missing env → operator persona = correct behavior).

## Risks and mitigations

- **Fail-fast on assertion failure blocks all agent startup** — a single misconfigured agent (e.g., mid-migration from operator to family) crashes the entire mika-spirit process. Mitigation: intentional — silent policy inversion is a worse failure mode than hard-crash-with-actionable-error. Operator sees the error message with the agent name and fixes the config.
- **Marker detection false-negative if soul.md is edited by hand** — an operator who hand-edits `soul.md` to remove the `FAMILY_SOUL_MARKER` comment (thinking it's cosmetic) breaks family detection. Mitigation: identity-allowlist match is a second detection axis — even without the soul marker, the family allowlist match fires. Both paths would need to be manually broken to hide family provisioning.
- **Marker detection false-positive on operator soul that happens to start with the marker** — impossible if `FAMILY_SOUL_MARKER` is unique-by-construction (e.g., includes the string `MIKA_FAMILY_SOUL_MARKER`).
- **Race between file read and startup assertion** — an operator deleting `soul.md` mid-assertion. Mitigation: the file read propagates its IO error via anyhow, mika-spirit fails startup with the IO error rather than silently skipping the agent. Same outcome: hard failure, not silent drift.
- **Cached tier means env-set post-boot has no effect** — an operator setting `MIKA_AGENT_TIER=family` in a running process's environment (via a debugger or /proc/PID/environ hack) sees no behavioral change. Mitigation: intentional per the ticket's core design (Option B). The failure mode being fixed is env-drift AFTER boot, not env-set AFTER boot. Deploy discipline: set the env BEFORE first startup.

## Related solutions

- mika#1783 / PR#1965 — founding fix; this ticket is hygiene.
- `feedback_prompt_enforcement_fragile` — env-based tier resolution is empirically fragile; structural boot-time gate is the correct shape.
- `feedback_structural_enforcement_layer_for_tool_requirements` — same pattern applied to tool requirements; this ticket applies it to agent-tier.
- `crates/mika-agent/src/skills/manifest.rs::apply_load_safety_check` — precedent shape (already drops skills whose handlers are missing — the same boot-time-capability-drop shape Option C reuses).

## Compounding potential

After merge:

- **Boot-time provisioning-vs-env consistency assertion pattern** (~60 lines): the general shape of detecting on-disk provisioning state + matching against process env + fail-fast on mismatch. Reusable for any future config that spans two authorities (on-disk template + runtime env). Compound doc naming this makes the pattern repeatable.
- **Cached-at-init state field vs env-read-at-use anti-pattern**: the specific decision to cache `AgentTier` on `AgentState` instead of re-reading at every ctx construction is a general rule — any config value that "must not change per container lifetime" should be cached at init. Compound doc documents the tradeoff (memory + init cost vs runtime drift resilience).
