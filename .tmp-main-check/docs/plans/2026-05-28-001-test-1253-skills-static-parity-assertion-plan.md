---
title: "test(skills): static parity assertion — engine-referenced tool names must be loader-reachable"
type: feat
status: completed
date: 2026-05-28
issue: mika#1253
---

# Static Parity Assertion: Engine-Referenced Tool Names Must Be Loader-Reachable

## Overview

Add a compile-time test that asserts every skill tool name referenced by string literal in the engine's Rust code is reachable through the always-on dependency closure of `BUNDLED_SKILL_MANIFESTS`. This catches the class of bug surfaced by mika#1251 — where a tool name is known to the engine but unreachable by the skill loader on keyword-less turns (e.g., GitHub webhook auto-groom).

## Problem Frame

mika#1173 created `dev-groom` as a tool-owning skill but omitted the loader-side dependency edge from `self-dev` (always_on=true). For 6 days, `run_claude_pilot_groom` was engine-referenced but loader-unreachable on every github-webhook auto-groom turn. mika#1251 fixed the specific instance. This test generalizes the defense so any future engine-referenced skill tool name that lacks a loader path is caught at build time.

## Requirements Trace

- R1. Test named `test_engine_referenced_tool_names_are_loader_reachable` exists in `crates/mika-agent/src/skills/`
- R2. Test reads `BUNDLED_SKILL_MANIFESTS` (build-time constant), not filesystem
- R3. Test PASSES at current HEAD
- R4. Test FAILS if `dev-groom` is removed from `self-dev.dependencies`

## Scope Boundaries

- Only skill-owned tools (defined in `tools.json`) are in scope — builtin tools (Rust-defined in `crates/mika-agent/src/tools/`) are always registered by the engine and don't need loader reachability
- The test validates the worst-case keyword-less turn (only `always_on=true` seeds) — keyword-triggered reachability is a superset and doesn't need separate assertion
- No runtime or integration test — this is a pure data-structure assertion over compiled-in constants

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/bundled_skills.rs` — `all_bundled_skills()` merges legacy + build-time ENTRIES; existing test `test_self_dev_declares_both_dispatch_siblings_as_dependencies` (line 1365) uses the same data access pattern
- `crates/mika-agent/src/skills/matcher.rs` — `match_skills()` BFS algorithm for transitive dependency resolution (lines 56-73); the test replicates this walk starting from `always_on=true` seeds only
- `crates/mika-agent/src/skills/manifest.rs` — `SkillManifest` with `skill.always_on`, `skill.dependencies` fields. **No `enabled` field exists in `SkillManifest` or `skill.toml`** — enabled/disabled is a runtime DB-backed state via `skill_overrides.enabled` (tri-state nullable: NULL=default enabled, 0=disabled, 1=explicitly enabled; schema v24, mika#629). `BUNDLED_SKILL_MANIFESTS` has zero knowledge of enabled/disabled status.
- Skill tool definitions live in `tools.json` files embedded as `SkillFile` entries in each `BundledSkill`. The `BundledSkill` struct (bundled_skills.rs:44–52) carries `files: &'static [SkillFile]` where each `SkillFile` has `path: &'static str`, `content: &'static str`, `executable: bool`. To extract tool names, filter `files` for `path == "tools.json"`, parse `content` as JSON. **JSON shape:** top-level array of tool objects `[{"name": "...", "description": "...", "input_schema": {...}, "handler": {...}}]`. Tool name is `obj["name"]`. **Not all skills have tools.json** — 10 of 22 bundled skills lack it (e.g., `self-dev`, `permission-policy`, `dev-handsoff`); these must be skipped gracefully during tool-name collection.

### Engine-Referenced Skill Tool Names (Current HEAD)

Two categories of skill tool names appear in engine Rust code. The distinction determines what belongs in the registry:

**Category A — Engine post-condition guards (production code in agent.rs, engine.rs, executor.rs).** These tool names appear in `EndTurn` guard predicates, intent-precondition checks, and dispatch-class derivation. The guards fire regardless of which skills are loaded — a missing tool means the guard silently no-ops or misfires. These MUST be in the always-on closure:

| Tool Name | Owning Skill | Guard References |
|-----------|-------------|------------------|
| `run_claude_pilot` | dev-pilot | agent.rs:1713,5532,5559,5645,5709,5771; engine.rs; executor.rs; dispatcher.rs |
| `run_claude_pilot_groom` | dev-groom | agent.rs:1465,1476,1713,5532,5559,5645,5709,5771 |
| `deploy_mika` | deploy-mika | agent.rs:5785; engine.rs; executor.rs |

**Category B — Builtin handler dispatch and test-only code.** These tool names appear in `KNOWN_BUILTINS` handler dispatch tables (`builtin_handlers.rs`) or in test fixtures (`agent.rs` test modules). They're active only when the owning skill is loaded via keyword match or identity allowlist. Loader unreachability is by design (agent-specific scope), not a bug:

| Tool Name | Owning Skill | Reference Type |
|-----------|-------------|----------------|
| `gh_read` | mika-arch-groom-ticket/milestone/second-review | builtin handler dispatch + test fixtures |
| `review_skill` | skill-review | builtin handler dispatch + test fixtures |
| `qa_pr_view` | qa-review | test fixtures only |

Only Category A tools belong in `ENGINE_REFERENCED_SKILL_TOOLS`. Category B tools are correctly agent-scoped — their unreachability on non-matching agents is intentional.

### Institutional Learnings

- mika#1251 (regression) — the specific instance this test generalizes
- mika#1173 (origin) — the PR that created the engine/loader asymmetry
- mika#1244, mika#1248 — sibling structural defenses

## Key Technical Decisions

- **Central const registry over source scanning**: The ticket suggests `EngineReferencedToolNames`. A `const` slice is explicit, searchable, and maintainable. Source-scanning alternatives (regex over `agent.rs` at test time) are fragile and produce false positives. The const requires manual maintenance when adding new engine tool references, but this is a feature — it forces the developer to consciously register the dependency.

- **Test location in `bundled_skills.rs`**: The existing sibling test (`test_self_dev_declares_both_dispatch_siblings_as_dependencies`) lives here and uses the same data access pattern (`all_bundled_skills()`). The ticket says `crates/mika-agent/src/skills/<file>.rs` — `bundled_skills.rs` satisfies this constraint. No new file needed.

- **BFS walk replication**: The test replicates the `match_skills()` BFS from `matcher.rs` but simplified — no keyword matching, just pure always-on transitive closure. This mirrors the worst-case reachability (a webhook turn with no keyword hits). **Disabled-skill filtering is intentionally omitted** because `enabled` is a runtime DB flag (`skill_overrides.enabled`, schema v24), not a TOML field — `BUNDLED_SKILL_MANIFESTS` contains no enabled/disabled state, and `build.rs` has zero knowledge of it. The production `match_skills()` BFS disabled-skill gate at matcher.rs:67 is a defensive secondary guard for the runtime registry; the test operates over compile-time data where the concept does not apply. (Citation: matcher.rs:197–199 confirms "disabled skills are evicted from the registry by `apply_overrides()` before `match_skills()` is ever called"; review-guide.md § Orthogonality.)

## Implementation Units

- [x] **Unit 1: Add `ENGINE_REFERENCED_SKILL_TOOLS` const and parity test**

**Goal:** Create the central registry const and the static parity assertion test that validates every entry is reachable through the always-on dependency closure.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/bundled_skills.rs` — add const + test

**Approach:**

1. Add `ENGINE_REFERENCED_SKILL_TOOLS: &[&str]` inside the `#[cfg(test)] mod tests` block. The sole consumer is the parity test — no production code path reads this const, so `#[cfg(test)]` is the correct placement per review-guide.md § YAGNI. A well-named test-only const with a doc comment explaining the Category A/B distinction and maintenance contract serves the documentation purpose equally well without adding ~200 bytes to the production binary. Contains the 3 Category A tool names: `run_claude_pilot`, `run_claude_pilot_groom`, `deploy_mika`.

2. Add `test_engine_referenced_tool_names_are_loader_reachable` test that:
   - Calls `all_bundled_skills()` to get all embedded skill manifests
   - Parses each skill's `skill.toml` to extract `always_on` and `dependencies`
   - For each skill, finds the `SkillFile` with `path == "tools.json"` in `files`; skips skills without one (10 of 22 bundled skills lack `tools.json`). Parses the `content` as a JSON array `[{"name": "...", ...}]` and collects each `obj["name"]` as a tool name
   - Computes the always-on closure via BFS: seed with all `always_on=true` skills, walk transitive `dependencies`
   - Collects all tool names from closure-member skills' `tools.json`
   - Asserts every entry in `ENGINE_REFERENCED_SKILL_TOOLS` is in the closure-reachable set
   - On failure: names the missing tool(s) and which skill owns them, to make the fix obvious

**Patterns to follow:**
- `test_self_dev_declares_both_dispatch_siblings_as_dependencies` (bundled_skills.rs:1365) — same `all_bundled_skills()` + TOML parse pattern
- `match_skills()` BFS in `matcher.rs:56-73` — same transitive dependency walk algorithm (simplified: no keyword matching; disabled-skill filtering omitted because `enabled` is a runtime DB flag not present in compile-time `BUNDLED_SKILL_MANIFESTS` — see matcher.rs:197–199)

**Test scenarios:**
- Happy path: test passes at current HEAD — all 3 Category A engine-referenced tools (`run_claude_pilot`, `run_claude_pilot_groom`, `deploy_mika`) are reachable through the always-on closure via self-dev (always_on=true, depends on dev-pilot, dev-groom, deploy-mika)
- Regression detection: if `dev-groom` were removed from `self-dev.dependencies`, `run_claude_pilot_groom` would not be in the closure and the test would fail (verified by implementer during development, not committed)
- Error message quality: the assertion message names the unreachable tool and ideally its owning skill, so the developer knows exactly what dependency edge to add

**Verification:**
- `cargo test -p mika-agent test_engine_referenced_tool_names_are_loader_reachable` passes
- Temporarily removing `dev-groom` from `skills/bundled/self-dev/skill.toml` dependencies causes the test to fail (restore after verification)

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| New engine guard references to skill tools added without updating the const | The const has a doc comment explaining the maintenance contract and the Category A/B distinction; the developer adding the reference must add it to the const |
| Category B tool mistakenly added to the const | The doc comment on the const explains the distinction; Category B tools (agent-scoped, only active when skill is loaded) would cause test failure since they're intentionally not in the always-on closure |

## Sources & References

- Related issues: mika#1251 (regression), mika#1173 (origin), mika#1244, mika#1248 (siblings)
- Related code: `crates/mika-agent/src/skills/matcher.rs` (BFS algorithm), `crates/mika-agent/src/bundled_skills.rs` (data access pattern)

## Revision history

- rev 2 (2026-05-28): addressed F1 by adding Phase 0 pin confirming `enabled` is a runtime DB flag (`skill_overrides.enabled`, schema v24) not present in `skill.toml` or `BUNDLED_SKILL_MANIFESTS` — disabled-skill filtering omission is safe and now documented with justification citing matcher.rs:197–199 and review-guide.md § Orthogonality; addressed F2 by adding Phase 0 pins for `BundledSkill` struct definition (`files: &[SkillFile]` where `path == "tools.json"`), `tools.json` JSON shape (top-level array of `{"name": ..., ...}` objects), and tool-less skill handling (10 of 22 bundled skills lack `tools.json`, must be skipped); addressed F3 by changing `ENGINE_REFERENCED_SKILL_TOOLS` placement from "prefer module level" to `#[cfg(test)]` — sole consumer is the parity test, no production use exists or is planned, per review-guide.md § YAGNI and § KISS.
