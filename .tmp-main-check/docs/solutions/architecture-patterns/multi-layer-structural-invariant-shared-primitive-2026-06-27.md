---
title: "Multi-layer structural invariants must share one primitive — and the four sub-rules for adding a layer"
date: 2026-06-27
category: architecture-patterns
module: agent-loop
problem_type: best_practice
component: skills
severity: medium
applies_when:
  - Adding a new enforcement layer for an invariant already checked elsewhere (build-time, load-time, per-turn)
  - The invariant is "a config token must resolve to a real X" (a tool name, a skill name, an agent id, an event class)
  - Two or more checks each compute "what counts as a valid X" independently
  - A new check evicts/skips items, and one eviction can invalidate another item
  - A check runs at a different vantage point (whole-disk vs per-agent) than an existing sibling
tags:
  - structural-invariant
  - predicate-sharing
  - shared-primitive
  - drift
  - required_tools
  - coherence-check
  - fixpoint
  - allowlist-aware
  - mika-1576
---

# Multi-layer structural invariants must share one primitive

## Context

mika#1576 added a fourth enforcement layer for one invariant: **an agent must not hold a loaded skill that requires a tool it can't call** (`[constraints] required_tools` ↔ the agent's tool surface). The same invariant was already enforced at three other layers:

- **build-time** — `verify_bundled_skills` check 4 (allowlist-unaware) + check 5 (allowlist-aware), mika#1575
- **per-turn** — the `#516` required-tools gate (keyword-matched skills only)
- and now **load-time** — `SkillRegistry::apply_required_tools_coherence_check` (allowlist-aware, all loaded skills)

Each layer must answer the sub-question "does token T resolve to a real tool?" — and "real tool" means the same thing (`BUILTIN_TOOL_NAMES ∪ KNOWN_BUILTINS`, plus the `mcp__*` exemption). This is a concrete instance of the [asymmetric-perimeter-predicate-drift](asymmetric-perimeter-predicate-drift.md) pattern (same concept, multiple consumers, diverging sets), generalized from two perimeters to N layers. The first review pass shipped the new layer with the surface-builder and the `mcp__` predicate **copied verbatim** into both the runtime check and the CLI — exactly the drift seed the parent pattern warns about.

## Guidance

When you add the Nth check of an invariant the codebase already enforces, four rules apply. They are independent — a layer can satisfy three and violate the fourth.

### 1. Share the primitive, not the policy

Extract the *definition* of "valid X" into one named function and call it from every layer. Do **not** re-implement it per call site, even when the surrounding policy differs.

```rust
// crates/mika-agent/src/skills/mod.rs — one home for "what resolves"
pub fn effective_tool_surface(skills: &[SkillEntry]) -> HashSet<String> { /* builtins ∪ skill tools */ }
pub fn required_tool_resolves(token: &str, surface: &HashSet<String>) -> bool {
    token.starts_with("mcp__") || surface.contains(token)   // the mcp__ exemption lives here, once
}
```

The runtime check and the `mika skills validate` CLI now call the *same* `required_tool_resolves`; if the exemption grows another prefix, both update together. The builtin set itself (`BUILTIN_TOOL_NAMES`) is already parity-test-guarded (mika#1217 F4) — reuse the guarded constant rather than re-listing names. What differs between layers is **which skills feed the surface** (the policy), and that stays at the call site.

### 2. Severity follows vantage point: hard-fail only where you have full information

The **same finding** warrants different severity at different vantage points:

| Vantage | Information | Disposition |
|---|---|---|
| Per-agent runtime (load-time) | Knows the agent's exact loaded skill set (allowlist-aware) | **Hard-skip** the incoherent skill |
| Whole-disk CLI (`mika skills validate`) | Allowlist-unaware; a provider may be an uninstalled dependency | **Warn only** — never exit-1 |

The CLI emitting `Fail`/exit-1 on a token a dependency would provide at runtime breaks standalone CI (e.g. a community-skill repo whose deps aren't co-installed). Match the severity to what the layer can actually prove. This mirrors `validate_skill` step 5b and check 4, which are also Warn for the allowlist-unaware class.

### 3. Eviction-based checks must reach a fixpoint

If your check *removes* items (skip/evict), removing one item can make a second item newly invalid — a consumer skill whose required tool was provided by a skill you just evicted. A single pass leaves the consumer holding an uncallable tool: precisely the state the check exists to forbid. Loop until a pass changes nothing (the set strictly shrinks, so it terminates):

```rust
loop {
    let effective = effective_tool_surface(&self.skills);
    let to_skip = /* skills with an unresolvable required_tool */;
    if to_skip.is_empty() { break; }
    self.skills.retain(|e| !to_skip.contains(&e.name));
}
```

### 4. Coherence scope ≠ enforcement scope

"Is this held skill internally consistent?" is a different question from "will this constraint be enforced this turn?". The `#516`/#463 per-turn gate deliberately does **not** enforce `required_tools` on always-on-only (non-keyword-matched) skills — it won't force a tool call (see [conditional-required-tools-enforcement-via-match-reason](conditional-required-tools-enforcement-via-match-reason.md)). The load-time coherence check is deliberately **broader**: it checks *every* loaded skill, because an always-on skill whose prompt presumes an uncallable tool is incoherent regardless of whether the constraint would fire. (The motivating mika#1406 case — Prime's always-on bearing skill — is exactly that shape.) When you reuse an existing scoping rule for a new check, confirm the scope matches the *new* question, not the old one.

## Why This Matters

The four layers exist to make the invariant impossible to violate silently. They only deliver that if they agree. Independent re-implementation (rule 1) makes them silently disagree; wrong severity (rule 2) turns a correct check into a CI outage; a non-fixpoint eviction (rule 3) leaves the exact hole the check advertises closing; borrowing the wrong scope (rule 4) either misses the target case or evicts working skills. Each rule failed silently — every per-layer test still passed. The drift is only visible from the cross-layer seat, which is why it belongs in a learning, not a single test.

## When to Apply

Reach for this checklist whenever you add a structural guard/gate/check for an invariant the codebase already enforces elsewhere — `required_tools`, identity allowlists, dispatch authorization, event-class routing, schema coherence. Before shipping: (1) did you call a shared primitive or copy one? (2) is your severity honest about what your vantage can prove? (3) if you evict, do you re-scan to a fixpoint? (4) does the scope you borrowed answer your check's question?

## Examples

mika#1576 review caught all four in one PR: the surface-build + `mcp__` predicate were duplicated (rule 1 → extracted `effective_tool_surface`/`required_tool_resolves`); the CLI emitted `Fail` (rule 2 → downgraded to `Warn`); the check was single-pass (rule 3 → wrapped in a fixpoint loop with a cascade regression test); and the broad load-time scope vs #463's keyword-only enforcement scope was undocumented (rule 4 → documented as deliberate). The F2 verification gate (`test_well_known_agents_pass_required_tools_coherence`) is the runtime sibling of check 5, asserting all shipped agents pass the new layer clean. See `docs/architecture/bundled-skill-verification.md` for the four-layer composition table.

## Related

- [asymmetric-perimeter-predicate-drift](asymmetric-perimeter-predicate-drift.md) — the parent pattern (two perimeters → N layers here)
- [intent-guard-predicate-sharing-2026-05-14](intent-guard-predicate-sharing-2026-05-14.md) — predicate-sharing instance in the dispatch-guard family
- [conditional-required-tools-enforcement-via-match-reason](conditional-required-tools-enforcement-via-match-reason.md) — the #463 keyword-only *enforcement* scope (rule 4's contrast)
