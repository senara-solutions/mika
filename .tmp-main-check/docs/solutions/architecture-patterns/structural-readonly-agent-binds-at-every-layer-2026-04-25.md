---
title: "Structural read-only agent invariant must bind at every layer it manifests"
date: 2026-04-25
category: architecture-patterns
module: well-known-agents, skills, tools, prompt, agent-loop
problem_type: best_practice
component: agent-engine
severity: high
applies_when:
  - Adding a new well-known agent that is advertised as scoped, read-only, or advisory
  - Designing identity-driven enforcement (allowlists / denylists) for any agent invariant
  - Auditing whether a "read-only" claim actually holds at runtime
tags: [read-only-agent, identity-allowlist, structural-enforcement, soc, defense-in-depth, mika-arch, well-known-agent]
---

# Structural read-only agent invariant must bind at every layer it manifests

## Context

#811 introduced `mika-arch` as a fourth well-known agent — the "read-only architect" that reviews plans before code is written. The plan (D2) committed to making the read-only-ness *structural* via an identity-driven `[skills].allowlist` in `identity.toml`, replacing the existing `skill_overrides` DB-row pattern for well-known agents.

The first implementation landed the skill allowlist correctly. `/ce:review` afterward surfaced that the read-only invariant was still violated: mika-arch could call `pr_merge_with_gate`, `update_core_memory`, `update_skill`, `set_config`, `delegate_task`, and so on. None of those are skills — they're built-in tools registered in `tools::default_tools()` and shared across every agent via `Arc<ToolRegistry>`.

Three independent reviewers (adversarial, security, agent-native) converged on the same finding. The skill allowlist filtered *which skills* run; the tool layer was untouched.

## Problem

A "read-only agent" is two architectural promises:

1. The agent only runs skills authorized for its role.
2. The agent only calls tools that have no mutational effect on platform state.

These are different layers. The skill registry (`SkillRegistry`) and the tool registry (`ToolRegistry`) are independent in mika's architecture — built at different times, scoped differently (`Arc<ToolRegistry>` is shared across all agents at server startup; `SkillRegistry` is per-agent). Identity-driven enforcement that only binds to one layer leaves the other free.

The failure mode is silent. `mika-arch` would boot, run its review skills correctly, and *also* be one prompt-injection or model-drift event away from calling `pr_merge_with_gate`. No log line, no audit trail, no test catches it — the read-only invariant is enforced only by the system prompt.

The team had already learned this lesson once (`docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`: *"read-only enforcement fights a trained gradient — must bind structurally, not via prompt"*). Yet the mika-arch v1 plan re-instantiated the gap because the structural enforcement stopped at one layer.

## Solution

For any agent invariant that spans multiple architectural layers, enforcement must bind at every layer it manifests. Concretely for mika-arch:

**Layer 1: Skill registry.** `[skills].allowlist` in `identity.toml`, applied as Phase -1 in `SkillRegistry::apply_overrides()`. Evicts non-allowlisted skills from the registry before DB overrides.

**Layer 2: Tool registry.** `[tools].disabled` in `identity.toml`, applied via `agent::apply_agent_tool_visibility()` at LLM-tool-array assembly inside `inject_skills_and_resolve_tools()`. Filters denied tools from the array sent to the model. Named hook so future allowlist migration reuses the same call site.

**Layer 3: Identity loader.** `prompt::load_identity()` distinguishes well-known from user-defined agents on parse failure. Well-known agents get a *fail-closed* `Identity` (sentinel allowlist that matches nothing, full mutational denylist) so a malformed `identity.toml` neuters the agent until the operator fixes it. User agents fall back to defaults.

**Layer 4: Provision-time configuration.** Computed identities (mika-arch's `[kg].docs_roots`) are rendered from `Settings` at provision time so absolute paths reach disk; relative paths are rejected at render time, not at runtime.

The pattern: **for every layer where the invariant manifests, enforce structurally and fail loud on misconfiguration.**

## Why this works

- **Each layer's enforcement is independent.** Skill eviction doesn't depend on tool filtering or vice versa. Either layer can be regression-tested in isolation. A reviewer reading the code at any layer can verify the invariant locally.
- **Defense in depth.** A bug at one layer doesn't break the invariant — the others still bind. Concretely: even if `apply_identity_allowlist` is reordered or skipped, the tool denylist still prevents `pr_merge_with_gate`.
- **The model never sees disabled tools.** Filtering at LLM-tool-array assembly (not at dispatch refusal) means the tool description never enters the model's context. No prompt-injection surface, no token cost, no failure-mode noise from the model attempting and being refused.
- **Fail-closed > fail-open for security-critical agents.** A malformed identity is more likely than not to be operator error, not adversarial input — but the cost of fail-open (full bundled skill set + all built-in tools) is much larger than the cost of fail-closed (operator sees the error, fixes the file, agent boots correctly next restart). The discrimination happens once, in the loader, so callers don't need to repeat it.
- **Provision-time over runtime for path-shaped config.** Relative `docs_roots` work in tests where CWD is the repo root and silently break under `OpenRC`/`systemd` where CWD=`/`. Computing absolute paths at provision time from `Settings` (driven by `MIKA_KG_DOCS_ROOTS` env) means production deployments hit the same code path tests do, and the failure mode is a loud `error!` log naming the env var, not a silent "all paths unresolvable" downstream.

## When to use

Any time an agent (well-known or user-defined) is advertised as scoped, read-only, advisory, or otherwise restricted along an axis other than "what skills it runs."

Examples beyond mika-arch:
- An "audit-only" agent that should not write to `audit_events` or any other persistent log.
- A "free-tier" agent that should not call billable LLM provider tools.
- A "sandbox" agent that should not call exec/http handlers.

For each, ask: *what layers does the invariant cross?* If the answer is more than skills, the skill allowlist alone is insufficient.

The cluster of layers that typically need binding for any constrained agent in mika:

| Layer | Enforcement mechanism |
|---|---|
| Skills | `[skills].allowlist` or `disabled_skills` denylist; Phase -1 eviction |
| Built-in tools | `[tools].disabled` (or future `[tools].allowlist`); filter at `inject_skills_and_resolve_tools` |
| MCP tools | Currently unfiltered; would extend the same hook to cover them |
| Configuration paths | Provision-time computation from `Settings`; absolute paths only |
| Malformed configuration | Fail-closed at `load_identity` for well-known agents |

## Alternatives considered

**Refuse-at-dispatch** (model sees the tool, engine refuses the call): Rejected. Higher token cost (description in context every turn), prompt-injection surface remains, fails the "model never sees the tool" criterion. Worse than not offering the tool at all.

**Per-agent ToolRegistry rebuild at startup**: Rejected. Forks the shared `Arc<ToolRegistry>` per agent, doubling the surface where "what tools does agent X have?" must stay consistent. Today there's one source of truth filtered N ways; this would create N sources of truth that must stay in sync. Worse for archaeology, worse for the eventual mika-skills migration.

**Hardcoded denylist in Rust source rather than `[tools].disabled` in identity.toml**: Rejected for symmetry. `[skills].allowlist` lives in identity.toml; `[tools].disabled` should follow the same pattern so operators have one place to edit and one mental model. The Rust constant (`MIKA_ARCH_DISABLED_TOOLS`) is the *seed* baked into identity at provision time, not the runtime source of truth.

**Empty `[tools].allowlist` instead of denylist**: Architecturally cleaner (identity says what an agent has, not what it lacks). Deferred to a follow-up that migrates well-known agents from denylist to allowlist for tools, alongside the same migration for skills (D2 follow-up). Shipping denylist now keeps the diff bounded and matches the existing `disabled_skills` pattern on mika-dev/qa/relay.

## See also

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the prior compound that motivated the structural-binding requirement
- `docs/solutions/architecture-patterns/work-item-write-tools-orchestrator-restriction.md` — precedent for filtering tools at registration based on agent role
- `docs/solutions/best-practices/kg-per-agent-docs-root-config-isolation-2026-04-24.md` — "hard-error on explicit missing paths" convention for config-driven path resolution
- `crates/mika-agent/src/agent.rs::apply_agent_tool_visibility` — the named filter hook
- `crates/mika-agent/src/well_known_agents.rs::MIKA_ARCH_DISABLED_TOOLS` — the canonical denylist
- `crates/mika-agent/src/well_known_agents.rs::test_well_known_allowlist_excludes_write_capable_skills` — invariant test that protects the silent-reorder regression
- `crates/mika-agent/src/prompt.rs::parse_identity_or_fail_closed` — the well-known-vs-user-defined discrimination
