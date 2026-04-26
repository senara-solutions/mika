---
title: "mika-arch memory-write tools denied by overly broad denylist"
date: 2026-04-26
category: logic-errors
module: well-known-agents
problem_type: logic_error
component: tooling
symptoms:
  - "mika-arch cannot persist cross-session context — resets to baseline every session"
  - "mika-arch explicitly reports 'I don't have update_core_memory or store_fact in my available tools'"
  - "Operator must manually write to core_memory/facts tables as a proxy for mika-arch"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - mika-arch
  - denylist
  - tool-visibility
  - memory-writes
  - well-known-agents
  - identity-toml
  - orthogonality
---

# mika-arch memory-write tools denied by overly broad denylist

## Problem

mika-arch — the "Principal-Engineer-class advisory reviewer" agent — could not persist any self-state across sessions. She could not accumulate cross-ticket pattern recognition, prior-decision recall, or commitment tracking, because the tool denylist (`MIKA_ARCH_DISABLED_TOOLS`) denied `update_core_memory`, `store_fact`, and `update_fact` alongside platform-mutation tools like `pr_merge_with_gate` and `run_shell`.

## Symptoms

- mika-arch explicitly reported during a grooming session: *"I don't have `update_core_memory` or `store_fact` in my available tools for this session."*
- The operator had to manually write to `core_memory` and `facts` tables via direct DB access as a proxy — a capability the agent should have had natively.
- Every session started from baseline, with no recall of prior architectural decisions or patterns surfaced in previous reviews.

## What Didn't Work

- The denylist was originally assembled (#811 / PR #813) by enumerating all mutational tools without distinguishing *what gets mutated*. Memory writes were bundled with PR merge, shell exec, skill mutations, and other platform-mutation tools because they were all "writes."

## Solution

Drop three names from `MIKA_ARCH_DISABLED_TOOLS` in `crates/mika-agent/src/well_known_agents.rs`:

```rust
// Before: memory writes were in the denylist alongside platform mutations
pub const MIKA_ARCH_DISABLED_TOOLS: &[&str] = &[
    // Memory mutations           <-- removed
    "update_core_memory",         <-- removed
    "store_fact",                 <-- removed
    "update_fact",                <-- removed
    // Skill mutations (kept)
    "create_skill",
    // ... all other platform-mutation tools (kept)
];

// After: memory writes are in "Notably allowed" alongside send_message
pub const MIKA_ARCH_DISABLED_TOOLS: &[&str] = &[
    // Skill mutations (kept)
    "create_skill",
    // ... all other platform-mutation tools (kept)
];
```

Updated the doc comment to document memory writes as "Notably allowed" with citation to `docs/architecture/review-guide.md` § Orthogonality. Added regression test `test_mika_arch_disabled_tools_excludes_agent_self_state` and flipped existing test assertion.

**Deploy note:** Existing `~/.mika/agents/mika-arch/identity.toml` files must be manually edited (the provisioning path's `agent_exists` short-circuit prevents re-rendering). Remove the three tool names from `[tools].disabled` and restart mika-server.

## Why This Works

The denylist's design intent is the **read-only architect contract** — no commits, merges, shell exec, or code generation. Memory writes were incorrectly categorized as platform mutations. They actually mutate the agent's own self-state (5 core memory blocks + 4 facts categories, all scoped to `agent_id = 'mika-arch'` in SQLite). This is persistence, not a side-effect.

The architectural principle: **deny by what gets mutated, not by whether something is mutated.** Agent self-state writes are constitutive of being an agent (same reasoning as `send_message` being allowed). Platform-state writes are side-effects that the read-only contract correctly blocks.

## Prevention

- **Principle documented in `docs/architecture/review-guide.md` § Orthogonality** (commit `2bba6223`): future denylist changes must classify tools by mutation target (agent self-state vs platform state), not by whether they write at all.
- **Regression test** `test_mika_arch_disabled_tools_excludes_agent_self_state` uses a local array of agent-self-state tool names. Adding a future self-state tool requires a one-line append to the array — prevents silent re-bundling.
- **Defense in depth:** The test guards at the const level; `test_mika_arch_identity_has_tools_disabled_block` guards at the rendered-toml level with `assert!(!toml.contains("\"update_core_memory\""))`.

## Related Issues

- senara-solutions/mika#818 — this fix
- senara-solutions/mika#811 / PR #813 — introduced `MIKA_ARCH_DISABLED_TOOLS` with the original over-broad bundle
- `docs/architecture/review-guide.md` § Orthogonality — the principle that prevents this bundling mistake from recurring
- `docs/solutions/architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md` — companion pattern doc from the same PR that introduced the denylist
