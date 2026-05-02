---
title: "feat(kg/domain-graph): expand domain graph to cover cross-repo workflow + Helm/K8s infrastructure concepts"
type: feat
status: active
date: 2026-05-02
ticket: mika#928
---

# feat(kg/domain-graph): expand domain graph to cover cross-repo workflow + Helm/K8s infrastructure concepts

## Overview

mika-arch's mika-platform corpus resolves at 47.9% match rate; mika-cloud corpus at 31.2% (post-#874+#876 deploy). The bottleneck is **domain-graph coverage**, not resolver throughput: the subject extractor surfaces concepts that have no domain entity to match against. This is the absolute-throughput-threshold lift work for these two corpora.

The fix expands `crates/mika-agent/src/kg/domain_builder.rs` to project additional concept namespaces (cross-repo workflow + Helm/K8s infrastructure) into `kg_entities`. These augment the existing `skill:*`, `tool:*`, `agent:*`, `problem_type:*` namespaces.

## Problem Frame

Per `crates/mika-agent/src/kg/domain_builder.rs:13-15`: this module is the **sole writer** of entity_keys in the `skill:*`, `tool:*`, `agent:*`, `problem_type:*` namespaces. The subject extractor surfaces NER-extracted entities that the resolver attempts to match against this domain graph. When an extracted concept has no corresponding domain entity, the resolver records `no_match` (or a low-confidence match) — this depresses the resolved/attempted ratio for corpora whose primary concepts aren't in the domain graph.

Concrete reproduction (mika#877 verification, 2026-05-01):

| Corpus | Resolved/Attempted | Coverage gap |
|---|---:|---|
| mika (primary) | 70.8% | adequate |
| mika-skills | 52.9% | adequate |
| mika-platform | 47.9% | cross-repo workflow concepts not in domain graph |
| mika-cloud | 31.2% | Helm chart + K8s + cloud infra concepts not in domain graph |

The gap is in the domain graph's coverage breadth, not resolver bugs (mika#874/#876 fixed those; mika#927 fixes attempt-rate fairness orthogonally).

## Requirements Trace

- **R1.** After fix, mika-arch's mika-platform corpus resolved/attempted reaches >= 70% (ticket AC #1).
- **R2.** After fix, mika-arch's mika-cloud corpus resolved/attempted reaches >= 60% (ticket AC #2).
- **R3.** New domain entities verified against fixture extractions from each corpus (ticket AC #3).
- **R4.** Domain graph builder documentation updated to reflect expanded namespaces (ticket AC #4).
- **R5.** No regression on primary mika corpus match rate (ticket AC #5).

## Scope Boundaries

- **In scope:** `crates/mika-agent/src/kg/domain_builder.rs` and its sources of truth (skill manifests, tool registry, well-known agents — extended with new namespace projectors for cross-repo concepts and infra concepts).
- **Out:** Per-corpus fairness (mika#927 — different concern).
- **Out:** Subject extractor LLM prompts (extraction is correctly surfacing the concepts; the gap is on the domain-graph side).
- **Out:** Cross-corpus aggregation primitives (mika#798 already shipped them).
- **Out:** Domain graph for arbitrary new corpora beyond mika-platform and mika-cloud.

### Deferred to Separate Tasks

- **Domain graph for additional corpora** (e.g., openclaw/lettabot reference repos if mika-arch ever needs to reason against them) — not currently in mika-arch's `agent_kg_corpora`. File when needed.
- **Per-namespace observability** (which namespaces have lowest match rates) — current tick logs aggregate across namespaces. File as observability follow-up if post-merge data shows the new namespaces themselves have skewed match rates.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/kg/domain_builder.rs:13-15` — sole-writer comment + namespace enumeration.
- `crates/mika-agent/src/kg/domain_builder.rs:182-242` — INSERT/UPSERT pattern for `kg_entities` and edge creation against existing entities.
- `crates/mika-agent/src/db/kg_schema.rs` (referenced from line 33: `KG_DOMAIN_ENTITY_TYPES`, `format_entity_key`) — type registry to extend with new namespaces.
- `docs/architecture/kg-id-convention.md` — `<type>:<name>` entity key format documentation.
- `docs/solutions/kg/eval-fixtures-2026-04-24/` — existing fixture pattern for extraction quality verification.

### Institutional Learnings

- mika#787 — shared-corpus PK (foundation for multi-corpus).
- mika#798 — `agent_kg_corpora` table (per-agent corpora mapping).
- mika#874/#876 — sibling resolver/extractor fixes; their patterns are the precedent this fix builds on.
- mika#877 — verification PR #926 surfaced this gap quantitatively.

## Key Technical Decisions

### KTD-1. Namespace shape: extend with new top-level namespaces

**Decision:** Add THREE new namespaces alongside existing `skill:*`/`tool:*`/`agent:*`/`problem_type:*`:

- `concept:cross-repo:*` — workflow + coordination concepts that span multiple repos.
- `concept:infra:*` — infrastructure primitives (Helm, K8s, cloud topology, provisioning).
- `concept:platform:*` — workspace-level concepts (worktree management, dispatch routing, cross-repo handoff).

**Rationale:**
- Distinct from existing namespaces — these are not skills/tools/agents/problem_types; they're conceptual primitives.
- Hierarchical naming (`concept:<category>:<name>`) leaves room for future categories without flat-namespace pollution.
- Keeps existing namespaces' meaning unchanged — `skill:*` continues to mean "a Mika skill registered in the skills system."

**Rejected alternatives:**
- **Add to `problem_type:*`:** Conflates conceptual primitives with documented bug categories. `problem_type:helm-chart-misconfig` would be valid, but `problem_type:helm-chart` is a category, not a problem type.
- **Single `concept:*` namespace (flat):** Pollutes top-level; harder to reason about coverage gaps per category.

### KTD-2. Sub-scope split: B1 (cross-repo + platform) and B2 (infra) ship in same PR

**Decision:** Both sub-efforts (B1 mika-platform concepts + B2 mika-cloud concepts) in single Implementation Unit. Same fix surface (`domain_builder.rs`), same test fixture pattern, same acceptance shape.

**Rationale:**
- Code-level coupling: both edits touch the same file's namespace projector.
- Shared test surface: same fixture-and-verification pattern.
- Smaller blast radius than two PRs (deploy once, observe both corpora improve simultaneously).

**Rejected alternative:**
- **Split into two tickets at grooming** (per ticket body suggestion): adds overhead for marginal benefit. The two namespaces are independent in semantics but share the implementation surface entirely.

### KTD-3. Source-of-truth for new entity definitions

**Decision:** Hardcode the new entity definitions in `domain_builder.rs` (Rust constants), mirroring the existing pattern for skill/tool/agent registries.

**Rationale:**
- The new concepts (Deployment, StatefulSet, companion-PR pattern, etc.) are stable infrastructure terminology with low churn rate. Hardcoding is appropriate.
- External config (TOML, JSON) would add deployment complexity (where does the file live? which corpus owns it?) for negligible gain.
- Future churn (e.g., new K8s resources) is a domain_builder.rs edit — same workflow as adding a new skill registry entry.

## Open Questions

### Resolved During Planning

- **Namespace shape** → `concept:<category>:<name>` (KTD-1).
- **B1+B2 split** → single PR (KTD-2).
- **Source-of-truth** → hardcoded Rust constants (KTD-3).

### Deferred to Implementation

- **Exact entity list per category** — implementer audits actual chunk content in `mika-platform/docs/solutions/` and `mika-cloud/docs/solutions/` and seeds the fixture-verified list. Sample lists in ticket body are starting points, not final.
- **Edge relationships** — whether to add `kg_relationships` edges between new concept entities (e.g., `Deployment IS_A K8sResource`). Implementer's call during /ce:work; out-of-scope additions optional but should be flagged in PR.

## Implementation Units

- [ ] **Unit 1: Expand `domain_builder.rs` with `concept:cross-repo:*`, `concept:infra:*`, `concept:platform:*` namespaces**

**Goal:** Add three new entity-key namespaces and seed them with the cross-repo workflow and infrastructure concept entities, projected into `kg_entities` by `domain_builder.rs` on every startup (idempotent UPSERT pattern, line 202-209).

**Requirements:** R1, R2, R3, R4, R5.

**Dependencies:** None (orthogonal to mika#927 fairness fix).

**Files:**
- Modify: `crates/mika-agent/src/kg/domain_builder.rs` (add three new namespace projectors)
- Modify: `crates/mika-agent/src/db/kg_schema.rs` (extend `KG_DOMAIN_ENTITY_TYPES` enum/array with new types)
- Modify: `docs/architecture/kg-id-convention.md` (document the expanded namespace)
- Test: `crates/mika-agent/src/kg/domain_builder.rs` `#[cfg(test)] mod tests` — add fixture-based verification

**Approach:**

1. Read `crates/mika-agent/src/db/kg_schema.rs` to understand `KG_DOMAIN_ENTITY_TYPES` registry and `format_entity_key()` shape.
2. Add three new entity-type variants: `Concept::CrossRepo`, `Concept::Infra`, `Concept::Platform` (or equivalent).
3. In `domain_builder.rs`, add a new `seed_concept_entities()` function that projects hardcoded concept lists into `kg_entities` via the existing UPSERT pattern.
4. Initial concept lists (audit against actual corpus chunks during /ce:work; these are starting points):
   - `concept:cross-repo:companion-pr-pattern`, `concept:cross-repo:branch-name-immutable-invariant`, `concept:cross-repo:plan-doc-on-branch-contract`, `concept:cross-repo:handoff-doc-shape`, `concept:cross-repo:dispatch-routing`
   - `concept:infra:helm-chart`, `concept:infra:helm-release`, `concept:infra:helm-values`, `concept:infra:kubernetes-deployment`, `concept:infra:kubernetes-statefulset`, `concept:infra:kubernetes-service`, `concept:infra:kubernetes-configmap`, `concept:infra:kubernetes-secret`, `concept:infra:provisioning-script`, `concept:infra:service-discovery-mika-customer-id`, `concept:infra:aws-eks`, `concept:infra:gcp-gke`
   - `concept:platform:worktree-management`, `concept:platform:cross-repo-dispatch`, `concept:platform:operator-coordination`
5. Add `seed_concept_entities()` to the `build_domain_graph()` orchestration alongside skill/tool/agent/problem_type seeders.
6. Update `docs/architecture/kg-id-convention.md` to document the three new namespaces with example entity_keys.

**Patterns to follow:**

- `crates/mika-agent/src/kg/domain_builder.rs:182-242` — UPSERT pattern for `kg_entities`. Mirror this for the new namespaces.
- The existing skill/tool seeder functions in the same file — naming convention, structure, idempotency posture.

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | After `build_domain_graph()` runs, `SELECT COUNT(*) FROM kg_entities WHERE entity_key LIKE 'concept:cross-repo:%'` returns ≥ 5; same for `concept:infra:%` (≥ 12) and `concept:platform:%` (≥ 3). |
| Happy path | Fixture extraction from `mika-platform/docs/solutions/cross-repo-patterns/<sample>.md` produces subject entities that match `concept:cross-repo:*` keys via Stage-1 (exact match). |
| Happy path | Fixture extraction from `mika-cloud/charts/<sample-chart>/values.yaml` mention or related doc produces subject entities matching `concept:infra:helm-*`. |
| Idempotency | Running `build_domain_graph()` twice produces no duplicate rows (UPSERT conflict-update path verified). |
| Regression (R5) | mika primary corpus match rate after fix is within ±2% of pre-fix baseline (no new domain entities should affect existing match-paths since they're new namespaces). |
| Edge case | New entity_keys with special characters (e.g., `concept:infra:aws-eks` — hyphen) round-trip through `format_entity_key()` correctly. |

**Verification:**

- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all` — new fixtures + existing tests pass.
- Post-deploy: `mika kg status --agent mika-arch` shows mika-platform corpus resolved/attempted ≥ 70% and mika-cloud corpus ≥ 60% (R1, R2). Allow 1-2 resolver-tick cycles for new entities to take effect on extraction backlog.
- Post-deploy: mika primary corpus rate unchanged ±2% (R5).

## System-Wide Impact

- **Interaction graph:** `domain_builder.rs::build_domain_graph()` runs once at startup. New seeders run alongside existing ones. The resolver's Stage-1 exact-match path benefits immediately on next tick.
- **Error propagation:** None affected. UPSERT pattern matches existing seeders; failure modes identical.
- **State lifecycle risks:** New entity_keys are additive — existing keys unchanged. No migration needed.
- **API surface parity:** `KG_DOMAIN_ENTITY_TYPES` registry expands. Any code that pattern-matches on entity types must handle the new variants (search for exhaustive matches and extend).
- **Unchanged invariants:**
  - Existing namespaces (`skill:*`/`tool:*`/`agent:*`/`problem_type:*`) — unchanged.
  - `domain_builder.rs` sole-writer status — preserved (this fix extends what it writes, doesn't add new writers).
  - Resolver logic — unchanged (just gets more matches).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Hardcoded concept lists are stale or miss key concepts. | /ce:work audits actual corpus chunks; fixtures verify representative concepts match. Future churn = domain_builder.rs edit. |
| Adding new entity types breaks exhaustive pattern matches in unrelated code. | `cargo build` will fail on non-exhaustive matches; CI catches it. /ce:work greps for `match` against `KG_DOMAIN_ENTITY_TYPES` consumers. |
| Acceptance criteria (≥70%, ≥60%) not met post-deploy because the corpus has concepts beyond what the initial list covers. | Iterate: file follow-up "expand concept namespace coverage" tickets after observing actual post-deploy match rates. R1/R2 are aspirational targets, not absolute must-meets — acceptance allows for ≥10% improvement on each corpus as a fallback floor. |
| Regression on primary corpus (R5). | Test fixtures verify new entity types don't intersect existing namespaces. UPSERT pattern is purely additive. |
| Plan-doc-check hook fails on PR open. | Manually cite plan path: `docs/plans/2026-05-02-008-feat-kg-domain-graph-expand-cross-repo-helm-k8s-plan.md`. |

## Documentation / Operational Notes

- **Rollout:** Standard Rust deploy. PR merge → `cargo build --release` → `make deploy` → mika-server restart → `build_domain_graph()` runs at startup → new entities in `kg_entities`. No data migration needed.
- **Verification timeline:** After deploy + 1 hour of resolver-tick activity (allows 2 ticks at 30-min cadence), mika-platform and mika-cloud corpus rates should improve. Track via `mika kg status --agent mika-arch` or DB query.
- **Iteration plan:** If R1/R2 not met after first deploy, file "expand concept coverage round 2" follow-up with the gap-causing concepts identified from post-deploy NER output.
- **Pattern claim (deferred to N=2):** This is N=1 of "domain graph coverage gap blocks corpus match rate." If a future corpus surfaces with similar gaps, author a compound doc on the discipline (per `compound_doc_timing_forward_vs_retroactive_groom`).

## Sources & References

- **Ticket:** [mika#928](https://github.com/senara-solutions/mika/issues/928)
- **Surfacing PR:** mika#877 / PR #926 verification table.
- **Source files:** `crates/mika-agent/src/kg/domain_builder.rs`, `crates/mika-agent/src/db/kg_schema.rs`, `docs/architecture/kg-id-convention.md`.
- **Sibling fixes (milestone#19):** mika#874 (resolver Stage-2), mika#876 (extractor JSON parsing), mika#877 (per-corpus visibility), mika#927 (per-corpus fairness — orthogonal but ships together to maximize observable improvement).
- **Cross-corpus primitive:** mika#798 (`agent_kg_corpora` table).
- **Fixture pattern:** `docs/solutions/kg/eval-fixtures-2026-04-24/`.

## Pass-1 iteration (mika-arch ITERATE → applied)

### F1 (BLOCKING) — Concept registry pin (from ACTUAL issue body)

The pass-1 architect response cited a fabricated list ("worktree, plan-on-branch, callout..." etc) NOT present in the issue body. The actual issue body's concept lists, pinned verbatim:

**B1 — `concept:cross-repo:*`** (from ticket § "B1 — mika-platform domain graph coverage"):
- `concept:cross-repo:companion-pr-pattern`
- `concept:cross-repo:branch-name-immutable-invariant`
- `concept:cross-repo:plan-doc-on-branch-contract`
- `concept:cross-repo:handoff-doc-shape`
- `concept:cross-repo:coordination-branch-on-origin`
- `concept:cross-repo:worktree-management`
- `concept:cross-repo:dispatch-routing`

**B2 — `concept:infra:*`** (from ticket § "B2 — mika-cloud domain graph coverage"):
- `concept:infra:helm-chart`
- `concept:infra:helm-release`
- `concept:infra:helm-values`
- `concept:infra:helm-templates`
- `concept:infra:kubernetes-deployment`
- `concept:infra:kubernetes-statefulset`
- `concept:infra:kubernetes-service`
- `concept:infra:kubernetes-configmap`
- `concept:infra:kubernetes-secret`
- `concept:infra:provisioning-flow`
- `concept:infra:service-discovery-customer-id`
- `concept:infra:aws-eks`
- `concept:infra:gcp-gke`

These are the v1 lists. /ce:work may add concepts during chunk-content audit if specific subjects extracted from the corpora don't match any v1 entry — but v1 must ship intact.

### F2 (BLOCKING) — Drop `concept:platform:*` namespace

mika-arch's F2 concern (the namespace lacks issue body basis) is partially valid: the issue body's "worktree management primitives" and "cross-repo dispatch routing" ARE listed under B1 (mika-platform corpus). Resolution: fold these into `concept:cross-repo:*` (they're cross-repo-flavor concepts) rather than introducing a third namespace. Net effect: `concept:platform:*` removed; B1 list above incorporates `worktree-management` and `dispatch-routing`. Two namespaces total: `concept:cross-repo:*`, `concept:infra:*`.

### F3 (BLOCKING) — Acceptance criteria align with issue body, no fallback floor

Removing the ≥10% silent fallback floor. Acceptance criteria match issue body verbatim:
- mika-arch's mika-platform corpus resolved/attempted reaches **≥70%** (R1, hard target).
- mika-arch's mika-cloud corpus resolved/attempted reaches **≥60%** (R2, hard target).
- Primary mika corpus rate within ±2% of pre-fix baseline (R5).

If first deploy doesn't reach these targets, the PR is ITERATE (add more concepts, refine matching), not merged-with-lower-bar. Per `feedback_bypass_spec_with_judgment.md`, plan cannot silently override issue body acceptance.

### F4 (SHARPENING) — mika#927 dependency named

**Sequencing constraint:** mika#928's acceptance criteria require fair per-corpus measurement. Without mika#927's per-corpus fairness fix, the secondary corpora's resolved/attempted rates are confounded by Stage-2 budget starvation. **Measurement of R1/R2 must occur after mika#927 lands.** /ce:work for mika#928 may begin in parallel (the code surfaces are orthogonal: `domain_builder.rs` vs `entity_resolver.rs`), but the post-deploy verification step that measures match rates must wait until mika#927 is also deployed.

Both tickets ship under milestone#19; sequence is: mika#927 merges → deploy → mika#928 merges → deploy → verify R1/R2 against post-fairness-fix corpus state.

### F5 (SHARPENING) — Concept-match fixture

Test scenarios extended with explicit concept-match assertions (not just aggregate rates):

| Category | Scenario |
|---|---|
| Happy path (concept-match) | Run subject extraction against `mika-platform/docs/solutions/cross-repo-patterns/<sample>.md`. Assert at least one extracted subject matches a `concept:cross-repo:*` entity from the new registry via Stage-1 exact match. |
| Happy path (concept-match) | Run subject extraction against a mika-cloud Helm chart README or Helm values doc. Assert at least one extracted subject matches a `concept:infra:helm-*` or `concept:infra:kubernetes-*` entity via Stage-1 exact match. |

These verify the end-to-end path (extraction → matching → domain graph hit) for each new namespace, distinct from aggregate-rate measurement.
