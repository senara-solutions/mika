---
title: "feat(mika-arch): gh_read tool + agent provisioning + two bundled skills (v1 Units 2-6)"
type: feat
status: active
date: 2026-04-25
origin: mika-platform branch chore/mika-arch-v1-plan — docs/plans/2026-04-25-001-feat-mika-arch-v1-plan.md
depth: deep
---

# feat(mika-arch): gh_read tool + agent provisioning + two bundled skills (v1 Units 2-6)

## Overview

Stand up `mika-arch` as the fourth well-known agent: a read-only architect that reviews plans before code is written. This PR lands four code units in one atomic change: a new `gh_read` builtin handler, the mika-arch agent definition with identity-driven skill allowlists, and two bundled review skills (groom-ticket on Opus 4.7, second-review on Sonnet 4.6). The identity-driven allowlist is a cross-cutting SOC fix that replaces `skill_overrides` DB row writes for well-known agents with `[skills].allowlist` in `identity.toml`.

## Problem Frame

Mika's team today has mika-dev (executor) and mika-qa (PR reviewer). Plan-stage architectural review is implicit — no agent's job is to push back on plans before code writes happen. Plans land with inconsistencies that surface too late at `/ce:review` time. mika-arch closes this gap with citation-grounded pushback at the plan stage.

The second problem this PR addresses: skill provisioning for well-known agents is split between Rust seed code (`well_known_agents.rs`) and `skill_overrides` DB rows, creating drift that operators manually fix. The D2 SOC fix moves skill ownership into `identity.toml` where it belongs.

## Requirements Trace

- **R2.** Agent identity, soul, and base model defined per the cross-repo plan §Unit 3.
- **R6.** `gh_read` builtin handler — four read-only ops with structured errors (see origin: cross-repo plan §Unit 2).
- **R7.** Multi-corpus KG config via `[kg].docs_roots` in identity.toml (four repos).
- **R11.** No third pass — second-review produces GROOMED or ESCALATE only.
- **R12.** Skills reference `docs/architecture/review-guide.md` (Unit 1, already shipped).
- **R15.** GitHub App reuse for `gh_read` auth — no new machine user.
- **R16.** Identity-driven skill provisioning via `[skills].allowlist` in identity.toml; no `skill_overrides` rows for well-known agents.

## Scope Boundaries

- **In scope:** `gh_read` builtin handler, mika-arch agent provisioning, identity allowlist mechanism, two bundled skills.
- **NOT in scope:** Gateway escalation (Unit 7), cost monitoring (Unit 8), E2E dogfood (Unit 5 — operational).
- **NOT in scope:** `[llm]` section revival in skill.toml. Per-skill LLM overrides use the existing DB-backed `skill_overrides.llm_provider/llm_model` path.

### Deferred to Separate Tasks

- Cross-cutting D2 migration of mika-dev/mika-qa/mika-relay to `[skills].allowlist`: File as follow-up ticket. This PR introduces the allowlist mechanism for mika-arch only. Existing agents continue using `disabled_skills` denylist + DB rows until the follow-up lands.
- OpenRouter prompt variants for skills (per plan D8): v1 hardcodes Anthropic-direct.
- Cost-monitoring dashboard panels: log fields land in v1, dashboard deferred.
- Per-skill `[llm]` section in skill.toml: removed in #504. Per-skill model selection for mika-arch's two skills uses identity-driven seeding into `skill_overrides` DB rows (see Unit 3 approach below).

## Context & Research

### Relevant Code and Patterns

- **`crates/mika-agent/src/skills/builtin_handlers.rs`** — `KNOWN_BUILTINS` array, `execute()` dispatch, `run_gh()` handler pattern (allowlist, `validate_gh_input()`, `parse_command_array()`, env scrubbing, `spawn_and_collect()`). `gh_read` mirrors this exactly.
- **`crates/mika-agent/src/well_known_agents.rs`** — `WellKnownAgent` struct with `disabled_skills`, `provision_well_known_agents()` (filesystem phase), `seed_well_known_skill_overrides()` (DB phase). Tests cover provisioning, skill overrides, relay superset invariant.
- **`crates/mika-agent/src/prompt.rs`** — `Identity` struct with `KgIdentityConfig`. `load_identity()` reads `identity.toml` with serde defaults. The `[skills]` block will be added here.
- **`crates/mika-agent/src/skills/mod.rs`** — `SkillRegistry.apply_overrides()` has Phase 0 (evict disabled) and Phase 1 (apply always_on/LLM). Allowlist eviction will be a new Phase -1 before existing phases.
- **`crates/mika-agent/src/skills/manifest.rs`** — `SkillManifest` has `llm: LlmOverride` with `#[serde(skip)]`. `[llm]` section removed from skill.toml in #504. Per-skill LLM is DB-only via `skill_overrides`.
- **`skills/bundled/qa-review/`** — Review-class bundled skill pattern. `skill.toml` with triggers, constraints, context. `system_prompt.md` with structured process/output/constraints sections.
- **`skills/bundled/skill-review/tools.json`** — Only existing bundled skill with `"type": "builtin"` handler reference.
- **`crates/mika-agent/build_support/bundled_skills_discover.rs`** — Build-time discovery walks `skills/bundled/*/`, requires `skill.toml`. No registration code changes needed.

### Institutional Learnings

- **`docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md`** — Two-phase provisioning (filesystem then DB). Phase 1 writes identity/soul before DB; Phase 2 runs inside `init_agent()` after `seed_bundled_skills`. First-creation-only skill overrides to preserve customization.
- **`docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md`** — Disabled = evicted from registry. Identity allowlist should follow the same eviction pattern.
- **`docs/solutions/architecture-patterns/skill-llm-override-keyword-filter.md`** — `resolve_skill_llm_override()` must filter to `MatchReason::Keyword` only. Never mark skills with LLM overrides as `always_on`.
- **`docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md`** — `gh_read` structured errors must be co-designed with skill prompts. Each error variant needs a prompt branch.
- **`docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md`** — `gh_read` name must not collide with existing builtins. No collision detection at dispatch — shadowing is silent.
- **`docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`** — Skills needing GitHub data should use `[context]` declarations.
- **`docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md`** — New bundled skills with builtin handler tools must have the handler in `KNOWN_BUILTINS` or validation evicts them silently.

### External References

None required. All patterns are well-established in the codebase.

## Key Technical Decisions

### D1. `gh_read` is a builtin handler, not a `Tool` trait impl

The issue assumed `gh_read` would be a standalone file at `tools/gh_read.rs`. Research reveals that `run_gh` is a **builtin handler** in `builtin_handlers.rs`, dispatched by the skill system — not a `Tool` trait impl registered in `default_tools()`. `gh_read` follows the same pattern: handler function in `builtin_handlers.rs`, referenced by `tools.json` in the skills that need it.

This means:
- No new file at `tools/gh_read.rs` — handler lives in `builtin_handlers.rs`
- No registration in `default_tools()` — registration is via `KNOWN_BUILTINS` array + `execute()` match arm
- Skills declare the tool in their `tools.json` with `"handler": {"type": "builtin", "function": "gh_read"}`

### D2. Per-skill LLM override via identity-driven DB seeding (not skill.toml)

The issue assumed `[llm]` sections in skill.toml. But `[llm]` was removed from skill.toml in #504 — the field on `SkillManifest` is `#[serde(skip)]`. Per-skill LLM overrides are DB-only via `skill_overrides.llm_provider/llm_model`.

For mika-arch's two skills (Opus for groom-ticket, Sonnet for second-review), the approach is:
- `seed_well_known_skill_overrides()` gains a new concept: `llm_overrides` on `WellKnownAgent` — a list of `(skill_name, provider, model)` tuples.
- On first creation (no existing overrides), the function seeds both the enabled/disabled state AND the LLM provider/model for mika-arch's skills.
- This composes cleanly with the existing `apply_overrides()` Phase 1 which already reads `llm_provider`/`llm_model` from DB.

This is a pragmatic middle ground: the identity allowlist controls which skills are active (Phase -1 eviction), while DB rows control LLM routing (Phase 1). Both are seeded by `well_known_agents.rs` on first creation.

### D3. Identity allowlist scope: mika-arch only in this PR

The cross-cutting D2 fix (migrate all well-known agents to identity allowlist) is deferred to a follow-up ticket. This PR:
- Adds the `[skills]` block to the `Identity` struct
- Implements `apply_identity_allowlist()` as Phase -1 in the override chain
- Uses it for mika-arch only (identity.toml has `[skills].allowlist`)
- Leaves mika-dev/mika-qa/mika-relay on the existing `disabled_skills` denylist path

### D4. `gh_read` uses operation-level allowlist, not subcommand-level

Unlike `run_gh` which allows broad subcommands (`pr`, `issue`, `run`, etc.), `gh_read` uses a stricter operation-level allowlist: `issue_view`, `pr_view`, `pr_diff`, `issue_list`. The `op` parameter maps directly to `gh` CLI commands:
- `issue_view` → `gh issue view <number> --json ...`
- `pr_view` → `gh pr view <number> --json ...`
- `pr_diff` → `gh pr diff <number>`
- `issue_list` → `gh issue list ...`

This means the input schema is `{"op": "issue_view", "target": "123", "repo": "owner/repo"}` rather than `{"command": ["issue", "view", "123"]}`. Simpler for the LLM, more constrained for security.

### D5. `gh_read` tools.json shared between both skills

Both mika-arch skills need `gh_read` access. Rather than duplicating `tools.json` in both skill directories, each skill directory gets its own `tools.json` referencing the same builtin function. This follows the existing pattern where skills own their tool declarations.

## Open Questions

### Resolved During Planning

- **Where does `gh_read` live?** → Builtin handler in `builtin_handlers.rs`, not a standalone tool file (D1).
- **How do skills get per-skill LLM models?** → DB-seeded via `seed_well_known_skill_overrides()` (D2). `[llm]` section removed from skill.toml in #504.
- **Should cross-cutting D2 land in this PR?** → No, deferred to follow-up (D3). Scope risk too high.
- **What input schema for `gh_read`?** → Operation-level (`op` + `target` + `repo`), not raw command array (D4).

### Deferred to Implementation

- Exact `gh` CLI flags for each operation (e.g., which `--json` fields for `issue_view`). Determine at implementation time by testing actual `gh` output.
- Whether `pr_diff` needs `--patch` vs default unified diff format.
- Exact soul.md content for mika-arch — the spec is an uncommitted local doc. Will paraphrase from the plan's Problem Frame section and review guide preamble.
- Whether `gh_read` should have a `tools.json` in a standalone skill (like `github/`) or only be declared in the mika-arch skills. Implementation will determine based on whether other agents should have access.

## Output Structure

```
skills/bundled/
├── mika-arch-groom-ticket/
│   ├── skill.toml
│   ├── system_prompt.md
│   └── tools.json              # gh_read builtin reference
└── mika-arch-second-review/
    ├── skill.toml
    ├── system_prompt.md
    └── tools.json              # gh_read builtin reference

crates/mika-agent/src/
├── skills/
│   └── builtin_handlers.rs     # MODIFY — add gh_read handler
├── well_known_agents.rs        # MODIFY — add MIKA_ARCH + LLM override seeding
├── prompt.rs                   # MODIFY — add SkillsIdentityConfig to Identity
└── skills/
    └── mod.rs                  # MODIFY — add apply_identity_allowlist()
```

## Implementation Units

- [ ] **Unit 1: `gh_read` builtin handler**

**Goal:** Add a read-only GitHub CLI handler with four operations and structured error responses.

**Requirements:** R6, R15.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs` (add `gh_read` handler, `GH_READ_ALLOWED_OPS`, `GhReadArgs`, `validate_gh_read_input()`, structured error enum, audit logging)
- Test: `crates/mika-agent/src/skills/builtin_handlers.rs` (inline `#[cfg(test)]` tests)

**Approach:**

Input schema: `{"op": "<operation>", "target": "<number_or_filter>", "repo": "owner/repo"}`. The `op` field is validated against `GH_READ_ALLOWED_OPS: &[&str] = &["issue_view", "pr_view", "pr_diff", "issue_list"]`. Each op maps to a specific `gh` CLI invocation:

| Op | gh command | Notes |
|---|---|---|
| `issue_view` | `gh issue view <target> --repo <repo> --json number,title,body,labels,milestone,comments` | Target required |
| `pr_view` | `gh pr view <target> --repo <repo> --json number,title,body,labels,headRefName,state` | Target required |
| `pr_diff` | `gh pr diff <target> --repo <repo>` | Target required |
| `issue_list` | `gh issue list --repo <repo> [--milestone <target>] [--label <target>]` | Target optional |

Structured error variants (returned as JSON strings in `ToolOutput::error`):
- `NotFound` — gh exits with "no such issue/PR" pattern
- `AuthFailed` — gh exits with auth failure / 401 pattern
- `RateLimited` — gh exits with 429 / rate limit pattern; parse `Retry-After` if present
- `NetworkError` — gh subprocess crash, timeout, or connection failure
- `MalformedRequest` — op outside allowlist, missing required `target`, missing `repo`

Audit logging: `tracing::info!` with event name `gh_read_invocation` and fields `agent_id` (from `ctx`), `op`, `resource` (target or filter), `latency_ms`, `status` (ok/error variant name). Uses existing `tracing` span context for `trace_id`.

Pattern: Follow `run_gh` exactly — `parse_command_array` is NOT reused (different input schema). Build the `gh` command array from validated `op` + `target` + `repo`. Use `scrub_mika_env_vars`, inject `GH_TOKEN` from `ctx.github_token`, call `spawn_and_collect`.

**Patterns to follow:**
- `builtin_handlers.rs` — `run_gh()` structure, `validate_gh_input()`, `spawn_and_collect()`, env scrubbing pattern.
- `KNOWN_BUILTINS` array and `execute()` match arm.

**Test scenarios:**
- Happy path: `issue_view` with valid target and repo returns structured JSON output.
- Happy path: `pr_diff` with valid target returns unified diff string.
- Edge case: `issue_list` with no target (no filter) succeeds.
- Edge case: `issue_list` with a milestone target succeeds.
- Edge case: op outside allowlist (e.g., `issue_create`) → `MalformedRequest` error, no subprocess spawned.
- Edge case: missing `repo` parameter → `MalformedRequest` error.
- Edge case: missing `target` for `issue_view` → `MalformedRequest` error.
- Error path: mocked subprocess exit indicating "no such issue" → `NotFound` variant.
- Error path: mocked subprocess exit indicating auth failure → `AuthFailed` variant.
- Integration: `"gh_read"` appears in `KNOWN_BUILTINS` and dispatches correctly via `execute()`.

**Verification:**
- All test scenarios pass (`cargo test -p mika-agent builtin_handlers::tests::gh_read`).
- `"gh_read"` is in `KNOWN_BUILTINS` and has a match arm in `execute()`.
- No write ops are possible even if requested.

---

- [ ] **Unit 2: Identity struct `[skills]` block + `apply_identity_allowlist()`**

**Goal:** Add `[skills].allowlist` deserialization to the `Identity` struct and implement Phase -1 allowlist eviction in `SkillRegistry`.

**Requirements:** R16.

**Dependencies:** None (independent of Unit 1).

**Files:**
- Modify: `crates/mika-agent/src/prompt.rs` (add `SkillsIdentityConfig` struct, add `skills` field to `Identity`)
- Modify: `crates/mika-agent/src/skills/mod.rs` (add `apply_identity_allowlist()` method on `SkillRegistry`)
- Test: `crates/mika-agent/src/prompt.rs` (inline test for deserialization)
- Test: `crates/mika-agent/src/skills/mod.rs` (inline test for allowlist eviction)

**Approach:**

New struct in `prompt.rs`:
```
SkillsIdentityConfig {
    allowlist: Option<Vec<String>>,
}
```

Added to `Identity` as `#[serde(default)] pub skills: SkillsIdentityConfig`. When `allowlist` is `None` or `Some(empty vec)`, all skills remain enabled (backward compatible).

New method `SkillRegistry::apply_identity_allowlist(&mut self, allowlist: &[String])`:
- Runs **before** `apply_overrides()` (Phase -1).
- If `allowlist` is non-empty: evict all skills whose name is NOT in the allowlist (case-insensitive). Evicted skills go to `self.disabled` with the same `DisabledSkill` pattern as Phase 0.
- Log each eviction at INFO level with `skill=<name>, reason="identity_allowlist"`.
- If `allowlist` is empty: no-op (all skills stay).

The call site is in `init_agent()` (or wherever `apply_overrides` is called) — load identity, check for allowlist, call `apply_identity_allowlist` before `apply_overrides`.

**Patterns to follow:**
- `KgIdentityConfig` deserialization pattern in `prompt.rs`.
- `apply_overrides()` Phase 0 eviction pattern — `retain()` + staging vec for `self.disabled`.

**Test scenarios:**
- Happy path: identity.toml with `[skills].allowlist = ["skill-a", "skill-b"]` deserializes correctly; `SkillsIdentityConfig.allowlist` is `Some(vec)` with two entries.
- Happy path: identity.toml with no `[skills]` section → `allowlist` is `None`.
- Happy path: `apply_identity_allowlist` with `["skill-a"]` evicts `skill-b` and `skill-c` but keeps `skill-a`.
- Happy path (regression): empty allowlist or no allowlist → all skills remain.
- Edge case: allowlist references a skill that doesn't exist in the registry → no crash, just no match (the named skill is absent from the active set; logged as warn at startup per Unit 3).
- Edge case: case-insensitive matching — `["Skill-A"]` matches `skill-a`.
- Integration: `apply_identity_allowlist` runs before `apply_overrides` and DB overrides still apply on surviving skills.

**Verification:**
- `cargo test -p mika-agent` passes.
- An agent with `[skills].allowlist = ["x"]` in identity.toml ends up with only skill `x` in the registry after the full override chain.

---

- [ ] **Unit 3: mika-arch well-known agent definition + provisioning**

**Goal:** Add `mika-arch` as the fourth well-known agent with identity, soul, base model, and LLM override seeding for its two skills.

**Requirements:** R2, R7, R16.

**Dependencies:** Unit 1 (`gh_read` exists), Unit 2 (identity allowlist mechanism), Unit 4 & Unit 5 (skills exist to reference in the allowlist — but the provisioning code can be written first; skills just need to exist by build time).

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (add `MIKA_ARCH` static, `MIKA_ARCH_SOUL`, `MIKA_ARCH_CONFIG`, `LlmOverrideSpec` struct, update `seed_well_known_skill_overrides()`)
- Test: `crates/mika-agent/src/well_known_agents.rs` (inline tests)

**Approach:**

New `WellKnownAgent` constant `MIKA_ARCH`:
- `name: "mika-arch"`, `display_name: "Architect"`, `emoji: "🏛"` (or similar architecture emoji)
- `soul: MIKA_ARCH_SOUL` — advisory architect persona, read-only, citation-or-silence discipline
- `disabled_skills: &[]` — empty, because mika-arch uses identity allowlist instead of denylist
- `config_toml: Some(MIKA_ARCH_CONFIG)` — sets Kimi as base model: `llm_provider = "openrouter"`, `openrouter_model = "moonshotai/kimi-k2.5"`

Identity.toml content for mika-arch (written by `provision_well_known_agents`):
```toml
name = "Architect"
emoji = "🏛"

[kg]
enabled = true
docs_roots = [
  "mika/docs/solutions",
  "mika-platform/docs/solutions",
  "mika-skills/docs/solutions",
  "mika-cloud/docs/solutions",
]

[skills]
allowlist = ["mika-arch-groom-ticket", "mika-arch-second-review"]
```

Note: The `docs_roots` use relative paths that resolve against the workspace root. This matches the existing KG resolution chain behavior for multi-corpus agents.

**LLM override seeding:** Extend `WellKnownAgent` struct with an optional `llm_overrides: &[(&str, &str, &str)]` field (skill_name, provider, model). For mika-arch:
- `("mika-arch-groom-ticket", "anthropic", "claude-opus-4-7")`
- `("mika-arch-second-review", "anthropic", "claude-sonnet-4-6")`

`seed_well_known_skill_overrides()` already iterates disabled_skills; extend it to also write `set_skill_llm(agent_name, skill_name, provider, model)` for each `llm_overrides` entry. Same first-creation-only guard applies.

**Patterns to follow:**
- `MIKA_DEV`, `MIKA_QA`, `MIKA_RELAY` static definitions and their soul/config constants.
- `provision_well_known_agents()` filesystem write pattern.
- `seed_well_known_skill_overrides()` DB write pattern.

**Test scenarios:**
- Happy path: fresh DB start → mika-arch agent created with correct identity.toml content (including `[skills].allowlist` and `[kg].docs_roots`), soul.md, and config.toml.
- Happy path: `seed_well_known_skill_overrides` writes LLM overrides for both skills to `skill_overrides` table.
- Happy path: mika-arch's skill registry after full override chain shows exactly the two allowlisted skills, with correct LLM providers.
- Edge case: existing mika-arch agent → provisioning skips (idempotent).
- Edge case: `MIKA_DISABLE_AGENT_PROVISIONING=true` → mika-arch not created.
- Edge case: existing `skill_overrides` rows for mika-arch → `seed_well_known_skill_overrides` skips (preserves customization).
- Regression: mika-dev, mika-qa, mika-relay provisioning unchanged.

**Verification:**
- All tests pass including new mika-arch tests and existing agent tests.
- `WELL_KNOWN_AGENTS` contains 4 agents.
- mika-arch identity.toml has `[skills].allowlist` and `[kg]` sections.

---

- [ ] **Unit 4: `mika-arch-groom-ticket` bundled skill**

**Goal:** First-pass plan review skill invoked on Opus 4.7. Produces READY / ITERATE / ESCALATE disposition with annotated plan content.

**Requirements:** R3 (first review).

**Dependencies:** Unit 1 (`gh_read` builtin exists for tools.json reference).

**Files:**
- Create: `skills/bundled/mika-arch-groom-ticket/skill.toml`
- Create: `skills/bundled/mika-arch-groom-ticket/system_prompt.md`
- Create: `skills/bundled/mika-arch-groom-ticket/tools.json`

**Approach:**

`skill.toml`:
- `name = "mika-arch-groom-ticket"`, `version = "0.1.0"`, `always_on = false`
- `[triggers] keywords = ["groom-ticket", "review-plan", "architect-review", "arch-review", "first-review"]`
- `[constraints] required_tools = ["gh_read"]` — forces tool use, prevents prose-only fabrication
- No `[llm]` section (removed in #504; LLM override seeded via DB in Unit 3)

`tools.json`: Declares `gh_read` as a builtin handler tool with its input schema (`op`, `target`, `repo`). Also declares `query_knowledge_graph`, `conversation_search`, `recent_chats`, `web_search` as the remaining five of the six-tool kit. `context7` is an MCP tool and doesn't need declaration here — it's available via MCP namespace.

`system_prompt.md`:
- Title: "## mika-arch — Plan Grooming (First Review)"
- Role: Principal-Engineer-class advisory reviewer
- References `docs/architecture/review-guide.md` by path — does NOT embed principles
- Operating discipline: citation-or-silence
- PROCESS section: (1) Read the brief + plan + issue context, (2) Use `gh_read` to fetch the issue and any referenced PRs, (3) Query KG for institutional learnings relevant to the plan's domain, (4) Review against the review guide's principles, (5) Annotate the plan content with inline findings
- OUTPUT section: Return annotated plan content as a string followed by an explicit `Disposition: READY` | `Disposition: ITERATE` | `Disposition: ESCALATE` line
- CONSTRAINTS section: Read-only — no shell, no commit, no merge. Every architectural concern must cite the review guide, an ADR, or a compound doc. If you cannot cite it, do not flag it.

**Patterns to follow:**
- `skills/bundled/qa-review/` — review-class skill structure with constraints and context.
- `skills/bundled/skill-review/tools.json` — builtin handler tool declaration pattern.

**Test scenarios:**
- Test expectation: none at unit level — prompt-only skill, validated by build-time discovery and startup validation. The skill appears in `BUNDLED_SKILL_MANIFESTS` after `cargo build`.

**Verification:**
- `cargo build` succeeds and the skill appears in the generated bundled skills table.
- `skill.toml` parses as valid `SkillManifest`.
- `system_prompt.md` references `docs/architecture/review-guide.md`.

---

- [ ] **Unit 5: `mika-arch-second-review` bundled skill**

**Goal:** Iteration-pass review skill invoked on Sonnet 4.6. Produces GROOMED or ESCALATE; never ITERATE.

**Requirements:** R3 (second review), R11 (no third pass).

**Dependencies:** Unit 1 (`gh_read` builtin exists).

**Files:**
- Create: `skills/bundled/mika-arch-second-review/skill.toml`
- Create: `skills/bundled/mika-arch-second-review/system_prompt.md`
- Create: `skills/bundled/mika-arch-second-review/tools.json`

**Approach:**

`skill.toml`:
- `name = "mika-arch-second-review"`, `version = "0.1.0"`, `always_on = false`
- `[triggers] keywords = ["second-review", "groom-iteration", "iterate-review"]`
- `[constraints] required_tools = ["gh_read"]`
- No `[llm]` section (DB-seeded in Unit 3)

`tools.json`: Same six-tool kit as Unit 4 (gh_read, query_knowledge_graph, conversation_search, recent_chats, web_search).

`system_prompt.md`:
- Title: "## mika-arch — Plan Review (Second Pass)"
- Role: Iteration reviewer checking whether ITERATE findings from first pass were addressed
- References `docs/architecture/review-guide.md` by path
- Operating discipline: citation-or-silence
- PROCESS section: (1) Read the revised plan and the prior first-pass review from conversation memory, (2) For each prior ITERATE finding, verify whether the plan revision addressed it, (3) Use `gh_read` for any issue/PR context needed, (4) Annotate the revised plan
- OUTPUT section: Annotated revised plan followed by `Verdict: GROOMED` | `Verdict: ESCALATE`. **Hard constraint: no "ITERATE" or "needs-third-pass" verdict.** If concerns remain after this pass, the answer is ESCALATE — a human must decide.
- CONSTRAINTS section: Same read-only constraints as groom-ticket. Additional: if conversation memory is unavailable (no session correlation), fall back to the prior review content passed in the package payload.

**Patterns to follow:**
- Same as Unit 4.
- `skills/bundled/qa-review/system_prompt.md` — structured verdict output pattern.

**Test scenarios:**
- Test expectation: none at unit level — prompt-only skill, validated by build-time discovery.

**Verification:**
- `cargo build` succeeds and the skill appears in the generated bundled skills table.
- `skill.toml` parses as valid `SkillManifest`.
- `system_prompt.md` explicitly prohibits ITERATE verdict.

---

- [ ] **Unit 6: Integration wiring and CLAUDE.md update**

**Goal:** Wire `apply_identity_allowlist` into the agent init path, update CLAUDE.md, and verify end-to-end.

**Requirements:** All.

**Dependencies:** Units 1-5 all complete.

**Files:**
- Modify: the module where `apply_overrides` is called (likely `crates/mika-agent/src/bundled_skills.rs` or `crates/mika-agent/src/agent.rs`) — add `apply_identity_allowlist` call before `apply_overrides`
- Modify: `CLAUDE.md` — reference mika-arch in well-known agents section, update bundled skills list

**Approach:**

Find the call site where `registry.apply_overrides(overrides)` is invoked. Before that call, load the agent's identity and check for an allowlist:
```
let identity = load_identity(home_dir);
if let Some(ref allowlist) = identity.skills.allowlist {
    if !allowlist.is_empty() {
        registry.apply_identity_allowlist(allowlist);
    }
}
registry.apply_overrides(&overrides);
```

Update CLAUDE.md:
- Add `mika-arch` to the well-known agents list
- Add `mika-arch-groom-ticket` and `mika-arch-second-review` to the bundled skills directory listing
- Reference `docs/architecture/review-guide.md` if not already documented

**Test scenarios:**
- Integration: end-to-end test that provisions mika-arch, loads skills, applies identity allowlist, applies DB overrides, and verifies the final skill registry contains exactly `mika-arch-groom-ticket` and `mika-arch-second-review` with correct LLM providers.
- Integration: `cargo build` succeeds and `cargo test` passes across the entire workspace.

**Verification:**
- All existing tests pass (regression).
- New mika-arch integration test passes.
- `cargo clippy` clean.

## System-Wide Impact

- **Interaction graph:** `apply_identity_allowlist` runs in the skill-loading chain before `apply_overrides`. It affects `SkillRegistry` state. No callbacks or middleware affected.
- **Error propagation:** `gh_read` errors propagate as `ToolOutput::error` strings — the LLM sees them and can respond appropriately. No cross-layer failure modes.
- **State lifecycle risks:** First-creation-only guard in `seed_well_known_skill_overrides` prevents overwriting operator customizations. Identity allowlist is file-based, immutable per restart.
- **API surface parity:** No API changes. Dashboard is unaffected. CLI `mika skills list` will show mika-arch's skills after provisioning.
- **Integration coverage:** The main risk is the Phase -1 → Phase 0 → Phase 1 override chain ordering. An integration test must verify that identity allowlist eviction happens before DB-driven eviction and LLM override application.
- **Unchanged invariants:** `default_tools()` registry is unchanged. `run_gh` behavior is unchanged. Existing three well-known agents' provisioning and skill override behavior is unchanged. `skill_overrides` table schema is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `[skills].allowlist` references skills that don't exist yet at startup (skills bundled in same PR) | Build-time discovery runs before startup validation; skills are in the generated table before `apply_identity_allowlist` runs. Log warn for unresolvable names but don't crash. |
| `gh_read` name collision with existing tool | Verified: `gh_read` not in `KNOWN_BUILTINS` or any `tools.json`. Name is unique. |
| Per-skill LLM override not applied because `apply_overrides` runs after allowlist eviction removes all but 2 skills | By design: allowlist eviction keeps the 2 skills alive, then Phase 1 applies LLM overrides from DB to those survivors. |
| mika-arch's `[kg].docs_roots` paths don't exist on all machines | KG resolution chain validates each path independently; missing paths logged as WARN and skipped. Agent starts if at least one path is valid. |
| Existing well-known agent tests break due to `WellKnownAgent` struct change | Add `llm_overrides: &[]` default to existing agents. Existing tests should pass with empty arrays. |

## Documentation / Operational Notes

- After deploying this PR, restart mika-spirit with `MIKA_DEV_MODE=true` to trigger mika-arch provisioning.
- Verify: `mika agents list` shows `mika-arch`. `mika skills --agent mika-arch list` shows two enabled skills.
- The four `[kg].docs_roots` paths must exist on the deployment host for KG ingestion to populate `agent_kg_corpora`.
- Per-skill LLM overrides can be changed post-deploy via `mika skills llm mika-arch-groom-ticket set anthropic/claude-opus-4-7`.

## Sources & References

- **Origin document:** mika-platform branch `chore/mika-arch-v1-plan` — `docs/plans/2026-04-25-001-feat-mika-arch-v1-plan.md`
- Related PR: senara-solutions/mika#810 (Unit 1 — review-guide.md)
- Related issue: senara-solutions/mika#811
- Parent milestone: senara-solutions/mika-platform#51
