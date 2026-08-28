---
module: agent-core
tags: [structural-invariant, grep-gate, definition-of-done, gate-erosion, exemption-set, callsite-audit]
problem_type: workflow-discipline
category: best-practices
related_issues: [mika#1962]
---

# Write a structural gate's exemption set by role, not as a file whitelist

## Problem

A grep-based structural gate needs an exemption set — the callsites that are *allowed* to do the thing the gate forbids. The obvious way to write it is as a list of files or lines. That wording ages badly against the gate's own feature, and it fails in the most damaging direction: **it fires on correct code.**

Founding incident, mika#1962. The Definition of Done carried:

> `grep -rn 'AgentTier::from_env()' crates/mika-agent/` returns zero hits outside the single `init_agent` callsite.

The intent was sound — the per-turn consumers must read a cached tier, not the environment. The wording needed correcting **twice inside the same ticket**:

1. The ticket itself adds a boot-time guard whose whole job is to compare the env against on-disk state. That guard *must* read the env. The gate, as written, failed on the correct implementation before a single line of consumer code drifted.
2. Review then surfaced a second legitimate reader (the lazy-resolve guard callsite) and a third (an error-message coherence check). Two more amendments, same ticket.

A gate that fires on correct code gets disabled the first time it fires. That is the real cost: not the amendments, but that the third false positive is where someone deletes the gate, and the class it was protecting goes unwatched from then on.

## Pattern

State the exemption by **role in the design**, not by location:

```
# Ages badly — needs an amendment every time the surface legitimately grows:
no hits outside `crates/mika-agent/src/server/mod.rs:514`

# Survives — the reader can adjudicate a new callsite without editing the gate:
No production read outside the tier-resolution surface — the init read that
populates the cache, and the guards whose job is to compare env against disk.
Any read in the per-turn consumer paths (agent_loop/, teams/, task_engine/,
server/investigate.rs) is a regression.
```

The role form does two things a file list cannot:

- **It tells a reader why**, so a genuinely new legitimate reader is adjudicable on the spot instead of looking like a violation.
- **It names the regression class positively** — "a per-turn consumer read the env" — so the gate keeps meaning something after refactors move files around.

## How to tell which form you have

Ask: *if my feature grows one more legitimate callsite, does the gate need editing?*

- **Yes** → it is a location list. It will fire on correct code, and the amendment will land under time pressure or not at all.
- **No** → it is a role rule.

A second tell: if you cannot state the exemption without file paths, the exemption set may not have a coherent role behind it — which is itself worth knowing before you ship the gate.

## Applicability and boundaries

This is about the **exemption** half of a gate. The forbidden half is often correctly a literal token list — see `lecture-seule-structural-gate-2026-08-21.md`, where `FORBIDDEN_TOKENS` names concrete write-authority calls. A token denylist is stable because the tokens *are* the concept; a location allowlist is unstable because locations are an artifact of where the code currently sits.

Composes with `feedback_structural_gate_audit_grep_all_callsites` (audit every callsite when adding a gate): that memory says *find them all*, this one says *write down why each allowed one is allowed*. The audit without the reasoning produces exactly the file whitelist that erodes.
