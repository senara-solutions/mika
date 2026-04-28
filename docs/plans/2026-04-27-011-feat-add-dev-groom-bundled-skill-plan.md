---
title: "feat: Add dev-groom bundled skill encoding the two-pass grooming flow"
type: feat
status: active
date: 2026-04-27
origin: https://github.com/senara-solutions/mika/issues/845
---

# feat: Add dev-groom bundled skill

## Overview

Add a new bundled skill `mika/skills/bundled/dev-groom/` that encodes the 6-phase grooming flow currently living as the slash command `mika-platform/.claude/commands/mika-groom-ticket.md`. The skill is the sibling of `dev-pilot` (formerly `claude-pilot`, renamed in companion mika#844) in the autonomous dev loop's skill family — `dev-groom → dev-pilot → (future) dev-review`. With this work plus mika#844's rename, the family reads coherently as roles, not as the app.

**Hard constraint (Vincent's stated boundary):** there is no autonomous grooming. dev-groom is operator-triggered only — invoked by a human (Vincent or another principal) via `mika ask --agent mika "groom <ticket-ref>"` or via the `/mika-groom-ticket` slash command. mika-dev does NOT auto-invoke it on webhook events. This asymmetry with dev-pilot (which IS auto-invoked) is intentional and load-bearing. Enforcement is **structural**, not documentational — see Decision D2.

## Problem Frame

The autonomous dev loop has two halves: groom and dispatch. Today only dispatch is a first-class skill (`claude-pilot`, becoming `dev-pilot` per mika#844). Grooming lives as a slash command in mika-platform's `.claude/commands/`. The naming asymmetry hides the loop's structure and forces operators to remember which surface delivers which half. After this ticket and mika#844 ship together, the dev-* skill family is the legible representation of the workflow.

This plan is paired with mika#844 (rename `claude-pilot` skill → `dev-pilot`, GROOMED on session `d20ac2fb-bb61-4822-ba0a-4c07d73014a3`, branch `refactor/844/rename-claude-pilot-skill-to-dev-pilot` at sha `f8d33b12`, plan at `mika/docs/plans/2026-04-27-010-refactor-rename-claude-pilot-skill-to-dev-pilot-plan.md`). Tool-schema decisions in mika#844's plan Unit 2 — specifically the `skill:` argument enum on `run_claude_pilot` — are inherited by this plan with explicit Path A/B branching (see D6).

## Requirements Trace

- R1. New directory `mika/skills/bundled/dev-groom/` exists with `skill.toml`, `system_prompt.md`, `tools.json`, and `handlers/run.sh` (or equivalent — see D1).
- R2. `system_prompt.md` encodes the 6-phase flow from `mika-platform/.claude/commands/mika-groom-ticket.md`: parse ticket-ref → worktree + `/ce:plan` → first-pass `/mika-ask-arch` (parse Disposition: READY|ITERATE|ESCALATE) → iterations + second-pass with `--session-id` continuity (parse Verdict: GROOMED|ESCALATE) → finalize + `gh issue edit` → optional dispatch.
- R3. **Operator-only enforcement is structural, not documentational** (D2): `well_known_agents.rs` adds `"dev-groom"` to `disabled_skills` for mika-dev, mika-qa, and mika-relay. mika-arch and the operator-facing `mika` agent retain it. Webhook-handler in mika-gateway adds a structural guard preventing webhook-driven instantiation of the skill (analogous to mika#841's positive-consent gate).
- R4. Session capture follows mika-platform#56's JSON-metadata pattern (D3): `mika ask --format json --verbose | jq -r '.metadata.session_id'`. The skill consumes only `.metadata.session_id` regardless of envelope shape (additive-contract resilient to mika#843 expansion).
- R5. `/mika-groom-ticket.md` slash command in mika-platform stays as the operator-ergonomic entry; it becomes a thin wrapper that invokes the dev-groom skill (D4). Slash command and skill agree on canonical entry point; no implementation drift.
- R6. KG seed entry for `dev-groom` skill descriptor exists in `crates/mika-agent/src/db/kg_schema.rs`; auto-rebuilds on next server boot (per mika#844 plan F4 resolution — domain builder is sole writer + idempotent + per-boot).
- R7. Tool-selection eval fixture (`tests/eval/kg_self_knowledge/`) covers "groom <ticket-ref>" routing.
- R8. mika#844 Path A/B (D6) handled: implementer checks mika#844 merge status at start of implementation; selects the path; plan does not silently default.
- R9. End-to-end smoke: `mika ask --agent mika "groom <real-ticket>"` runs the 6 phases, plan-on-branch contract holds, issue body callouts attach correctly. Operator-only enforcement smoke: synthetic webhook event naming `dev-groom` is rejected; `mika-dev` agent invocation of `dev-groom` is rejected.

## Scope Boundaries

- Autonomous grooming (Vincent's hard constraint — separate future work).
- Modifying `dev-pilot` (companion mika#844's domain).
- Modifying `mika-arch`'s grooming or second-review skill prompts (`mika-arch-groom-ticket`, `mika-arch-second-review` are the architect-side; this skill is the operator-side).
- Modifying `/ce:plan`, `/mika-ask-a-friend`, `/mika-ask-arch` slash commands (used by dev-groom but not modified).
- Modifying mika-platform#56's `--verbose` metadata envelope shape (mika#843's domain).
- Replacing `/mika-groom-ticket.md` slash command with skill-only invocation (D4 — thin wrapper, not replace).

### Deferred to Separate Tasks

- **mika#844 (rename `claude-pilot` → `dev-pilot`):** companion ticket, sprint-bundled, deploy after both land. Tool-schema enum extension (mika#844 Path B) lands in this plan if mika#844 is not yet merged at implementation time — see D6.
- **mika#843 (`--verbose` metadata envelope expansion):** cross-cutting; affects D3's session-capture but with additive-contract resilience. dev-groom consumes one field regardless of envelope shape.
- **dev-review skill:** future ticket; not part of this work. Mentioned in Overview only as future family member.

## Context & Research

### Relevant Code and Patterns

- `mika-platform/.claude/commands/mika-groom-ticket.md` — source-of-truth for the 6-phase flow logic. The skill's `system_prompt.md` encodes this same flow with the LLM as executor.
- `mika/skills/bundled/claude-pilot/{skill.toml,system_prompt.md,tools.json,handlers/run.sh}` — canonical bundled-skill template (becoming `dev-pilot` per mika#844). Same structural shape; adapted for the grooming flow's stateful multi-step nature in D1.
- `mika/skills/bundled/permission-policy/skill.toml` — example of a skill with explicit `[triggers]` keywords and `always_on = false` activation. dev-groom follows the same shape: `always_on = false` + keyword triggers ("groom", "groom ticket").
- `crates/mika-agent/src/well_known_agents.rs` — `WellKnownAgent` records with `disabled_skills: &[...]` arrays. mika-relay disables ~all bundled skills except permission-policy. mika-qa disables a curated list. **dev-groom enters this list** for mika-dev, mika-qa, mika-relay.
- `crates/mika-agent/src/skills/mod.rs` — skill registry test fixtures showing dependency wiring patterns.
- `crates/mika-agent/src/db/kg_schema.rs` — KG seed for skill descriptors. Existing `tool:run_claude_pilot` entry at line ~209 is the reference shape; dev-groom adds a `skill:dev-groom` entry.
- `crates/mika-gateway/src/github.rs` — mika#841's positive-consent gate (the `ready` label gate). dev-groom's webhook structural guard follows the same pattern: explicit denylist or positive-consent rather than implicit-allow.
- `tests/eval/kg_self_knowledge/{path_a_direct_domain_match,path_c_semantic_via_chunks,tool_selection_query_knowledge_graph}.rs` — tool-selection eval fixtures. dev-groom adds at least one routing assertion ("groom mika issue#<n>" → dev-groom skill activation).

### Institutional Learnings

- `mika/docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md` — the contract dev-groom enforces in Phase 5.
- `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — why the branch callout matters; dev-groom inherits this discipline.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — known prompt-adherence drift on first-pass disposition keywords; dev-groom's system prompt tolerates paraphrased dispositions per existing convention.
- `mika/docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — structural guard pattern; dev-groom's webhook guard follows.
- mika#841 (gate dispatch on `ready` label or direct prompt only) — closure-bound positive-consent gate; analogous structural pattern for D2's webhook guard.

### External References

- None. This work follows established repo patterns; external research is not load-bearing.

### Companion Ticket / Cross-Coupling

- **mika#844** (GROOMED, sha `f8d33b12`): tool-schema decisions inherited per D6. Cross-ticket dependency comment posted on this ticket at https://github.com/senara-solutions/mika/issues/845#issuecomment-4329915089.
- **mika#843** (`--verbose` metadata envelope expansion, OPEN): additive-contract resilience captured in D3.

## Key Technical Decisions

### D1 — Handler shape: pure-system-prompt skill with thin convenience handler

**Position:** dev-groom uses `system_prompt.md` as the primary surface for the 6-phase flow. The LLM (operator-side mika or mika-arch agent) drives execution using existing builtin tools (`run_gh`, `mika ask` via subprocess, `git` via run-gh-style wrappers). `handlers/run.sh` is a **thin convenience layer** for deterministic IO that benefits from being scripted — branch slug derivation from issue title, plan-file sequence-number incrementing, idempotency check (worktree-already-exists). It is NOT a long-running exec handler like `claude-pilot/handlers/run.sh`; the grooming flow is multi-step LLM-driven, not a single subprocess.

**Rationale:**
- The /mika-groom-ticket slash command in mika-platform proves the pure-prompt + builtin-tools shape works today; this skill is the same shape moved into mika core for KG indexing and operator-agent ergonomics.
- A long-running exec handler (claude-pilot's shape) is wrong for grooming: the flow has two architect calls with conditional branching, plan iterations between calls, and operator confirmation at the end — not amenable to a single subprocess invocation.
- Splitting deterministic IO into a thin handler script preserves the script's value (consistent slug derivation across runs, fewer LLM re-derivations) without forcing the grooming flow into the long-running pattern.

**Alternative rejected:** the ticket body's "hybrid `exec` script + LLM `system_prompt`" framing is technically describable but practically wrong — it implies more deterministic concerns than actually exist (most of the IO is `gh` and `git` calls that the LLM already orchestrates). The pure-prompt + thin convenience handler is the same shape with less surface area. The architect's first-pass review is expected to confirm or correct this.

### D2 — Operator-only enforcement: structural, two layers (Layer 1 + Layer 3)

The skill MUST be unreachable from autonomous flows. Structural enforcement at two layers — Layer 2 (`operator_only` skill.toml flag) was rejected per YAGNI: the flag would have exactly one user (dev-groom) and would semantically duplicate Layer 1's allowlist check. New manifest schema fields for a single concrete user are speculative-flexibility; add the flag the day a second `operator_only` skill exists.

1. **Layer 1 — Skill registry / `WellKnownAgent` allowlist:** add `"dev-groom"` to `disabled_skills` arrays in `crates/mika-agent/src/well_known_agents.rs` for:
   - `MIKA_DEV` — autonomous dispatch agent. MUST NOT have grooming in scope.
   - `MIKA_QA` — review agent. Out of operator-trigger scope.
   - `MIKA_RELAY` — permission relay. Already has nearly all skills disabled.

   The operator-facing `mika` agent and `mika-arch` retain the skill. There is no `MIKA_OPERATOR` constant; the default agent (no `disabled_skills` exclusion for `dev-groom`) is the operator surface.

2. **Layer 3 — Webhook-handler structural guard in `crates/mika-gateway/src/github.rs`:** webhook-driven event flows (issue.opened, issue.labeled, comment.created, etc.) MUST NOT route to a `dev-groom`-invoking dispatch. Pattern follows mika#841's positive-consent gate: the gateway routing layer explicitly denylists `dev-groom` for any webhook-originated message dispatch. Implementation: when the gateway resolves which agent handles a webhook payload, it must reject the path if the inferred skill is `dev-groom`.

   **Layer 3 is defense-in-depth.** Layer 1 alone prevents autonomous-agent instantiation, but Layer 3 catches a future regression mode: if a new webhook event type is added that bypasses agent-identity classification (e.g., directly dispatches based on payload shape), Layer 1's allowlist may not apply. Layer 3 enforces at the gateway routing seam — the explicit guard makes the closure-bound rule visible to future engineers maintaining webhook routing.

The two layers compose against different regression modes. Layer 1 is the load-bearing primary check; Layer 3 is the safety net for routing-path additions that might bypass Layer 1.

**Layer 2 re-evaluation trigger (per architect second-pass):** Layer 2 (`operator_only` skill.toml flag) is deferred until either (a) a second `operator_only` skill exists, OR (b) a new dispatch entry point is added that bypasses Layer 1 + Layer 3 (e.g., a new tool, a new gateway routing seam, an internal RPC). When (a) or (b) becomes true, re-evaluate Layer 2 before the new dispatch path ships. This converts the YAGNI deferral into a visible decision with a re-evaluation trigger, not a forgotten hole.

### D3 — Session capture follows mika-platform#56 JSON-metadata pattern; additive-contract on mika#843

The skill's first-pass and second-pass mika-arch invocations use:

```
mika ask --agent mika-arch --format json --verbose [--session-id <id>] "<message>"
```

`session_id` is extracted via `jq -r '.metadata.session_id'`. The skill consumes ONLY `.metadata.session_id` from the response envelope — additional metadata fields (`trace_id`, `agent_id`, `provider`, `model`, timestamps, token counts, future fields landed by mika#843) are intentionally ignored. **Additive contract:** the skill's behavior does not change when mika#843 expands the envelope; consumers extract what they need and ignore the rest.

**Failure mode:** if the JSON response lacks `.metadata.session_id`, the skill fails loud with a named error — not silent fallback to text-mode trailer-line parsing. The CLI's JSON schema is the contract; this skill enforces it.

### D4 — Relationship to `/mika-groom-ticket.md` slash command: thin wrapper

The slash command at `mika-platform/.claude/commands/mika-groom-ticket.md` becomes a thin wrapper that invokes the dev-groom skill. The slash command stays as the operator-ergonomic entry (no `--skill dev-groom` typing required; matches established slash-command muscle memory).

**Three options surveyed:**

| Option | Decision | Rationale |
|---|---|---|
| (a) Replace | Rejected | Operators currently type `/mika-groom-ticket` muscularly; deletion forces re-learning. |
| (b) Thin wrapper | **Adopted** | Eliminates implementation drift (single source of truth in skill); preserves slash-command ergonomics; new operators can also invoke directly via `mika ask --agent mika "groom ..."`. |
| (c) Coexist | Rejected | Drift risk — slash command and skill diverge over time as one is updated and not the other. |

**Implementation:** after the skill ships and is verified, edit `mika-platform/.claude/commands/mika-groom-ticket.md` to replace its 6-phase prose with **imperative instructions that activate the skill** (NOT a passive delegation pointer — slash command bodies execute, they don't render):

> Activate the `dev-groom` bundled skill (mika core, `skills/bundled/dev-groom/`) and execute its 6-phase grooming flow against `$ARGUMENTS` (the typed ticket reference). The skill drives plan-on-branch contract end-to-end: parse ticket-ref → worktree + `/ce:plan` → first-pass `/mika-ask-arch` → iterations + second-pass with `--session-id` → finalize + `gh issue edit` → optional dispatch. See `mika/skills/bundled/dev-groom/system_prompt.md` for canonical phase definitions.

Imperative voice ("Activate the skill...") matches existing thin-wrapper slash commands like `mika-platform-status.md` and `mika-platform-refresh.md` — the body contains executable instructions the LLM follows. A passive delegation pointer ("This delegates to X") would be read as documentation rather than executed.

### D5 — KG seed for dev-groom auto-rebuilds on boot

Per mika#844 plan F4 resolution: `crates/mika-agent/src/kg/domain_builder.rs` is the **sole writer** of `skill:*` namespace entries; runs once per server boot after `SkillRegistry::apply_overrides()`; idempotent. Adding a new bundled skill `dev-groom` at `mika/skills/bundled/dev-groom/` causes the next server boot to auto-create the `skill:dev-groom` KG entity from the loaded skill manifest. **No explicit KG re-index command needed.**

The implementer adds a `skill:dev-groom` descriptor entry to `kg_schema.rs` for the static seed pattern (consistent with how other bundled skills are seeded), but the live KG corpus is rebuilt automatically on next boot from the registry.

### D6 — mika#844 enum coordination: NO enum extension (both paths)

**Resolved 2026-04-27 post first-pass architect review:** the original Path A/B branching (preserved below for lineage) was speculative. Under D7 (skill-not-tool) and D2 (operator-only enforcement via Layers 1+3), no caller ever passes `skill: "dev-groom"` to `run_claude_pilot` — operators activate dev-groom via skill keyword match, not via the dispatch tool. Adding `"dev-groom"` to the enum produces a structurally-unreachable code path (YAGNI violation written into the type system).

**Both Path A and Path B reduce to:** `run_claude_pilot`'s `skill:` enum stays `["dev-pilot"]` permanently. mika#845 registers the dev-groom skill but does NOT extend this enum. Cross-ticket coordination on enum value is no longer required.

**mika#844 plan amendment:** mika#844's plan Unit 2 Path A/B was amended at sha `49694521` to reflect this resolution; the cross-link comment at https://github.com/senara-solutions/mika/issues/845#issuecomment-4329915089 was superseded by a corrective addendum.

**Implementer rule (simplified):** ignore the original Path A/B branching. Skill registration happens in Unit 1; no enum-extension step appears in this plan.

### D7 — Tool selection: dev-groom is a skill, not a tool

dev-groom is invoked by skill keyword activation (e.g., "groom <ticket-ref>" matches the trigger keywords in `skill.toml`), not by a dedicated tool call. There is NO new entry in `crates/mika-agent/src/tools/` for this skill. The skill's prompt instructs the LLM to invoke existing builtin tools (`run_gh`, `mika ask`, etc.) for IO.

**Contrast with dev-pilot:** dev-pilot has the `run_claude_pilot` tool because it's a long-running subprocess wrapper. dev-groom is a multi-step LLM-driven workflow — no subprocess, no tool needed.

This is consistent with how `qa-review` and other prompt-only skills work.

## Open Questions

### Resolved During Planning

- **Handler shape:** pure-system-prompt with thin convenience handler (D1).
- **Operator-only enforcement:** two-layer structural — Layer 1 allowlist + Layer 3 gateway guard. Layer 2 (`operator_only` skill.toml flag) rejected per YAGNI (D2).
- **Session capture:** JSON-metadata + additive contract (D3).
- **Slash command relationship:** thin wrapper, not replacement (D4).
- **KG seed mechanism:** auto-rebuild on boot (D5, inherited from mika#844 F4).
- **Path A/B branching (D6):** RESOLVED to "no enum extension, both paths" per first-pass architect review and mika#844 plan amendment at sha `49694521`.
- **Tool vs skill activation:** skill-only, no new tool (D7).

### Deferred to Implementation

- ~~`skill.toml` `operator_only` field~~ — REMOVED per D2 YAGNI rejection. No schema change to the skill manifest in this PR. Add the field the day a second `operator_only` skill exists.
- **Exact text for `system_prompt.md` 6-phase encoding:** the slash command at `mika-platform/.claude/commands/mika-groom-ticket.md` is the source-of-truth template, but skill prompts differ from slash commands in framing (skill prompt addresses "you" the LLM in present-tense imperative; slash command is declarative description). The translation is mechanical but produces ~5KB of prose that doesn't fit the plan body. Implementer drafts the prose during Unit 2 from the slash-command source.
- **Webhook-handler structural guard implementation site:** the exact function in `crates/mika-gateway/src/github.rs` where the guard belongs (after webhook-source classification, before agent dispatch). Implementer reads the file at start of Unit 3 and picks the insertion point that mirrors mika#841's `ready`-label gate.
- **Whether to add a `tool:dev-groom` entry to `kg_schema.rs`:** D7 says no new tool; but if the KG seed pattern includes "skill activation events" as KG entities (analogous to `tool:run_claude_pilot`), there might be a `skill:dev-groom` entry needed for KG completeness. Implementer audits `kg_schema.rs` during Unit 5 and adds the entry if the pattern requires it.

## Output Structure

```
mika/skills/bundled/dev-groom/
├── skill.toml            # name, description, version, always_on=false,
│                         # [triggers] keywords ["groom", "groom ticket", "/mika-groom-ticket"]
│                         # NO operator_only field — D2 YAGNI rejection;
│                         # enforcement via Layer 1 (allowlist) + Layer 3 (gateway guard)
├── system_prompt.md      # 6-phase flow encoded as LLM instructions (translated from
│                         # mika-platform/.claude/commands/mika-groom-ticket.md)
├── tools.json            # Empty list [] — skill uses builtin tools, defines no new tools
└── handlers/
    └── run.sh            # Thin convenience handler for deterministic IO:
                          # branch slug derivation, plan-file path naming with sequence
                          # number, worktree idempotency check, exit trap for cleanup
```

## Implementation Units

- [ ] **Unit 1: Skill scaffolding (skill.toml + tools.json + handlers/run.sh + KG seed)**

**Goal:** Create the skill directory with the four canonical files. `system_prompt.md` is created in Unit 2; this unit establishes the structural surface (manifest, tool list, handler skeleton, KG visibility).

**Requirements:** R1, R6 (KG seed)

**Dependencies:** None (greenfield — no prior dev-groom artifacts)

**Files:**
- Create: `skills/bundled/dev-groom/skill.toml` (`name = "dev-groom"`, `description`, `version = "0.1.0"`, `always_on = false`, `timeout_secs = 600`, `[triggers] keywords = ["groom", "groom ticket", "/mika-groom-ticket", "groom <repo> issue#"]`). NO `operator_only` field — Layer 2 dropped per D2 YAGNI; enforcement via Layer 1 (allowlist) + Layer 3 (gateway guard).
- Create: `skills/bundled/dev-groom/tools.json` (empty list `[]` — D7: no new tools)
- Create: `skills/bundled/dev-groom/handlers/run.sh` (thin convenience handler — derive branch slug from `gh issue view` output, increment plan-file sequence number, check worktree idempotency)
- Create: `skills/bundled/dev-groom/system_prompt.md` (placeholder — content lands in Unit 2)
- Modify: `crates/mika-agent/src/db/kg_schema.rs` (add `skill:dev-groom` descriptor entry; auto-rebuilds on boot per D5)
- Test: existing `cargo test --package mika-agent` + `crates/mika-agent/tests/eval/test_self_knowledge_kg.rs` for KG entity surface

**Approach:**
- Mirror the structure of `skills/bundled/claude-pilot/` (becoming `dev-pilot`) for `skill.toml` and `handlers/run.sh` patterns. The `claude-pilot` template is canonical even mid-rename — the directory exists in the worktree base (sha `97584f3d`).
- `operator_only = true` is a new field in `skill.toml`. Implementer audits `crates/mika-agent/src/skills/mod.rs` for the manifest parser; adds the field as optional bool with default `false` if not already accepted (Deferred Question 1).
- KG seed entry follows the existing `tool:run_claude_pilot` pattern at `kg_schema.rs:209-210` — adapted to `format_entity_key("skill", "dev-groom")` with appropriate description text.

**Patterns to follow:**
- `skills/bundled/claude-pilot/skill.toml` — canonical bundled-skill manifest shape.
- `skills/bundled/claude-pilot/handlers/run.sh` lines 1-30 — handler skeleton pattern (jq + mika dependency check, stdin parsing, exit trap).
- `crates/mika-agent/src/db/kg_schema.rs:209-210` — KG seed entry pattern.

**Test scenarios:**
- Happy path: `cargo build` succeeds; `cargo test --package mika-agent` green.
- Edge case: skill registry test fixtures in `crates/mika-agent/src/skills/mod.rs` — assert `names.contains(&"dev-groom")`.
- Integration: `mika skills list --agent mika` (or equivalent CLI) shows dev-groom in the manifest list.
- KG: after server boot, `mika kg status --agent mika` shows `skill:dev-groom` entity.

**Verification:**
- The four canonical files exist at `skills/bundled/dev-groom/`.
- `skill.toml` parses via the existing manifest parser; `name = "dev-groom"` and `operator_only = true` are present.
- KG seed entry compiles; `kg_schema.rs` parses without error.
- `cargo build` + `cargo test --package mika-agent` green.

---

- [ ] **Unit 2: Encode the 6-phase grooming flow in system_prompt.md**

**Goal:** Translate `mika-platform/.claude/commands/mika-groom-ticket.md` into a skill `system_prompt.md` that drives an LLM through the 6 phases when the skill activates. Preserve the canonical discipline: plan-on-branch contract, two passes max (no third), citation-or-silence on architect verdicts, branch callout as canonical body element.

**Requirements:** R2

**Dependencies:** Unit 1 (skill structure exists)

**Files:**
- Modify: `skills/bundled/dev-groom/system_prompt.md` (replace placeholder with full 6-phase flow)

**Approach:**
- Source: `mika-platform/.claude/commands/mika-groom-ticket.md` § Execution (6 phases). Translate the slash-command's declarative description into LLM-prompt imperative voice. ("Take a ticket from open with description to GROOMED" → "When this skill activates, take the ticket from `<ticket-ref>` to GROOMED via the following 6 phases.")
- Preserve all the slash command's discipline:
  - Phase 2 staging-not-committing (commit gates on first-pass disposition).
  - Phase 3 paraphrase tolerance for first-pass dispositions per `mika-arch-first-dogfood-2026-04-25.md`.
  - Phase 4 no-third-pass rule (ESCALATE is real).
  - Phase 5 canonical body callouts (Branch / Plan / Grooming history).
  - Phase 6 no-auto-dispatch (operator confirms explicitly).
- The skill's prompt instructs the LLM to call existing builtin tools (`run_gh`, `mika ask --agent mika-arch ...`, `git` operations via subprocess) — no new tools per D7.
- Keep the prompt under ~5KB (system_prompt.md size budget per existing skills like `claude-pilot/system_prompt.md`).

**Execution note:** The flow has subtle sequencing — Phase 2's stage-not-commit, Phase 4's commit-revisions, Phase 5's final-commit-if-changed. The prompt must encode these explicitly; ambiguity here causes the failure mode from yesterday's mika#814 dogfood (committed unvalidated state).

**Patterns to follow:**
- `mika-platform/.claude/commands/mika-groom-ticket.md` — source of truth for the flow logic.
- `skills/bundled/claude-pilot/system_prompt.md` — example of an LLM-driven skill prompt that orchestrates multi-step IO via builtin tools.
- `skills/bundled/qa-review/system_prompt.md` — example of a prompt-only skill (no exec handler) with structured verdict output.

**Test scenarios:**
- Happy path: LLM invocation `mika ask --agent mika "groom mika issue#<test>"` activates the skill (keyword match) and produces a plan-on-branch artifact via the 6 phases.
- Edge case: ITERATE disposition at first pass triggers iteration commit + second pass (not skipped).
- Edge case: ESCALATE disposition halts; branch + plan stay committed; operator notified.
- Error path: `mika ask --agent mika-arch` returns non-JSON or missing `.metadata.session_id` → loud error, halt (per D3).
- Integration: end-to-end smoke (covered in Unit 6) verifies all 6 phases execute against a real test ticket.

**Verification:**
- `system_prompt.md` exists with all 6 phases encoded.
- Prompt size < 5KB.
- Manual prompt review confirms discipline preservation: stage-not-commit, two-pass max, paraphrase tolerance, branch callout canonical, no auto-dispatch.

---

- [ ] **Unit 3: Operator-only enforcement (two structural layers)**

**Goal:** Make autonomous instantiation of dev-groom impossible via two structural mechanisms — Layer 1 (allowlist) is the load-bearing primary check; Layer 3 (gateway guard) is defense-in-depth against future webhook-routing regressions. Layer 2 (`operator_only` skill.toml flag) was rejected per YAGNI in D2.

**Requirements:** R3

**Dependencies:** Unit 1 (skill exists in registry)

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` — add `"dev-groom"` to `disabled_skills` for `MIKA_DEV`, `MIKA_QA`, `MIKA_RELAY`. (mika-arch and the default operator agent retain it.)
- Modify: `crates/mika-gateway/src/github.rs` — webhook-handler structural guard. After webhook-source classification, reject any inferred skill activation that resolves to `dev-groom`. Pattern: explicit denylist check, similar to mika#841's positive-consent gate logic.
- Test: `crates/mika-gateway/src/github.rs` (or `tests/`) — synthetic webhook event whose payload would otherwise infer dev-groom is rejected at the gateway routing layer.
- Test: `crates/mika-agent/src/well_known_agents.rs` (inline tests `test_seed_skill_overrides_*`) — assert `dev-groom` is in disabled list for mika-dev/qa/relay, NOT in disabled list for the operator default agent.

**Approach:**
- Layer 1 (well_known_agents.rs): straightforward addition to the existing `disabled_skills` arrays. Each existing array already has 5-10 entries; this adds one more.
- Layer 3 (webhook guard): the gateway's GitHub webhook handler classifies the event and routes to an agent. Insertion point is **after** classification (so we know the source is webhook) and **before** dispatch. mika#841's `ready` label check is the reference pattern — same shape: "if condition, halt before dispatch." Layer 3 explicitly names dev-groom in a denylist, making the closure-bound rule visible to future engineers maintaining webhook routing.

**Execution note:** Layers compose against different regression modes. Layer 1 is the load-bearing primary check (catches mis-configured agents); Layer 3 catches future webhook-routing additions that bypass agent-identity classification. Don't refactor Layer 3 as "redundant" — it's deliberate defense-in-depth.

**Patterns to follow:**
- mika#841's `ready` label gate in `crates/mika-gateway/src/github.rs` (`refactor/841/...` merge — see mika#842 PR for the diff).
- `crates/mika-agent/src/well_known_agents.rs:447 seed_well_known_skill_overrides` — the existing override mechanism.

**Test scenarios:**
- Happy path: `mika ask --agent mika "groom mika issue#<n>"` activates dev-groom (mika is operator agent, not in disabled list).
- Happy path: `mika ask --agent mika-arch "groom mika issue#<n>"` activates dev-groom (mika-arch retains it).
- Error path (Layer 1): `mika ask --agent mika-dev "groom mika issue#<n>"` does NOT activate dev-groom (skill in disabled list).
- Error path (Layer 3): synthetic GitHub webhook payload that would otherwise route to dev-groom is rejected at the gateway layer; no agent dispatch occurs.
- Integration: composite — even with Layer 1 explicitly bypassed in a test fixture, Layer 3 still prevents instantiation via webhook-routing path.

**Verification:**
- `cargo test --package mika-agent` green; `well_known_agents.rs` allowlist tests pass with dev-groom in disabled lists for mika-dev/qa/relay.
- `cargo test --package mika-gateway` green; webhook-guard test passes.
- Manual smoke: synthetic webhook event POSTed to gateway in test mode does not produce a dev-groom invocation log line.

---

- [ ] **Unit 4: Tool-selection eval fixture for dev-groom skill activation**

**Goal:** Add the "groom <ticket-ref>" routing assertion to `tests/eval/kg_self_knowledge/`. The assertion verifies that "groom <ticket-ref>" prompts route to dev-groom skill activation (keyword match), NOT to `run_claude_pilot` tool dispatch.

**Requirements:** R7

**Dependencies:** Unit 1 (KG seed exists), Unit 2 (skill prompt encodes the activation flow), Unit 3 (operator-only enforcement is in place)

**Files:**
- Modify: `tests/eval/kg_self_knowledge/path_a_direct_domain_match.rs` (assertion: "groom <ticket-ref>" → `dev-groom` skill activation, NOT `run_claude_pilot` tool).

**Approach:**
- Per D6 resolution: NO enum extension on `run_claude_pilot`. The `skill:` enum stays `["dev-pilot"]` regardless of mika#844 merge order.
- Per D7: dev-groom is skill-only, never via the dispatch tool.
- Eval fixture assertion verifies the activation path: "groom <ticket-ref>" prompt → keyword match on `[triggers]` → dev-groom system_prompt loads → LLM drives 6 phases. Distinct from "implement <ticket-ref>" which routes to `run_claude_pilot {skill: "dev-pilot"}`.
- Verify the `run_claude_pilot {skill: "dev-groom"}` path does NOT exist (assertion that dispatch enum has only `"dev-pilot"`).

**Patterns to follow:**
- `tests/eval/kg_self_knowledge/path_a_direct_domain_match.rs` — existing routing assertions.
- mika#844's plan Unit 2 — the symmetric Path A/B pattern.

**Test scenarios:**
- Happy path: "groom mika issue#123" prompt → KG self-knowledge tool selection returns dev-groom skill activation (not `run_claude_pilot` tool).
- Edge case: "implement mika issue#123" → `run_claude_pilot {skill: "dev-pilot"}` (separate from dev-groom; reaffirms the implement-vs-groom routing distinction).
- Negative assertion: "groom mika issue#123" does NOT produce `run_claude_pilot {skill: "dev-groom"}` — that enum value never exists per D6.
- Integration: post-deploy, `mika ask --agent mika "groom mika issue#<test>"` activates dev-groom (verified in Unit 6 smoke).

**Verification:**
- `cargo test --package mika-agent --test eval` green.
- The new assertion in `path_a_direct_domain_match.rs` passes.
- The `skill:` enum in dev-pilot's `tools.json` (post-mika#844-merge) contains only `"dev-pilot"` — no `"dev-groom"`.

---

- [ ] **Unit 5: Slash command thin wrapper (mika-platform repo, companion PR)**

**Goal:** Edit `mika-platform/.claude/commands/mika-groom-ticket.md` to become a thin delegation wrapper; the dev-groom skill is now the source-of-truth for the 6-phase flow.

**Requirements:** R5

**Dependencies:** Unit 2 (skill prompt fully encodes the flow)

**Files (mika-platform repo, companion PR):**
- Modify: `.claude/commands/mika-groom-ticket.md` (replace 6-phase prose with brief delegation pointer to `mika/skills/bundled/dev-groom/system_prompt.md`).

**Approach:**
- The slash command becomes a discovery surface (operators searching for `/mika-groom` find it; the file body explains where the canonical flow lives).
- Suggested new content shape: keep the frontmatter (name, description, argument-hint) so `/mika-groom-ticket` continues to work as a slash command. Replace the body with **imperative skill-activation instructions** (slash command bodies execute, they don't render — matches existing thin-wrapper pattern in `mika-platform-status.md`, `mika-platform-refresh.md`):
  > Activate the `dev-groom` bundled skill (mika core, `skills/bundled/dev-groom/`) and execute its 6-phase grooming flow against `$ARGUMENTS` (the typed ticket reference). The skill drives plan-on-branch contract end-to-end: parse ticket-ref → worktree + `/ce:plan` → first-pass `/mika-ask-arch` → iterations + second-pass with `--session-id` → finalize + `gh issue edit` → optional dispatch. See `mika/skills/bundled/dev-groom/system_prompt.md` for canonical phase definitions. Operators who prefer direct invocation can equivalently use `mika ask --agent mika "groom <ticket-ref>"`.
- **Sequencing constraint (per architect Finding 4):** Unit 5 ships in a follow-up PR AFTER Units 1-4 are deployed and smoke-green. The slash command wrapper takes effect immediately on PR merge (no build step on mika-platform `.claude/commands/`); the skill needs deploy first. Same sprint OK; same PR not OK. If the wrapper merges before skill deploys, operators see a delegation pointer to a non-existent skill.

**Patterns to follow:**
- mika-platform's existing thin command wrappers like `/mika-platform-status` (delegates to a script in `scripts/`).
- The "single source of truth" pattern: skill is the implementation, slash command is the entry point.

**Test scenarios:**
- Happy path: operator runs `/mika-groom-ticket mika issue#<n>` in interactive mode; the slash command's brief body redirects to the skill, OR the operator agent loads the skill via the keyword "groom" in the command body and proceeds.
- Edge case: a stale operator client that hasn't loaded the new skill still sees the slash command file but encounters a delegation note rather than the canonical flow. Acceptable degradation: operator sees the pointer and runs `mika ask --agent mika "groom ..."` directly.
- Integration: end-to-end smoke (Unit 6) verifies both invocation paths produce equivalent grooming artifacts.

**Verification:**
- `mika-platform/.claude/commands/mika-groom-ticket.md` body is < 500 bytes (was ~7KB).
- The frontmatter (name, description, argument-hint) is preserved.
- A grep for `/mika-groom-ticket` in the mika-platform repo shows the wrapper is the single touchpoint; no orphan references to the old 6-phase prose.

---

- [ ] **Unit 6: Deploy + end-to-end smoke + operator-only enforcement smoke**

**Goal:** Land the skill via `make deploy`. Verify all three operator-only enforcement layers via smoke. Run end-to-end grooming on a low-stakes test ticket to confirm the 6 phases execute correctly through the new skill.

**Requirements:** R9

**Dependencies:** Units 1-5 merged; mika#844 merged or deploy-coordinated.

**Files:**
- No new source files. Procedure captured in PR description and re-emitted to deploy log.

**Approach:**

**Step 1 — Deploy:**
- `make deploy` rebuilds binary, restarts agent service.
- KG domain graph auto-rebuilds on next boot per D5; `skill:dev-groom` entity appears in `kg_entities` automatically.
- No explicit KG re-index command needed (per mika#844 plan F4 resolution).

**Step 2 — Operator-only enforcement smoke (three checks):**

1. **Layer 1 (allowlist):** `mika ask --agent mika-dev "groom mika issue#<test>"` — expect: skill does NOT activate; mika-dev's normal autonomous flow either rejects the unknown command or routes elsewhere. Log line should NOT contain `skill:dev-groom`.
2. **Layer 2 (`operator_only` flag):** synthetic test fixture invoking dev-groom from an autonomous-dispatch entry point — expect: rejection at the executor layer with a named error referencing the `operator_only` flag.
3. **Layer 3 (webhook guard):** synthetic GitHub webhook event POSTed to mika-gateway in test mode that would otherwise route to dev-groom — expect: gateway-side rejection, no agent dispatch.

**Step 3 — End-to-end grooming smoke:**

- Pick a low-stakes test ticket (or use a synthetic). `mika ask --agent mika "groom mika issue#<test>"` activates dev-groom.
- Verify all 6 phases execute:
  - Phase 1: branch slug derivation produces expected slug.
  - Phase 2: worktree created at canonical path; plan file at `docs/plans/<date>-<NNN>-...`.
  - Phase 3: `/mika-ask-arch` first-pass invoked; session_id captured from `.metadata.session_id`.
  - Phase 4: ITERATE flow applies revisions and second-pass with `--session-id`.
  - Phase 5: branch pushed; issue body updated with canonical callouts; summary comment posted.
  - Phase 6: skill stops; no auto-dispatch.
- Cleanup: cancel the test grooming (close the synthetic test ticket; remove the worktree).

**Step 4 — Slash command equivalence smoke (after Unit 5's mika-platform PR ships):**

- Operator runs `/mika-groom-ticket mika issue#<test>` in interactive mode.
- Verify equivalent artifact (same plan, same branch shape, same callouts).

**Patterns to follow:**
- mika#841 deploy verification pattern (positive-consent gate smoke).
- mika#844 plan Unit 6 deploy procedure (auto-rebuild on boot, smoke against test ticket).

**Test scenarios:**
- Happy path: deploy succeeds; smoke passes all 6 phases; operator-only enforcement smoke passes all 3 layers.
- Error path: any layer of operator-only fails → halt deploy, investigate; revert if needed.
- Error path: end-to-end smoke fails (Phase 3 architect call returns malformed JSON, Phase 5 `gh issue edit` fails permissions, etc.) → fix at root cause; the cutover discipline doesn't have an alias for "skip deploy verification."

**Verification:**
- `make deploy` exits 0; agent service is up.
- All three operator-only smoke checks pass; logs show explicit rejection at each layer for the rejection cases.
- End-to-end smoke produces a plan-on-branch artifact matching expected shape; issue body has Branch/Plan/Grooming-history callouts; summary comment posted.
- Post-deploy `mika kg status --agent mika` shows `skill:dev-groom` entity.

## System-Wide Impact

- **Interaction graph:** new skill activation flow — operator → `mika`/`mika-arch` agent → keyword match on "groom" → dev-groom skill prompt → orchestrates `run_gh`/`mika ask`/`git` builtins through 6 phases → produces plan-on-branch artifact + GitHub state changes (issue body update, comment).
- **Error propagation:** Phase 3/4 architect call failures (JSON parse, missing `.metadata.session_id`) surface as named errors in the skill's prompt-driven flow. Phase 5 `gh` command failures retry per existing builtin retry policy. Operator sees clear failure messages at each phase boundary.
- **State lifecycle risks:** Phase 2's stage-not-commit discipline prevents a critical race — committing unvalidated plan state before architect signs. The skill's prompt MUST encode this; ambiguity here was the failure mode of yesterday's mika#814 dogfood.
- **API surface parity:** `/mika-groom-ticket.md` slash command in mika-platform stays as the operator-facing surface (D4 thin wrapper); operators have two equivalent paths (slash command in interactive mode, `mika ask` for headless). Skill is the single source of truth.
- **Integration coverage:** Unit 4's eval fixture covers tool-selection routing; Unit 6's end-to-end smoke covers the full 6-phase flow against real GitHub state. Operator-only enforcement (Unit 3) has independent layer tests + composite layer-bypass test.
- **Unchanged invariants:** `dev-pilot` skill (mika#844's domain), `mika-arch-groom-ticket` and `mika-arch-second-review` skills (architect-side; this skill is the operator-side), `/ce:plan` and `/mika-ask-a-friend` and `/mika-ask-arch` slash commands (used but not modified), `tasks.metadata.claude_pilot.*` JSON namespace (app-level, untouched). The mika-gateway webhook routing for non-grooming flows (mika#841's positive-consent gate, dispatch via dev-pilot) is unaffected — Unit 3 Layer 3 adds a denylist branch parallel to mika#841's path.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `system_prompt.md` translation from slash command drops a discipline (e.g., stage-not-commit) | Unit 2 verification step: manual prompt review against the slash command source-of-truth, checking each discipline preservation explicitly |
| Operator-only enforcement Layer 1 regression (dev-groom removed from disabled_skills) goes unnoticed | Layers 2+3 catch it in production; CI test asserts `dev-groom` IS in disabled list for mika-dev/qa/relay (Unit 3 test fixture) |
| Webhook structural guard insertion point misses an event class | Composite test (Unit 3 test scenarios — Layer 3 with synthetic webhook) covers the event classes mika#841 enumerated; new event classes are caught at PR review for any future webhook additions |
| ~~mika#844 Path B coordination~~ | ~~Resolved per D6: no enum extension, both paths~~. mika#844 plan amended at sha `49694521`; cross-ticket comment superseded. No coordination needed on `skill:` enum value. |
| Slash command thin wrapper (Unit 5) lands on mika-platform PR before the skill ships on mika repo PR — operators see a delegation pointer to a non-existent skill | Sprint-bundle: deploy after both PRs land (mika#845 main PR + mika-platform companion PR). Unit 5 verification step explicitly confirms skill availability before merging the wrapper |
| `system_prompt.md` exceeds the 5KB size budget | Unit 2 verification: prompt size check. If over budget, factor common discipline patterns into shorter forms; the slash command is ~7KB but contains examples and rationale that the skill prompt can omit (skill prompt is imperative, not explanatory) |
| ~~Path A correctness check missing mika#844's merge~~ | ~~Resolved per D6: no Path A/B branching exists; rule is mechanical (no enum extension regardless)~~ |

## Documentation / Operational Notes

- The PR description must include:
  - The Path A vs Path B decision and the supporting `gh pr list` query output.
  - Manual prompt review against the slash command source (Unit 2 verification artifact).
  - All three operator-only enforcement layer tests passing.
  - End-to-end smoke output (which phases ran, plan-on-branch artifact link).
- After this ticket and mika#844 both ship, file a `/ce:compound` doc capturing the dev-* skill family pattern (operator-vs-autonomous separation, structural enforcement, slash-command-as-thin-wrapper) for future similar work.
- mika-platform's `CLAUDE.md` § Cross-Repo Relationships should mention `dev-groom` as the operator-side grooming skill (currently mentions `claude-pilot` only).
- The companion PR on mika-platform (Unit 5) has its own deploy concern: the slash command wrapper takes effect immediately on PR merge (no build step); test the equivalence smoke (Step 4 of Unit 6) only after both PRs deploy.

## Sources & References

- **Origin ticket:** [senara-solutions/mika#845](https://github.com/senara-solutions/mika/issues/845)
- **Companion ticket:** [senara-solutions/mika#844](https://github.com/senara-solutions/mika/issues/844) — rename `claude-pilot` skill → `dev-pilot` (GROOMED, sha `f8d33b12`)
- **Cross-cutting:** [senara-solutions/mika#843](https://github.com/senara-solutions/mika/issues/843) — `--verbose` metadata envelope expansion (additive contract per D3)
- **Prerequisite:** [senara-solutions/mika-platform#56](https://github.com/senara-solutions/mika-platform/issues/56) (merged) — `--verbose` JSON metadata session capture pattern (D3 inherits)
- **Reference pattern:** [senara-solutions/mika#841](https://github.com/senara-solutions/mika/issues/841) (merged, PR #842) — positive-consent gate / webhook structural guard pattern (Unit 3 Layer 3)
- **Source of truth (slash command):** `mika-platform/.claude/commands/mika-groom-ticket.md`
- **Canonical bundled-skill template:** `mika/skills/bundled/claude-pilot/` (becoming `dev-pilot` per mika#844)
- **Architect verdict (companion ticket):** mika-arch session `d20ac2fb-bb61-4822-ba0a-4c07d73014a3` (mika#844 grooming)
- **Pre-filing scope verification:** `mika-platform/docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md`
- **Plan-on-branch contract:** `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md`
- **Architect verdict drift:** `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` (paraphrase tolerance preserved in Unit 2)
