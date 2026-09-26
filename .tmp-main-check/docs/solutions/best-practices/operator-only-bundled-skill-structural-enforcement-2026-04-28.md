---
title: "Operator-only bundled skill: two-layer structural enforcement pattern"
date: 2026-04-28
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a bundled skill that must only be invoked by human operators
  - Preventing autonomous agents from triggering a skill via webhook events
  - Enforcing operator-only access without adding new schema fields to skill.toml
tags:
  - skills
  - operator-only
  - dev-groom
  - webhook-guard
  - well-known-agents
  - defense-in-depth
  - bundled-skills
---

# Operator-only bundled skill: two-layer structural enforcement pattern

## Context

The `dev-groom` skill (#845) encodes the two-pass grooming flow as a bundled skill. It must be invoked only by human operators (via `mika ask --agent mika "groom <ticket>"` or the `/mika-groom-ticket` slash command) and never by autonomous agents responding to webhook events. The design considered three enforcement layers but adopted two, rejecting the third on YAGNI grounds.

## Guidance

Enforce operator-only access structurally at two layers:

### Layer 1 — `disabled_skills` in `well_known_agents.rs` (primary)

Add the skill name to the `disabled_skills` array for every autonomous agent:

```rust
// crates/mika-agent/src/well_known_agents.rs
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    disabled_skills: &[
        // ... existing entries ...
        "dev-groom", // operator-only (#845)
    ],
    // ...
};
```

Add to `MIKA_DEV`, `MIKA_QA`, and `MIKA_RELAY`. The operator-facing `mika` agent and `mika-arch` retain the skill (they have no `disabled_skills` exclusion for it).

Update the `allowed_overlap` list in `test_well_known_agent_specs_dev_qa_no_overlap` to include the new skill — it's legitimately disabled for both dev and qa.

### Layer 3 — `WEBHOOK_SKILL_DENYLIST` in `github.rs` (defense-in-depth)

Add a gateway-side guard that rejects webhook events carrying the skill name as a label:

```rust
// crates/mika-gateway/src/github.rs
const WEBHOOK_SKILL_DENYLIST: &[&str] = &["dev-groom"];

fn is_webhook_denylisted_skill(event_type: &str, action: Option<&str>, label_name: Option<&str>) -> bool {
    if event_type == "issues" && action == Some("labeled") {
        if let Some(label) = label_name {
            let label_lower = label.to_lowercase();
            return WEBHOOK_SKILL_DENYLIST.iter().any(|skill| label_lower.contains(skill));
        }
    }
    false
}
```

**Critical: scope the check to structured fields only.** The initial implementation checked the full `format_event_text` output (which includes issue body, PR description, comments) and produced false positives — any event where a user mentioned "dev-groom" in free text was silently dropped. The fix scopes the check to the `label.name` field for `issues.labeled` events only.

### Layer 2 — `operator_only` skill.toml flag (rejected per YAGNI)

A new `operator_only = true` field in `skill.toml` was considered but rejected: the flag would have exactly one user (`dev-groom`) and semantically duplicates Layer 1's allowlist check. Re-evaluate when a second operator-only skill exists or when a new dispatch entry point bypasses Layers 1+3.

## Why This Matters

Without structural enforcement, a future webhook event or agent configuration change could accidentally trigger an operator-only skill in autonomous mode. Documentational enforcement ("don't invoke this skill") is insufficient — LLM agents don't reliably honor prose instructions under all conditions. Two structural layers defend against different regression modes: Layer 1 catches agent misconfiguration; Layer 3 catches webhook routing additions that bypass agent-identity classification.

## When to Apply

- Adding any bundled skill that should be operator-triggered only
- The skill uses existing builtin tools (no new tool registration needed)
- The skill is a prompt-only or thin-handler skill (not a long-running exec handler)

## Examples

The `dev-groom` skill (#845) is the canonical example:

```
skills/bundled/dev-groom/
├── skill.toml            # always_on=false, keyword triggers
├── system_prompt.md      # 6-phase grooming flow
├── tools.json            # [] — uses builtin tools only
└── handlers/run.sh       # Thin convenience handler (branch slug, plan naming)
```

The skill is activated by keyword match ("groom", "groom ticket") for the operator-facing `mika` agent. Autonomous agents (`mika-dev`, `mika-qa`, `mika-relay`) have it in `disabled_skills` and cannot activate it.

## Related

- [mika#845](https://github.com/senara-solutions/mika/issues/845) — origin ticket
- [mika#841](https://github.com/senara-solutions/mika/issues/841) — `ready` label positive-consent gate (analogous gateway guard pattern)
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — the plan-on-branch contract that dev-groom enforces
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — architect disposition paraphrase tolerance
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — structural guard pattern reference
