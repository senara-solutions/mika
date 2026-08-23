# Plan — fix(agent-core): boot-time assert env-tier consistency for family agents

**Status:** DRAFT
**Date:** 2026-08-23
**Ticket:** mika#1962
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Substrate reliability hygiene — env-drift silent policy inversion prevention
**Cross-refs:** mika#1783 (founding fix — PR#1965 OPEN), correctness F2, adversarial F3

## Why

mika#1783 (PR#1965 OPEN) threads `ToolContext.tier` via `AgentTier::from_env()` at every ctx construction. Correctness reviewer F2 (MEDIUM) and adversarial reviewer F3 (MEDIUM) both flagged the same failure mode:

> A container bootstrapped as `family` (persona/soul scrubbed, allowlist narrowed) whose `MIKA_AGENT_TIER` env var goes missing on restart silently downgrades to `Default` at runtime. The `dispatch_substrate_diagnostic` fold-back path then leaks operator-shaped diagnostics through what looks like an operator tier — but the persona, allowlist, and provisioning state still say "family."

**Class:** silent policy inversion — env-drift becomes silent-persona-drift. Real vectors: K8s ConfigMap edit, Helm value change, systemd drop-in change, `docker exec` into a running container, manual `mika-spirit` restart from a shell missing the env var. mika-cloud has form here (per adversarial reviewer's specific callout).

**Verified against current `main` state:**
- `AgentTier` enum + `AgentTier::from_env()` are defined in `crates/mika-common/src/home.rs:13-45`.
- `MIKA_AGENT_TIER` env var recognized values: `"default"` (unset/empty/literal → operator persona), `"family"` (case-insensitive). Unknown values fall through to `Default` with a WARN log.
- `FAMILY_AGENT_SKILL_ALLOWLIST` and `FAMILY_SOUL` — **verified NOT in current main** (`grep -rn 'FAMILY_AGENT_SKILL_ALLOWLIST\|FAMILY_SOUL' crates/` returns zero hits). These constants + `dispatch_substrate_diagnostic` + `ToolContext.tier` field are all defined in **PR#1965** (`fix/1783/...`), not merged yet.

**Priority (from ticket):** p2-normal. Detection: no telemetry today; no alarm; the leak surfaces only via user complaint (the exact shape of mika#1783's founding incident).

## What

Three coordinated changes (Options A + B from ticket; C deferred). Option A fails startup fast on env-drift; Option B guarantees running-agent consistency by caching tier at init. Composed together, they close the failure class from both ends.

### 1. Option A — Boot-time assertion in mika-spirit startup (`family_provisioning_consistency_check`)

**File:** `crates/mika-agent/src/server/mod.rs` (in the agent-init path called at startup for each agent in `~/.mika/agents/`).

**Change shape:** for every agent that shows evidence of family-tier provisioning, assert `AgentTier::from_env() == AgentTier::Family`. On mismatch: hard-fail startup with an actionable error naming the agent, the detected provisioning state, and the missing env var.

**Detection criteria for "family-provisioned":**
- `soul.md` starts with `FAMILY_SOUL` sentinel marker (a `<!-- MIKA_FAMILY_SOUL_MARKER -->` line inserted at bootstrap time by `write_family_soul_if_missing`).
- OR `identity.toml`'s `[skills].allowlist` matches `FAMILY_AGENT_SKILL_ALLOWLIST` exactly.

**Structural implementation:**

```rust
// crates/mika-agent/src/server/mod.rs, called inside init_all_agents()
fn assert_family_tier_env_consistency(
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

**Ordering:** runs AFTER `Settings::load` (so env is fully loaded) and BEFORE `run_server` (so the assertion fails startup, not runtime). Requires the two helpers `soul_starts_with_family_marker` and `identity_allowlist_matches_family` — both simple file reads with fail-loud error propagation on IO errors (do NOT silently skip an agent whose files can't be read — that could hide a genuine drift; instead, propagate the error, which is what `anyhow::Result` does).

**FAMILY_SOUL marker helper (`crates/mika-common/src/home.rs`):** requires PR#1965 to have added a `FAMILY_SOUL_MARKER` const (a string like `"<!-- MIKA_FAMILY_SOUL_MARKER -->"`). If PR#1965 does NOT ship the marker, this ticket needs to add it — coordinate with Vincent to include in PR#1965 or add here as a companion change.

### 2. Option B — Cache tier on `AgentState` (defense in depth)

**Files:** `crates/mika-agent/src/server/state.rs` (or wherever `AgentState` is defined), + all 4 production `ToolContext` construction sites in `crates/mika-agent/src/agent_loop/mod.rs` (line 3007, line 4536) + `crates/mika-agent/src/server/investigate.rs`.

**Change shape:** add `tier: AgentTier` field to `AgentState`. Populate once at `init_agent` (or wherever `AgentState` is constructed) by calling `AgentTier::from_env()`. Change all 4 `ToolContext` construction sites from `tier: AgentTier::from_env()` to `tier: agent_state.tier`.

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

**Unit test (`crates/mika-agent/src/agent_state.rs` `#[cfg(test)] mod tests`):**

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

## Dependency on PR#1965 (mika#1783)

PR#1965 is OPEN and closes mika#1783. This plan depends on:
- `FAMILY_AGENT_SKILL_ALLOWLIST` const (PR#1965 §2)
- `FAMILY_SOUL` template (PR#1965 §1)
- `ToolContext.tier` field (PR#1965 core change)
- 4 production ctx-construction sites (PR#1965 threading changes)
- `dispatch_substrate_diagnostic` fold-back path (PR#1965 gate)

**Path A (recommended):** ship this ticket AFTER PR#1965 merges. All references become concrete against post-merge main.

**Path B (companion branch):** rebase this ticket onto `fix/1783/agent-doctrine-l-tre-scell-demande-de-la`, add `> **Companion PR:** #1965` callout to issue body, re-run /mika-groom-ticket. Requires operator consent.

Plan commits to **Path A**. Implementation gated on PR#1965 merge; the plan itself is committable now.

## Acceptance Criteria (verbatim from ticket)

- [ ] `mika-spirit` startup asserts env-tier consistency for family-provisioned agents; mismatch fails fast with actionable error. **→ Satisfied by § 1.**
- [ ] `AgentState.tier: AgentTier` field populated once at init time; `ToolContext.tier` reads from `AgentState.tier` at all 4 production construction sites (not `AgentTier::from_env()`). **→ Satisfied by § 2.**
- [ ] Unit test simulates env-drift-mid-runtime → cached tier does not change. **→ Satisfied by § 4 unit test.**
- [ ] Integration test: startup with family-provisioned agent + unset env → mika-spirit refuses to start with named-agent error. **→ Satisfied by § 4 integration test.**
- [ ] Documented in `crates/mika-agent/CLAUDE.md` § Skills System (near `FAMILY_AGENT_SKILL_ALLOWLIST` reference) + workspace `CLAUDE.md` § MIKA_AGENT_TIER env-var section. **→ Satisfied by § 5.**

## Definition of Done

- [ ] `crates/mika-agent/src/server/mod.rs`: `assert_family_tier_env_consistency()` implemented + called at init.
- [ ] `crates/mika-agent/src/agent_state.rs` (or wherever `AgentState` is defined): `tier: AgentTier` field added; init sets it once.
- [ ] All 4 `ToolContext` construction sites in `agent_loop/mod.rs` + `server/investigate.rs` thread `tier` from `AgentState`, not `AgentTier::from_env()`.
- [ ] `crates/mika-common/src/home.rs`: `soul_starts_with_family_marker()` + `identity_allowlist_matches_family()` helpers (+ `FAMILY_SOUL_MARKER` const if not shipped by PR#1965).
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
