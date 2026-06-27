---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
title: "fix(skills): apply_load_safety_check coherence guard — allowlist ↔ required_tools"
plan_type: fix
date: 2026-06-27
issue: senara-solutions/mika#1576
branch: fix/1576/skills-apply-load-safety-check-coherence
product_contract_source: ce-plan-bootstrap
origin: "GitHub issue mika#1576 (architect-groomed, session f6971f73, second-pass Verdict: GROOMED)"
---

# fix(skills): apply_load_safety_check coherence guard — allowlist ↔ required_tools

> **Product Contract preservation:** This plan is sourced from an architect-groomed GitHub issue (mika#1576), not a ce-brainstorm requirements doc. The issue body (with architect Resolutions F1 + F2) is the contract. Scope is preserved verbatim; the only enrichment below is HOW (real code grounding from repo research). One **carried correction** to F1's illustrative tool-name list is documented in Key Technical Decisions — it completes F1's plain intent (which explicitly delegated exact names to code investigation) and does not overturn its design.

---

## Summary

Add a **runtime, load-time coherence check** to the mika-agent skill registry: after an agent's identity allowlist and DB/transient overrides have been applied, verify that every `[constraints] required_tools` token in every loaded skill resolves to a tool that is actually in the agent's effective tool surface (engine builtins + tools declared by the agent's *loaded* skills). A token that resolves to nothing is a **silent vacuous pass** today — the existing `required_tools` enforcement (mika#516, per-turn, keyword-scoped) only fires when the skill keyword-matches *and* the LLM is asked to use the tool, so a structurally-broken allowlist↔required_tools pairing surfaces only mid-work when the agent reaches for a tool that isn't there.

On a fire: **skip the broken skill** with an error-level structured log event `required_tool_unresolvable`, surface it in `mika skills validate`, and let the agent start degraded (the established `apply_load_safety_check` "load with warning + skip broken skill" pattern — **not** refuse-to-start). The check closes the silent-pass risk at the one layer where both operands (the allowlist and the skill's `required_tools`) are simultaneously visible — the load/apply path.

This is the runtime sibling of mika#1575's build-time check 5 (identity coherence). It shares the same builtin-tool-name surface (`BUILTIN_TOOL_NAMES ∪ KNOWN_BUILTINS`) so the two checks never disagree about what counts as a builtin.

---

## Problem Frame

**The invariant:** an agent must not hold a loaded skill that requires a tool the agent cannot call.

**The current gap (counter `invariant-enforced-at-dispatch-layer-not-load-layer n=1`):** the coherence between two pieces of config — the agent's `identity.toml [skills].allowlist` and a skill's `skill.toml [constraints] required_tools` — is only checked where one operand is visible at a time:

- `collect_required_tools()` (`crates/mika-agent/src/agent_loop/mod.rs:4581–4615`) gathers `required_tools` from **keyword-matched skills only** (`MatchReason::Keyword`), per turn.
- The EndTurn post-condition (mika#516) checks the agent actually *called* each required tool — but it allows a vacuous pass when a terminal tool error is detected, and it never fires at all for a skill that is loaded-but-not-keyword-matched this turn.

**Motivating incident (mika#1406 / PR #1570):** Prime's allowlist swap `github` → `gh-read-only` removes the `github` skill that *provides* the `run_gh` skill-tool binding to a given agent. A loaded skill whose `required_tools` references the (now-absent) provider would pass every existing check vacuously and fail silently only when the agent reaches for the tool. Per Mika Prime's bearing-read: *"A silent failure gated on a human remembering a PR-body note is precisely the shape structural enforcement exists for."*

**Where the fix belongs:** the load/apply path (`SkillRegistry::apply_load_safety_check`), which runs *after* allowlist + overrides and therefore sees both operands at once.

---

## Requirements

Traced from mika#1576 body (acceptance criteria) + architect Resolutions F1/F2.

- **R1** — Extend the canonical load-time safety check (`SkillRegistry::apply_load_safety_check` in `crates/mika-agent/src/skills/mod.rs`) with the allowlist↔required_tools coherence rule. (AC1)
- **R2** — Emit an error-level structured log event `required_tool_unresolvable` when the check fires, with fields `agent_id`, `skill`, `unresolvable_token`, `available_tool_count`. (AC2)
- **R3** — On fire: skip the offending skill (evict from loaded set, record in `skipped`), agent continues to start degraded. Load-with-warning, **not** refuse-to-start. (AC2, F2)
- **R4** — Tests covering: PASS (all tokens resolve, no fire), FAIL (a genuinely-unresolvable token fires → skill skipped + event), edge (builtin tokens always resolve with no skill declaration needed). (AC3)
- **R5** — `mika skills validate` reports the coherence diagnostic when it would fire. (AC4)
- **R6** — Documentation describing the coherence invariant and how it composes with the other layers (mika#516 / #1326 / #1575). (AC5)
- **R7 (F1)** — "Builtin tool name" = the union of `KNOWN_BUILTINS` (`builtin_handlers.rs`) and the engine-default `ToolRegistry` tool names — **the same set mika#1575's check uses**. Sourced from real code, not the contract's illustrative list.
- **R8 (F2, verification gate)** — Before opening the PR, run the coherence check against **all** well-known agent identity templates (mika-dev, mika-qa, mika-relay, mika-arch) and confirm **zero fires**. Any fire is a pre-existing bug surfaced by the detector and is fixed **in this PR**, not deferred.

---

## Key Technical Decisions

### KTD-1 — Builtin surface = `BUILTIN_TOOL_NAMES ∪ KNOWN_BUILTINS` (carried correction to F1)

F1's prose lists an illustrative `KNOWN_BUILTIN_FUNCTIONS` of 7 names (`run_shell, web_search, get_documentation, run_claude_pilot, run_claude_pilot_groom, run_gh, run_mcp_tool`). **Repo research found this list is factually wrong against HEAD:**

- The real const is `KNOWN_BUILTINS` (`crates/mika-agent/src/skills/builtin_handlers.rs:39–47`) = `["get_documentation", "gh_read", "git_ops", "review_skill", "run_gh", "run_gws", "web_search"]`.
- The authoritative engine-tool surface is `BUILTIN_TOOL_NAMES` (`crates/mika-agent/src/tools/mod.rs:716–775`), 46 names = `default_tools()` ∪ `management_tools_if_needed()` ∪ `KNOWN_BUILTINS`. The `test_builtin_tool_names_parity` test (mika#1217 F4) guards that this const stays in sync.
- Names from F1's list like `run_shell`, `run_claude_pilot`, `run_claude_pilot_groom`, `run_mcp_tool` are **skill-provided** tools (declared in a skill's `tools.json`), not engine builtins.

**Decision:** the coherence check's builtin base = `BUILTIN_TOOL_NAMES` (which already subsumes `KNOWN_BUILTINS` via the parity test). This is *exactly* the set mika#1575's build-time check consumes, satisfying F1's binding requirement ("the same set"). Using the real constants **completes** F1's plain intent — F1 explicitly instructed "investigate the actual code… do not assume the lists above are exhaustive" and pinned the definition to "mika#1575 check 4 option (a)". This is a carry-not-route call per the implementer-contradiction doctrine: it finishes a resolution's stated intent rather than overturning its design.

Include the **full** `BUILTIN_TOOL_NAMES` (including the conditionally-injected management tools) in the builtin base. The static mika#1575 checks do the same, and treating conditional management tools as always-builtin avoids false-positive fires for agents where those tools are conditionally present. Document this choice inline.

### KTD-2 — Effective surface is allowlist-AWARE (runtime sibling of #1575 check 5, not check 4)

The check runs over `self.skills` *after* `apply_identity_allowlist` + `apply_overrides` + transient overrides. The effective tool surface = `BUILTIN_TOOL_NAMES` ∪ { every tool name declared in the `tools.json` of each **loaded** skill }. This is allowlist-aware (only loaded skills contribute), mirroring mika#1575's check 5 (`known_tools_builtins_only()` + reachable-through-allowlist), **not** check 4 (which is allowlist-unaware and adds `all_bundled_tool_names()`). The shared piece between #1576 and #1575 is the *builtin* base (KTD-1); the skill-provided augmentation differs by design (runtime = loaded skills only).

### KTD-3 — Placement and signature

Implement as a distinct pass invoked immediately after `apply_load_safety_check()` in the call sequence (`crates/mika-cli/src/commands/ask.rs` and any other call site that builds a registry for a specific agent). Either (a) add a new method `SkillRegistry::apply_required_tools_coherence_check(&mut self, agent_id: &str)`, or (b) extend `apply_load_safety_check` to take `agent_id` and fold the rule in as a new phase. **Prefer (a)** — a separate, single-purpose method keeps `apply_load_safety_check`'s existing signature stable, is independently testable, and reads as its own concern. The new method needs `agent_id` for the log event (R2); `apply_load_safety_check` is currently `&mut self` with no agent_id in scope, so the call site must thread it through. Use the same Phase-1-collect / Phase-2-evict structure already used in `apply_load_safety_check` (lines 342–417) so eviction during iteration is safe.

### KTD-4 — FAIL-case token must be genuinely unresolvable (AC3 example correction)

AC3's literal FAIL example is `required_tools = ["run_gh"]` "fires because no allowlisted skill provides run_gh." But under KTD-1, `run_gh ∈ KNOWN_BUILTINS ⊂ BUILTIN_TOOL_NAMES`, so `run_gh` **always** resolves as a builtin and can never fire. The example token is inconsistent with F1's builtin definition. **Decision:** honor F1 (the BLOCKING, code-grounded resolution) over AC3's illustrative token. The FAIL-case test uses a token that is genuinely outside the effective surface — both (i) a synthetic non-existent token (`totally_unresolvable_tool`) for the pure unit test, and (ii) the realistic shape: a skill whose `required_tools` names a *skill-provided* tool whose providing skill is **excluded from the allowlist** (the actual mika#1406 motivating scenario, and the case mika#516's per-turn check misses). This completes AC3's plain intent (an unresolvable token fires) without contradicting F1.

### KTD-5 — Reuse the const directly; no new shared abstraction (KISS)

`BUILTIN_TOOL_NAMES` is already `pub` and parity-test-guarded. Both the runtime check (this ticket) and the build-time `verify_bundled_skills` binary reference the *same* const, so "use the same set" is satisfied without extracting a new helper. Do not introduce a shared `builtin_tool_surface()` function unless implementation reveals the assembly logic genuinely duplicates (it is a one-liner: collect the const into a set). Keep it simple.

---

## High-Level Technical Design

Load/apply sequence (call site: `crates/mika-cli/src/commands/ask.rs`), with the new pass appended:

```
SkillRegistry built from scan
        │
        ▼
apply_identity_allowlist(allowlist)   // Phase -1: evict non-allowlisted skills   (mod.rs:472–508)
        │
        ▼
apply_overrides(&overrides)           // Phase 0/1: evict DB-disabled, apply always_on/LLM (mod.rs:518–595)
        │
        ▼
apply_transient_disable / always_on   // Phase 1.5: CLI enable/disable
        │
        ▼
apply_load_safety_check()             // existing: skip broken handlers / tools.json / manifest (mod.rs:338–417)
        │
        ▼
apply_required_tools_coherence_check(agent_id)   // NEW (this ticket)
        │   ┌─────────────────────────────────────────────────────────────┐
        │   │ effective = BUILTIN_TOOL_NAMES ∪ {tool.name for each loaded   │
        │   │             skill's tools.json}                               │
        │   │ for each loaded skill S:                                      │
        │   │   for each token T in S.manifest.required_tools:             │
        │   │     if T ∉ effective:                                         │
        │   │        tracing::error!(required_tool_unresolvable, …)         │
        │   │        mark S for eviction → self.skipped                     │
        │   └─────────────────────────────────────────────────────────────┘
        ▼
log_summary()
```

Coherence rule (per loaded skill, after allowlist + overrides):

```
resolvable(token) ⟺ token ∈ BUILTIN_TOOL_NAMES
                  ∨ ∃ loaded skill L : token ∈ names(L.tools.json)
```

---

## Implementation Units

### U1. Add the coherence-check pass to `SkillRegistry`

**Goal:** implement `apply_required_tools_coherence_check(&mut self, agent_id: &str)` (KTD-3) that evicts loaded skills whose `required_tools` contain an unresolvable token, emitting `required_tool_unresolvable` per fire.

**Requirements:** R1, R2, R3, R7.

**Dependencies:** none.

**Files:**
- `crates/mika-agent/src/skills/mod.rs` — new method on `SkillRegistry` (place adjacent to `apply_load_safety_check`, ~line 417). Import `crate::tools::BUILTIN_TOOL_NAMES`.

**Approach:**
- Build `effective: HashSet<&str>` = `BUILTIN_TOOL_NAMES` collected, then extend with every tool name declared by each currently-loaded skill (read from the parsed `tools.json` / `skill_tools` on each `SkillEntry` — confirm the exact field name during implementation; research indicates `SkillEntry` carries resolved skill tools). Include the skill's own declared tools so a skill that both declares and requires a tool resolves.
- Two-phase like `apply_load_safety_check`: Phase 1 collect `(skill_name, unresolvable_token)` violations without mutating; Phase 2 `tracing::error!` each + `retain()` out the violating skills + push to `self.skipped`.
- Log fields exactly: `agent_id = %agent_id, skill = %skill_name, unresolvable_token = %token, available_tool_count = effective.len()`, message e.g. `"skill required_tools references a tool absent from the agent's effective surface — skipping skill"`. Match the house style in `skills/mod.rs:380–401` (snake_case fields, `%` display formatter, guidance to `mika skills validate`).
- Comment inline why the full `BUILTIN_TOOL_NAMES` (incl. conditional management tools) is used as the builtin base (KTD-1).

**Patterns to follow:** existing skip/evict + `tracing::warn!` block in `apply_load_safety_check` (`skills/mod.rs:380–417`); the two-phase collect-then-apply structure already there.

**Test scenarios** (inline `#[cfg(test)] mod tests` in `skills/mod.rs`, using `make_entry` / `SkillRegistry { … }` helpers at lines 833–871 / 1114–1119):
- **PASS:** a loaded skill with `required_tools = ["search_memory"]` (a builtin) and another with a token provided by a co-loaded skill's tools.json → no fire, both skills remain in `self.skills`, `self.skipped` empty.
- **FAIL (synthetic):** a loaded skill with `required_tools = ["totally_unresolvable_tool"]` and no skill providing it → skill evicted into `self.skipped`, exactly one violation. (KTD-4)
- **FAIL (realistic allowlist exclusion):** skill A requires a tool that only skill B's tools.json provides, but skill B is absent from the loaded set (simulating it being excluded by the allowlist) → A is evicted. Mirrors the mika#1406 scenario. (KTD-4)
- **Edge (builtin always resolves):** a loaded skill with `required_tools = ["run_gh"]` (∈ KNOWN_BUILTINS ⊂ BUILTIN_TOOL_NAMES) and **no** skill declaring `run_gh` → no fire. Directly asserts AC3's edge case and demonstrates why AC3's literal `run_gh` FAIL example was inconsistent. (R4)
- **Edge (empty required_tools):** loaded skill with `required_tools = []` → no fire.
- Assert the log event is emitted on fire (capture via `tracing-test` / `tracing_subscriber` if the crate already uses one for log assertions; otherwise assert on the `self.skipped` outcome + reason string, matching how existing skip tests verify behavior).

**Verification:** `cargo test -p mika-agent skills::` passes; new tests cover all four AC3 categories.

### U2. Thread `agent_id` into the call site(s)

**Goal:** invoke `apply_required_tools_coherence_check(agent_id)` immediately after `apply_load_safety_check()` wherever a registry is finalized for a specific agent.

**Requirements:** R1, R2.

**Dependencies:** U1.

**Files:**
- `crates/mika-cli/src/commands/ask.rs` — add the call after `apply_load_safety_check()` in the documented sequence.
- Any other registry-finalize site (grep for `apply_load_safety_check(` across the crate; the agent server startup path in `crates/mika-agent/src/server/` or `agent_loop` may build registries too). **Add the new call at every site that already calls `apply_load_safety_check`** so server-launched agents (mika-spirit) get the check, not just CLI.

**Approach:** `agent_id` is in scope at these call sites (the registry is being built *for* a named agent). Pass it through. If a call site builds a registry without a concrete agent (e.g., a generic validation context), pass the best-available identifier or a sentinel and note it — but prefer the real agent id everywhere an agent is being provisioned/loaded.

**Patterns to follow:** the existing ordering and call style at the `ask.rs` sequence (`apply_identity_allowlist` → `apply_overrides` → … → `apply_load_safety_check` → `log_summary`).

**Test scenarios:** `Test expectation: none — wiring only; behavior is covered by U1 unit tests and U4's well-known-agent integration test.` (If a call site is missed, U4's per-agent gate would not catch a runtime-only miss; manually confirm via grep that every `apply_load_safety_check` call has a sibling coherence call.)

**Verification:** `grep -rn "apply_load_safety_check(" crates/` — every hit has an adjacent `apply_required_tools_coherence_check(` call; `cargo build` clean.

### U3. Surface the diagnostic in `mika skills validate`

**Goal:** `mika skills validate` reports a coherence diagnostic when a skill's `required_tools` token would be unresolvable.

**Requirements:** R5.

**Dependencies:** U1.

**Files:**
- `crates/mika-cli/src/commands/skills.rs` — `validate_skills` (lines 1481–1636); add a diagnostic pass after the per-skill structural validation loop (~line 1569) and before the summary (~line 1616).

**Approach:**
- `mika skills validate` validates skills on disk (the bundled/installed surface) and is **not** inherently scoped to a single agent's allowlist. Scope the coherence diagnostic to what `validate` can know: resolve each skill's `required_tools` against `BUILTIN_TOOL_NAMES ∪ all-validated-skills' declared tools`. This is the build-time-style (allowlist-unaware) view appropriate for the CLI surface — it catches the structural class (a token resolving to *nothing anywhere*) even if it can't reproduce a specific agent's allowlist. State this scoping in the diagnostic message so the operator understands it is the disk-surface view, and that the per-agent runtime check (U1) is the allowlist-aware authority.
- Emit a `DiagnosticLevel::Fail` (or `Warn` — match the severity the existing `required_tools` structural diagnostics use; prefer `Fail` to align with mika#1575's pre-merge stance) entry per unresolvable token, rendered in all three output formats (Text / JSON / YAML) the command already supports (lines 1582–1615).

**Patterns to follow:** the existing diagnostic collection + `DiagnosticLevel` rendering in `validate_skills`; the diagnostic struct shape `{skill, level, message}`.

**Test scenarios:**
- A skill (in a temp skills dir) with `required_tools = ["totally_unresolvable_tool"]` → `validate` reports a Fail/Warn diagnostic naming the token; exit status reflects it (1 if Fail).
- A skill with only builtin / co-resolvable tokens → no coherence diagnostic.
- JSON output includes the diagnostic entry with the token in the message.

**Verification:** `cargo test -p mika-cli` passes; manual `cargo run --bin mika -- skills validate` on a crafted broken skill shows the line.

### U4. F2 verification gate — well-known agents pass clean (+ integration test)

**Goal:** prove (and lock in) that **all** well-known agent identity templates pass the coherence check with zero fires (R8). This is both the F2 implementer gate and a regression test.

**Requirements:** R8, R4.

**Dependencies:** U1, U2.

**Files:**
- `crates/mika-agent/src/well_known_agents.rs` (test module) **or** an integration test under `crates/mika-agent/tests/` — for each well-known agent (mika-dev: 26 skills @ lines 134–160; mika-qa: 17 @ 192–211; mika-relay: 1 `permission-policy`; mika-arch: 3 @ 298–302 via `build_mika_arch_identity`), build a `SkillRegistry` from the bundled skills, apply that agent's allowlist, run `apply_load_safety_check` + `apply_required_tools_coherence_check`, and assert `self.skipped` contains **no** coherence-class evictions.

**Approach:**
- Use the bundled skill manifests (`BUNDLED_SKILL_MANIFESTS`) as the source registry so the test reflects the real shipped surface.
- mika-arch's identity is computed via `build_mika_arch_identity()` and requires `MIKA_KG_DOCS_ROOTS`; in the test, supply a dummy value or construct the allowlist directly from `MIKA_ARCH_SKILL_ALLOWLIST` (lines 298–302) to avoid the env dependency — the allowlist is what the check consumes.
- **If any agent fires:** that is a pre-existing coherence bug surfaced by the detector. Per F2, fix it in this PR — either add the providing skill to the agent's allowlist or remove the dangling `required_tools` token from the offending skill. Document any such fix in the PR body. (Architect verified zero fires at grooming HEAD; this gate re-confirms at PR HEAD.)
- Watch specifically: `qa-review` (always_on) declares `required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]` — `run_gh` is a builtin; `run_shell`, `build_mika`, `qa_pr_view` must be provided by co-allowlisted skills (`shell-exec`, `build-mika`, and qa-review's own tools.json). mika-qa allowlists `build-mika` and `shell-exec`, so these should resolve; the gate confirms it.

**Test scenarios:**
- `test_well_known_agents_pass_coherence_check` — parametrized over the four agents; asserts zero coherence evictions for each.
- (If a fire is found and fixed) a focused test documenting the corrected pairing.

**Verification:** `cargo test -p mika-agent well_known` (or the integration test) passes for all four agents.

### U5. Documentation of the coherence invariant

**Goal:** document the invariant, the runtime check, and how it composes with the other three layers.

**Requirements:** R6.

**Dependencies:** U1.

**Files:**
- `docs/architecture/bundled-skill-verification.md` — extend (this is mika#1575's home and the natural place for the build-time↔runtime composition story), **or** a new `docs/solutions/best-practices/required-tools-coherence-runtime-check-2026-06-27.md` with YAML frontmatter (`module`, `tags`, `problem_type`, `category`). Prefer extending `bundled-skill-verification.md` so the four-layer composition lives in one place; cross-link from a short solutions entry if the doc-audit step wants a searchable record.
- `crates/mika-agent/docs/` fallback copy if the doc lives under `docs/` and is `include_str!`-synced — run `scripts/sync-agent-docs.sh` if applicable (CI `docs-sync` job enforces this for `docs/` changes consumed by `build.rs`).

**Approach:** describe the invariant ("an agent must not hold a loaded skill requiring a tool it can't call"), the builtin definition (KTD-1, shared with mika#1575), the allowlist-aware effective surface (KTD-2), the fire disposition (skip + log + degraded start), and the four-layer composition table:

| Layer | When | Scope | Ticket |
|-------|------|-------|--------|
| Availability filter | runtime, per-turn | LLM-call time | mika#516 |
| Cross-skill name collisions | build-time test | bundled surface | mika#1326 AC2 |
| `make verify-bundled-skills` | build-time / pre-merge | structural, allowlist-aware (check 5) | mika#1575 |
| **required_tools coherence** | **runtime, load-time** | **per-agent effective surface** | **mika#1576 (this)** |

**Test scenarios:** `Test expectation: none — documentation.` Doc-sync verified by CI if applicable.

**Verification:** doc renders; if synced, `scripts/sync-agent-docs.sh` leaves no diff; CI `docs-sync` green.

---

## Scope Boundaries

**In scope:** the runtime load-time coherence check, its log event, CLI surface, tests, the F2 well-known-agent gate (including fixing any pre-existing fire it surfaces), and documentation.

### Deferred to Follow-Up Work
- Refactoring `required_tools` to be data-driven from the skill registry instead of declared per-skill (out of scope per issue body).
- Cross-agent coherence (each agent's check is independent — per issue body).

### Outside this change's identity
- Backfilling existing skill prompts to fix coherence violations **other than** any fire surfaced by the U4 verification gate. The gate-surfaced fire is fixed here (F2); speculative/other violations are per-incident follow-ups.
- Changing mika#516 / #1326 / #1575 behavior. Those layers stay exactly as-is; this adds a fourth.
- Any operator action on Prime's allowlist swap (`github` → `gh-read-only`). This ticket *unblocks* that action structurally; performing it is a separate operator-gated step (issue body § Sequencing).

---

## Risks & Dependencies

- **False-positive fire skipping a legitimate skill.** Mitigation: builtin base is the full parity-guarded `BUILTIN_TOOL_NAMES` (KTD-1); effective surface includes the skill's own declared tools; U4 proves all four shipped agents pass clean before merge.
- **Missed call site** (a registry finalized without the new pass → no runtime coverage for that path). Mitigation: U2's grep verification; add the call at *every* `apply_load_safety_check` site.
- **`SkillEntry` tool-name field uncertainty.** The exact field exposing a loaded skill's declared tool names (`skill_tools` vs parsed `tools.json`) must be confirmed in code during U1 — research indicates `SkillEntry` carries resolved skill tools but the precise accessor needs verification.
- **Severity choice in `mika skills validate`** (Fail vs Warn) — align with the existing `required_tools` structural diagnostic severity; prefer Fail to match mika#1575's pre-merge stance.
- **Dependency:** none blocking. The substrate bug that wedged the autonomous dev-groom for this ticket (#1593) is being fixed in parallel; this plan is dispatched via direct-implementation mode and does not depend on #1593.

---

## Acceptance criteria

- [ ] **AC1** — `SkillRegistry::apply_load_safety_check` (or a dedicated sibling method) includes the allowlist↔required_tools coherence rule, running after identity allowlist + DB/transient overrides have been applied. (R1)
- [ ] **AC2** — On fire: emit an error-level structured log event `required_tool_unresolvable` (fields: `agent_id`, `skill`, `unresolvable_token`, `available_tool_count`) and skip the offending skill (evict from loaded set, agent starts degraded — load-with-warning, **not** refuse-to-start). (R2, R3)
- [ ] **AC3** — Tests covering: PASS (all tokens resolve), FAIL (genuinely-unresolvable token → skill skipped + event), edge (builtin tokens always resolve with no skill declaration needed). (R4)
- [ ] **AC4** — `mika skills validate` reports the coherence diagnostic when it would fire. (R5)
- [ ] **AC5** — Documentation describing the coherence invariant and how it composes with mika#516 / #1326 / #1575. (R6)
- [ ] **AC6** — "Builtin tool name" = `BUILTIN_TOOL_NAMES ∪ KNOWN_BUILTINS` — the same set mika#1575's check uses. (R7 / F1)
- [ ] **AC7** — F2 verification gate: before opening the PR, run the coherence check against all well-known agent identity templates (mika-dev, mika-qa, mika-relay, mika-arch) and confirm zero fires. Any fire is fixed in this PR, not deferred. (R8)
- [ ] **AC8** — `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check` clean; `cargo run --bin verify-bundled-skills` still passes; every `apply_load_safety_check` call site has an adjacent coherence-check call; PR body documents the F1 carried correction (KTD-1) and the AC3 token correction (KTD-4), and `Closes #1576`.

---

## Sources & Research

- **Issue contract:** mika#1576 body + comment `mika-arch second-pass (GROOMED)` session `f6971f73-4e24-4743-bc75-9c50ea58181e` (Resolutions F1, F2).
- **`apply_load_safety_check`** — `crates/mika-agent/src/skills/mod.rs:338–417`; `apply_identity_allowlist` 472–508; `apply_overrides` 518–595; call sequence in `crates/mika-cli/src/commands/ask.rs`.
- **Builtin surface (KTD-1)** — `KNOWN_BUILTINS` `crates/mika-agent/src/skills/builtin_handlers.rs:39–47`; `BUILTIN_TOOL_NAMES` `crates/mika-agent/src/tools/mod.rs:716–775`; parity test `test_builtin_tool_names_parity` (mika#1217 F4).
- **Existing enforcement** — `collect_required_tools()` `crates/mika-agent/src/agent_loop/mod.rs:4581–4615` (keyword-scoped, per-turn); EndTurn post-condition + terminal-failure bypass (mika#516).
- **`required_tools` manifest field** — `crates/mika-agent/src/skills/manifest.rs:101` (`[constraints]`).
- **`tools.json`** — parsed via `ToolDecl` in `crates/mika-agent/src/skills/index.rs`; example `skills/bundled/dev-groom/tools.json`; `qa-review` `required_tools` example `skills/bundled/qa-review/skill.toml`.
- **mika#1575** — `crates/mika-agent/src/bin/verify_bundled_skills.rs` (check 4 builtin assembly lines 468–475; check 5 builtin-only set lines 490–495); `docs/architecture/bundled-skill-verification.md`; CI `make verify-bundled-skills`.
- **Well-known agents** — `crates/mika-agent/src/well_known_agents.rs` (mika-dev 134–160, mika-qa 192–211, mika-arch allowlist 298–302, `build_mika_arch_identity` 310–366).
- **CLI** — `mika skills validate` `crates/mika-cli/src/commands/skills.rs:1481–1636`.
- **Logging house style** — `crates/mika-agent/src/skills/mod.rs:57–61, 380–401`.
- **Test harness** — `crates/mika-agent/src/skills/mod.rs` test module 827–1149 (`make_entry` 833–871; registry construction 1114–1119).
