# Plan: Reframe `validate_loaded()` as crash-protection, not validation gate (mika#1335)

## Context

Per Rec 2 of the 2026-05-09 lifecycle-redesign brainstorm, skill manifest validation should run at change-time (CI / CLI verb), and the running agent should consume pre-validated artifacts rather than re-validate at runtime — on orthogonality grounds.

The running agent currently re-validates every loaded skill manifest at startup via `SkillRegistry::validate_loaded()` (#530). This calls `validate_skill(&entry.dir)` per skill, classifies failures via `is_skip_worthy_failure()`, and either skips the skill (crash-protection) or records a warning.

**Tension:** `validate_loaded()` is not redundant — it is a defensive load-time backstop that prevents a malformed manifest from poisoning or crashing a running agent. The ticket asks us to resolve the orthogonality-vs-safety-net tension.

## Decision: Candidate framing #1 — Keep the startup safety check, reframe its role

**Rationale:** The startup `validate_loaded()` pass serves a fundamentally different purpose than CI/CLI validation:

- **CI/CLI validation** = change-time quality gate (catch errors before they reach production)
- **`validate_loaded()`** = runtime crash-protection (survive filesystem corruption, symlink races, manually-installed skills, marketplace skills that bypassed CI)

These are orthogonal concerns. The code is already correct — only the documentation and naming don't make the boundary explicit. This is the cheapest path that fully satisfies the orthogonality concern without removing any safety property.

**Why not framing #2 (demote to fail-fast-only):** Removing the richer semantic re-validation would leave locally-installed, `--link`, and marketplace skills without any runtime safety net. Those skills bypass CI entirely.

**Why not framing #3 (gate on skill origin):** The complexity of per-origin gating is not justified. The `validate_loaded()` pass is cheap (filesystem-only, no LLM calls, no network) and runs once at startup. The marginal cost of re-validating CI-covered skills is negligible.

## Phase 0 Pins

### Pin 1: Callsite inventory (`grep -rn "validate_loaded" crates/`)

Non-test method calls (4 callsites):
- `crates/mika-cli/src/commands/ask.rs:335` — `skill_registry.validate_loaded()`
- `crates/mika-cli/src/commands/chat.rs:128` — `skill_registry.validate_loaded()`
- `crates/mika-agent/src/server/mod.rs:415` — `skill_registry.validate_loaded()`
- `crates/mika-agent/src/tools/list_skills.rs:59` — `registry.validate_loaded()`

Internal references (definition + doc comments, in `crates/mika-agent/src/skills/mod.rs`):
- Line 272: `pub fn validate_loaded(&mut self)` — method definition
- Line 165, 218, 258: doc comments referencing `validate_loaded()`

Other crate references:
- `crates/mika-agent/src/skills/index.rs:306` — doc comment on `SkillValidationWarning`
- `crates/mika-agent/build.rs:118` — doc comment referencing `validate_loaded()`

Test functions (13 in `crates/mika-agent/src/skills/mod.rs`):
- `test_validate_loaded_no_issues` (line 1889)
- `test_validate_loaded_empty_registry` (line 1905)
- `test_validate_loaded_llm_section_warns_not_skipped` (line 1915)
- `test_validate_loaded_name_in_keywords_warns_not_skipped` (line 1946)
- `test_validate_loaded_missing_handler_script_skips` (line 1971)
- `test_validate_loaded_handler_not_executable_skips` (line 2001)
- `test_validate_loaded_invalid_tools_json_skips` (line 2033)
- `test_validate_loaded_skip_worthy_and_warn_both_present_skips` (line 2046)
- `test_validate_loaded_warn_only_diagnostics_kept` (line 2091)
- `test_validate_loaded_multiple_skills_mixed` (line 2111)
- `test_validate_loaded_symlink_race_all_fail_no_ok` (line 2157)
- `test_validate_loaded_always_on_oversized_prompt_skips` (line 2175)
- `test_validate_loaded_skip_reason_contains_diagnostic` (line 2212)

**No hot-reload sites found.** `server/handlers.rs` and `server/a2a.rs` do not call `validate_loaded()`.

### Pin 2: Doc cross-reference inventory (`grep -rn "agent-tool-must-call-validate-loaded\|validate_loaded" docs/ crates/mika-agent/CLAUDE.md`)

Files requiring update (current references, excluding historical plan files and the plan itself):
- `docs/solutions/best-practices/agent-tool-must-call-validate-loaded-on-skill-registry.md` — file rename + 13 internal references
- `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` — 2 references (line 33, line 69)
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — 2 references (line 76, line 89)
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — 1 cross-link to best-practices doc filename (line 201)
- `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md` — 1 reference (line 42)
- `crates/mika-agent/CLAUDE.md` — 2 references (line 191, line 193)
- `crates/mika-agent/docs/skills.md` — 1 reference (line 1143, crates.io fallback copy)
- `docs/skills.md` — 1 reference (line 1143, source of truth)
- `crates/mika-agent/build.rs` — 1 reference in doc comment (line 118)

Files NOT updated (historical records — plan files are immutable post-merge):
- `docs/plans/2026-04-13-004-feat-startup-skill-validation-plan.md` (20+ references)
- `docs/plans/2026-04-15-003-feat-list-skills-report-skipped-count-plan.md` (1 reference)
- `docs/plans/2026-04-16-005-refactor-bundled-skills-directory-source-plan.md` (1 reference)
- `docs/plans/2026-04-18-001-feat-skill-enabled-state-db-eviction-plan.md` (4 references)
- `docs/plans/2026-04-20-002-feat-skill-always-on-cli-flag-plan.md` (1 reference)
- `docs/plans/2026-04-20-005-feat-enable-disable-skill-flags-plan.md` (1 reference)
- `docs/plans/2026-04-21-004-feat-domain-graph-builder-plan.md` (1 reference)
- `docs/plans/2026-04-21-005-feat-lexical-ingestion-plan.md` (1 reference)
- `docs/plans/2026-04-13-005-fix-skills-validator-handler-path-canonicalization-plan.md` (1 reference)

## Implementation units

### Unit 1: Rename and document the role boundary

**Goal:** Make the orthogonality boundary explicit in code and documentation.

**Files:**
- `crates/mika-agent/src/skills/mod.rs` — rename method + update doc comment
- `crates/mika-agent/src/skills/index.rs` — update `is_skip_worthy_failure` doc comment
- `crates/mika-agent/CLAUDE.md` — update Skills System § Validation
- `docs/skills.md` — update validation section

**Changes:**

1. **Rename `validate_loaded()` → `apply_load_safety_check()`** on `SkillRegistry`. This name communicates that the method is a safety net (crash-protection), not a validation gate. The method body is unchanged — only the name and doc comments change.

2. **Update the doc comment** on the renamed method to explicitly state the orthogonality boundary:
   ```rust
   /// Run load-time crash-protection on all loaded skills.
   ///
   /// This is NOT the validation gate — CI and `mika skills validate` own change-time
   /// validation. This method is a runtime safety net that prevents malformed manifests
   /// from crashing or poisoning the running agent.
   ///
   /// Skills with skip-worthy structural failures (missing handler, broken tools.json,
   /// unreadable manifest, oversized always_on prompt) are removed from `self.skills`
   /// and added to `self.skipped`. Skills with non-fatal warnings are kept loaded and
   /// recorded in `self.validated_warnings`.
   ///
   /// Must be called **after** `apply_overrides()` since DB overrides can change
   /// `always_on` state and LLM configuration, affecting validation context.
   ```

3. **Update `is_skip_worthy_failure()` doc comment** to reference the crash-protection framing:
   ```rust
   /// Classify whether a Fail diagnostic warrants skipping the skill at load time.
   ///
   /// This is part of the runtime crash-protection layer — not the change-time
   /// validation gate. Skip-worthy failures indicate structural corruption that
   /// would cause runtime errors (missing handler, broken tools.json, unreadable
   /// manifest).
   ```

4. **Update `validated_warnings()` accessor doc comment** to use "load-safety" language.

5. **Update `crates/mika-agent/CLAUDE.md`** — in the Skills System § Validation paragraph, replace:
   > **Startup validation (#530):** `SkillRegistry::validate_loaded()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning.

   with:
   > **Load-time crash-protection (#530, #1335):** `SkillRegistry::apply_load_safety_check()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. This is NOT the validation gate — CI and `mika skills validate` own change-time validation. This method is a runtime safety net that prevents malformed manifests from crashing the agent. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning.

6. **Update `docs/skills.md`** — update any references to `validate_loaded()` to use the new name and framing.

### Unit 2: Update all callsites

**Goal:** Rename the method call at every site that invokes `validate_loaded()`.

**Files (4 callsites — exhaustive per Phase 0 Pin 1):**
- `crates/mika-cli/src/commands/ask.rs:335` — `skill_registry.validate_loaded()` → `skill_registry.apply_load_safety_check()`
- `crates/mika-cli/src/commands/chat.rs:128` — same rename
- `crates/mika-agent/src/server/mod.rs:415` — same rename
- `crates/mika-agent/src/tools/list_skills.rs:59` — same rename

**Also update internal references (3 files):**
- `crates/mika-agent/src/skills/index.rs:306` — doc comment on `SkillValidationWarning`
- `crates/mika-agent/build.rs:118` — doc comment referencing `validate_loaded()`
- `crates/mika-agent/src/skills/mod.rs` — doc comments at lines 165, 218, 258

**Mechanical:** Find-and-replace `validate_loaded()` → `apply_load_safety_check()` at each callsite and doc comment. No logic changes. No hot-reload sites exist (confirmed by Phase 0 Pin 1).

### Unit 3: Update solution docs and cross-references

**Goal:** Update all documented references to `validate_loaded()` to use the new name and framing. Full file list per Phase 0 Pin 2.

**Files (7 — exhaustive per Phase 0 Pin 2, excluding historical plan files which are immutable post-merge):**
- `docs/solutions/best-practices/agent-tool-must-call-validate-loaded-on-skill-registry.md` — rename file to `agent-tool-must-call-apply-load-safety-check-on-skill-registry.md`, update all 13 internal references
- `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` — update 2 references (lines 33, 69)
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — update 2 references (lines 76, 89)
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — update cross-link to renamed best-practices doc (line 201)
- `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md` — update 1 reference (line 42)
- `docs/skills.md` — update 1 reference (line 1143, source of truth)
- `crates/mika-agent/docs/skills.md` — update 1 reference (line 1143, crates.io fallback copy; sync via `scripts/sync-agent-docs.sh`)

### Unit 4: Update test names

**Goal:** Rename test functions that reference `validate_loaded` to reference the new name.

**Files:**
- `crates/mika-agent/src/skills/mod.rs` — rename all `test_validate_loaded_*` test functions to `test_apply_load_safety_check_*`

**Mechanical:** Pure rename, no logic changes. ~15 test functions.

## Interaction graph

```
validate_skill() ← unchanged (shared by CI/CLI and runtime)
    ↓
is_skip_worthy_failure() ← doc-only change (crash-protection framing)
    ↓
apply_load_safety_check() ← renamed from validate_loaded()
    ↓
4 callsites + 3 doc-comment refs ← mechanical rename (Phase 0 Pin 1)
```

No behavioral changes. The decision matrix, the skip-worthy classification, and the two-phase collect-then-apply pattern are all unchanged.

## Risks

| Risk | Mitigation |
|------|------------|
| Rename breaks external consumers | `validate_loaded()` is `pub` on `SkillRegistry` but the crate is not published to crates.io as a library. All consumers are in-tree. `cargo build` catches any missed callsites. Phase 0 Pin 1 enumerates all 4 callsites + 3 doc-comment refs. |
| Doc references go stale | Phase 0 Pin 2 enumerates the exhaustive cross-reference set (7 files). Historical plan files are immutable post-merge and excluded. `grep -r validate_loaded` after implementation confirms none are missed. |
| Rename is bikeshedding | The rename is the minimal change that satisfies the ticket's AC3 (document the orthogonality boundary). The alternative — keeping the name and only updating docs — leaves the naming inconsistent with the documented role. |

## Acceptance criteria mapping

| AC | Satisfied by |
|----|-------------|
| AC1: Resolve orthogonality-vs-#530-backstop tension explicitly | Decision section + Unit 1 doc updates: the boundary is "crash-protection, not validation gate" |
| AC2: Running agent must not crash on malformed manifest | No behavioral change — `apply_load_safety_check()` has identical semantics to `validate_loaded()` |
| AC3: Document orthogonality boundary in `crates/mika-agent/CLAUDE.md` | Unit 1 step 5 |

## Out of scope

- Changing the behavior of `validate_loaded()`/`apply_load_safety_check()` — the current behavior is correct
- Adding origin-based gating (framing #3) — complexity not justified
- Removing any validation checks (framing #2) — safety regression
- Changes to CI or CLI validation — already satisfied per mika-skills#162

## Revision history

- rev 2 (2026-05-30): addressed F1 by adding Phase 0 Pin 1 with exhaustive `grep -rn "validate_loaded" crates/` results — confirmed 4 non-test callsites (not 6), no hot-reload sites, 13 test functions, 3 doc-comment refs; updated Unit 2 and interaction graph to reflect pinned counts. Addressed F2 by adding Phase 0 Pin 2 with exhaustive doc cross-reference inventory (7 active files, 9 historical plan files excluded as immutable); expanded Unit 3 from 3 files to 7 with line-level citations. Per Phase 0 Pin pattern (core memory current_priorities) and review-guide.md § Orthogonality.
