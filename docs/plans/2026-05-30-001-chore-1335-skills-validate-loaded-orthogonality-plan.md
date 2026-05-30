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

**Files (6 callsites):**
- `crates/mika-cli/src/commands/ask.rs:335` — `skill_registry.validate_loaded()` → `skill_registry.apply_load_safety_check()`
- `crates/mika-cli/src/commands/chat.rs:128` — same rename
- `crates/mika-agent/src/server/mod.rs:415` — same rename
- `crates/mika-agent/src/tools/list_skills.rs:59` — same rename
- Any hot-reload sites in `server/handlers.rs` and `server/a2a.rs` that rebuild the registry

**Mechanical:** Find-and-replace `validate_loaded()` → `apply_load_safety_check()` at each callsite. No logic changes.

### Unit 3: Update solution docs and cross-references

**Goal:** Update all documented references to `validate_loaded()` to use the new name and framing.

**Files:**
- `docs/solutions/best-practices/agent-tool-must-call-validate-loaded-on-skill-registry.md` — rename file to `agent-tool-must-call-apply-load-safety-check-on-skill-registry.md`, update all internal references
- `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` — update references
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — update references

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
6 callsites ← mechanical rename
```

No behavioral changes. The decision matrix, the skip-worthy classification, and the two-phase collect-then-apply pattern are all unchanged.

## Risks

| Risk | Mitigation |
|------|------------|
| Rename breaks external consumers | `validate_loaded()` is `pub` on `SkillRegistry` but the crate is not published to crates.io as a library. All consumers are in-tree. `cargo build` catches any missed callsites. |
| Doc references go stale | Unit 3 covers all known doc references. `grep -r validate_loaded` after implementation confirms none are missed. |
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
