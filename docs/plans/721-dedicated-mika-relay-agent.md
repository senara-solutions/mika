# Plan: Dedicated mika-relay Agent for Claude-Pilot Permission Decisions (#721)

## Problem

claude-pilot sends 50-150 permission events per session through `mika --agent mika-dev ask`. Each event is a simple allow/deny JSON classification, but mika-dev's full orchestration stack (skills, 30K+ token system prompt, mid-tier model) processes every one. This causes:

- **Cost:** mid-tier model for trivial JSON classification
- **Skill collision:** relay payloads contain keywords like "build" that match self-dev triggers
- **Context bloat:** full mika-dev system prompt sent for each relay call

## Solution

A dedicated `mika-relay` well-known agent with minimal footprint: only the `permission-policy` skill, a cheap model (haiku-4.5), and a focused soul prompt. All other skills disabled.

## Implementation Steps

### Step 1: Add MIKA_RELAY to well_known_agents.rs

Add a third `WellKnownAgent` static alongside `MIKA_DEV` and `MIKA_QA`:

```rust
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    name: "mika-relay",
    display_name: "Relay",
    emoji: "🔑",
    soul: MIKA_RELAY_SOUL,
    disabled_skills: &[
        // Disable everything except permission-policy
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "self-dev-sprint",
        "qa-review",
        "qa-review-build-callback",
        "skill-review",
        "claude-pilot",
        "build-mika",
        "deploy-mika",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
    ],
};
```

Add `MIKA_RELAY_SOUL` const with relay-focused personality (direct, minimal, permission-decision-only).

Add `&MIKA_RELAY` to `WELL_KNOWN_AGENTS` array.

**File:** `crates/mika-agent/src/well_known_agents.rs`

### Step 2: Add per-agent LLM model to WellKnownAgent spec

The `WellKnownAgent` struct currently has no model field. Add an optional `model` field:

```rust
pub struct WellKnownAgent {
    // ... existing fields
    /// Optional LLM model override (provider/model format).
    /// Written to the agent's config.toml on creation.
    pub model: Option<&'static str>,
}
```

For `MIKA_RELAY`, set `model: Some("anthropic/claude-haiku-4-5-20251001")`. For `MIKA_DEV` and `MIKA_QA`, set `model: None` (use default).

In `provision_well_known_agents()`, after writing `identity.toml` and `soul.md`, if `spec.model` is `Some(m)`, write a `config.toml` with the model override:

```toml
[llm]
model = "anthropic/claude-haiku-4-5-20251001"
```

**File:** `crates/mika-agent/src/well_known_agents.rs`

**Research needed:** Check how per-agent `config.toml` is loaded and whether `[llm] model` field is already supported. If not, the model can be set via the existing per-skill LLM override mechanism on `permission-policy`.

### Step 3: Update claude-pilot.json

Change the relay target from `mika-dev` to `mika-relay`:

```json
{
  "command": "mika",
  "args": ["--agent", "mika-relay", "ask"],
  "timeout": 120000
}
```

**File:** `.claude/claude-pilot.json`

### Step 4: Update tests

- Add `MIKA_RELAY` to existing well-known agent tests (find, provision, identity, soul, seed overrides)
- Test that `MIKA_RELAY.disabled_skills` covers all bundled skills except `permission-policy`
- Update the skill-overlap test to include `mika-relay`
- Verify the model field is written correctly to `config.toml`

**File:** `crates/mika-agent/src/well_known_agents.rs` (test module)

### Step 5: Update documentation

- Update `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` to include mika-relay
- Note in claude-pilot skill docs that relay now targets mika-relay

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/well_known_agents.rs` | Add MIKA_RELAY spec, model field, soul const, tests |
| `.claude/claude-pilot.json` | Point relay to mika-relay |

## Risks & Mitigations

- **Risk:** Missing a bundled skill in the disabled list → new skills added later could activate on relay turns.
  **Mitigation:** Test that disabled_skills covers all bundled skills except permission-policy. Use an allowlist approach in the test.

- **Risk:** Per-agent config.toml model override not supported by config loading.
  **Mitigation:** Research config loading first. Fallback: use skill_overrides DB for per-skill LLM override on permission-policy specifically for this agent.

## Out of Scope

- Updating `claude-pilot.json` in other repos (mika-cloud, mika-skills) — those are separate PRs
- Changes to the permission-policy skill itself
- Changes to the claude-pilot handler
