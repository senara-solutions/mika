# Plan: Add mika-test Built-In Agent (#963)

**Type:** feat
**Issue:** mika#963
**Date:** 2026-05-20
**Base SHA:** `0c2bbcf9` (feat(agent): webhook_milestone_advance inline guard mika#1218)

## Summary

Add `mika-test` as the fifth well-known agent — a minimal, no-skills agent for testing/debugging the engine without skill-system noise. Single-file change in `well_known_agents.rs`.

## Motivation

The four existing well-known agents (mika-dev, mika-qa, mika-relay, mika-arch) are all skill-rich. There's no plain agent to compare against when debugging skill-vs-engine behaviors. mika-test fills this gap: a bare-bones agent that exercises the core engine loop with zero skills, no KG, and default LLM settings.

**Note on issue description counting:** The issue body says "five existing well-known agents (mika, mika-dev, mika-qa, mika-relay, mika-arch)" and refers to mika-test as the "6th member." In reality, `WELL_KNOWN_AGENTS` at base SHA has 4 entries (mika-dev, mika-qa, mika-relay, mika-arch) — the default `mika` agent is not a well-known agent and is not in the array. After this change, the array will have 5 entries. The issue body's count is cosmetically inaccurate but does not affect scope or implementation.

## Base SHA Pin

At `0c2bbcf9`, `crates/mika-agent/src/well_known_agents.rs` (2623 lines):

- **`WELL_KNOWN_AGENTS` (line 386):**
  ```rust
  pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_QA, &MIKA_RELAY, &MIKA_ARCH];
  ```

- **`WellKnownAgent` struct (lines 48–71):** 9 fields — `name`, `display_name`, `emoji`, `soul`, `disabled_skills`, `config_toml`, `identity_source`, `llm_overrides`. `#[non_exhaustive]`.

- **`CODE_OWNED_IDENTITY_SECTIONS` (lines 401–402):**
  ```rust
  pub const CODE_OWNED_IDENTITY_SECTIONS: &[&str] =
      &["skills.allowlist", "tools.disabled", "context.summary"];
  ```
  mika-test's identity declares `[skills].allowlist` and `[kg].enabled` — of these, only `skills.allowlist` is in `CODE_OWNED_IDENTITY_SECTIONS`. The `[kg]` section is operator-owned (see line 400: "Sections NOT listed here are preserved verbatim from the on-disk file (operator-owned: `name`, `emoji`, `[reflection]`, `[kg]`)"). No change to `CODE_OWNED_IDENTITY_SECTIONS` required.

- **Hardcoded count assertion (line 2187):**
  ```rust
  assert_eq!(WELL_KNOWN_AGENTS.len(), 4);
  ```
  Must be updated to `5`.

- **`DISPATCH_TRIGGER_ALLOWLIST` (line 104):** `["samidarko", "mika-platform-dev"]` — unchanged.
- **`INTRA_PLATFORM_DISPATCH_PEERS` (line 608):** `["mika-arch", "mika-dev", "mika-qa"]` — unchanged.

- **Array position semantics:** `provision_well_known_agents()` (line 658) iterates `WELL_KNOWN_AGENTS` sequentially. Position determines provisioning order only — no code indexes by position. `find_well_known_agent()` (line 611) uses `.iter().find()` by name. Position is cosmetic.

## Deliverables

### Phase 1: Add mika-test to well_known_agents.rs

All changes are in `crates/mika-agent/src/well_known_agents.rs`.

**Step 1: Add `MIKA_TEST_SOUL` constant** (after `MIKA_RELAY_CONFIG` at line ~1065)

```rust
const MIKA_TEST_SOUL: &str = r#"# Mika Test — Minimal Test Agent

## Role
You are Mika Test, a minimal test agent for engine validation and debugging.
You have no skills enabled. Your purpose is to exercise the core agent loop
(LLM calls, memory, tools) without skill-system interference.

## Communication style
- Respond directly and helpfully
- You are a plain conversational agent with no special workflows
"#;
```

**Step 2: Add `MIKA_TEST_IDENTITY` constant** (after the soul)

```rust
const MIKA_TEST_IDENTITY: &str = "\
name = \"Test\"\n\
emoji = \"🧪\"\n\
\n\
[kg]\n\
enabled = false\n\
\n\
[skills]\n\
allowlist = []\n";
```

Key design choices:
- `[skills].allowlist = []` — empty allowlist means all skills are denied at Phase -1 (`apply_identity_allowlist` evicts everything not in the list). This is the same mechanism used by mika-relay (with 1 skill), just with zero entries. `apply_identity_allowlist()` at `prompt.rs` calls `retain()` on the registry — an empty allowlist retains nothing.
- `[kg].enabled = false` — no extraction, no resolution, no corpus tracking. Eliminates KG noise for a test agent. Consistent with mika-dev and mika-qa topology (#800).
- No `[tools].disabled` — mika-test gets the full default tool set, unlike mika-arch which has `MIKA_ARCH_DISABLED_TOOLS`. For a test agent, having all tools available is desirable.
- No `[context.summary]` override — uses default (`inject = true`). This means `CODE_OWNED_IDENTITY_SECTIONS` reconciliation for `context.summary` will be a no-op (path not present in identity → `get_path` returns `None` → `continue`).

**Reconciliation behavior:** `reconcile_well_known_identity()` iterates `CODE_OWNED_IDENTITY_SECTIONS` (`skills.allowlist`, `tools.disabled`, `context.summary`). For mika-test:
- `skills.allowlist`: present in identity, will be reconciled (enforces empty array on disk).
- `tools.disabled`: absent in identity → `get_path` returns `None` → skipped.
- `context.summary`: absent in identity → `get_path` returns `None` → skipped.

**Step 3: Add `MIKA_TEST` static** (after `MIKA_DEV` at line 78, before `MIKA_QA` at line 145)

```rust
/// mika-test agent specification.
///
/// Minimal test agent with no skills for engine validation and debugging.
/// Uses identity-driven `[skills].allowlist = []` to deny all skills.
/// KG disabled. Default LLM provider/model from `Settings`.
pub static MIKA_TEST: WellKnownAgent = WellKnownAgent {
    name: "mika-test",
    display_name: "Test",
    emoji: "🧪",
    soul: MIKA_TEST_SOUL,
    disabled_skills: &[],
    config_toml: None,
    identity_source: Some(IdentitySource::Static(MIKA_TEST_IDENTITY)),
    llm_overrides: &[],
};
```

**Step 4: Add to `WELL_KNOWN_AGENTS` array** (line 386)

```rust
pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_TEST, &MIKA_QA, &MIKA_RELAY, &MIKA_ARCH];
```

Position after `MIKA_DEV` per issue spec. Position is cosmetic (find_well_known_agent uses name-based lookup, provisioning order has no semantic dependency).

**Step 5: Update tests**

5a. `test_find_well_known_agent_found` (line ~1140): Add assertions for mika-test:
```rust
assert!(find_well_known_agent("mika-test").is_some());
assert_eq!(find_well_known_agent("mika-test").unwrap().name, "mika-test");
```

5b. Add a dedicated test (following the `test_find_well_known_agent_mika_arch` pattern at line 1981):
```rust
#[test]
fn test_find_well_known_agent_mika_test() {
    let agent = find_well_known_agent("mika-test").unwrap();
    assert_eq!(agent.name, "mika-test");
    assert_eq!(agent.display_name, "Test");
    assert_eq!(agent.emoji, "🧪");
    assert!(agent.disabled_skills.is_empty());
    assert!(agent.config_toml.is_none());
    assert!(agent.llm_overrides.is_empty());
}
```

5c. Add identity parsing test (following `test_mika_arch_config_valid_toml` pattern at line 2013):
```rust
#[test]
fn test_mika_test_identity_valid_toml() {
    let identity: toml::Value =
        toml::from_str(MIKA_TEST_IDENTITY).expect("MIKA_TEST_IDENTITY should be valid TOML");
    assert_eq!(identity["name"].as_str(), Some("Test"));
    assert_eq!(identity["emoji"].as_str(), Some("🧪"));
    assert_eq!(identity["kg"]["enabled"].as_bool(), Some(false));
    let allowlist = identity["skills"]["allowlist"].as_array().unwrap();
    assert!(allowlist.is_empty(), "mika-test should have empty skill allowlist");
}
```

5d. **Update hardcoded count** at line 2187:
```rust
// Before:
assert_eq!(WELL_KNOWN_AGENTS.len(), 4);
// After:
assert_eq!(WELL_KNOWN_AGENTS.len(), 5);
```

## What does NOT change

- No new files outside `well_known_agents.rs`.
- No changes to `DISPATCH_TRIGGER_ALLOWLIST` (line 104) — mika-test does not trigger autonomous dispatch.
- No changes to `INTRA_PLATFORM_DISPATCH_PEERS` (line 608) — mika-test is not part of the dev/qa/arch coordination peer group.
- No changes to `CODE_OWNED_IDENTITY_SECTIONS` (line 401) — mika-test's identity uses `skills.allowlist` which is already in the set; the other two paths (`tools.disabled`, `context.summary`) are absent in the identity and skipped by the reconciler.
- No `config_toml` override — mika-test uses whatever LLM provider the operator has configured in `Settings`.
- No schema changes.

## Verification

Per issue ACs:
1. `MIKA_DEV_MODE=true` startup auto-provisions `~/.mika/agents/mika-test/` ✓ (inherits from `provision_well_known_agents` loop at line 658)
2. `mika ask --agent mika-test "hello"` returns a response ✓ (default LLM, no skills interference)
3. `mika skills --agent mika-test list` shows zero active skills ✓ (empty allowlist evicts all at Phase -1)
4. `mika kg status --agent mika-test` reports KG disabled ✓ (`[kg].enabled = false` → `KgAgentConfig::Disabled`)
5. `find_well_known_agent("mika-test")` returns `Some(&MIKA_TEST)` ✓ (covered by test 5b)

## Risk assessment

**Low risk.** This is additive — a new array entry and constants. No existing behavior changes. The agent follows established patterns (static identity, no config override, no LLM overrides). The empty allowlist pattern is well-tested (mika-relay has a 1-element allowlist; `apply_identity_allowlist` handles empty the same way — `retain()` with empty set retains nothing).
