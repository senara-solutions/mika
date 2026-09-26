---
title: "refactor: Rename claude-pilot skill to dev-pilot (decouple skill identity from app identity)"
type: refactor
status: active
date: 2026-04-27
origin: https://github.com/senara-solutions/mika/issues/844
---

# refactor: Rename claude-pilot skill to dev-pilot

## Overview

Rename the dispatch skill `mika/skills/bundled/claude-pilot/` → `mika/skills/bundled/dev-pilot/` to decouple **skill identity** (the role: dispatch a headless implementation session) from **app identity** (the `claude-pilot` binary the skill wraps). After this work plus the companion ticket (mika#845, dev-groom), the autonomous dev loop's skill family will read `dev-pilot` (dispatch) + `dev-groom` (grooming) — naming the **role**, not the **app**.

The refactor is structural: it changes how mika-dev's prompts, the skill registry, and tool routing reason about the dispatch surface. The architect's pre-filing consult (session `46084b1a-d873-4e8a-92ae-4e76b0396348`, 2026-04-27, Disposition: ITERATE) folded four iterations into the ticket body, all reflected in this plan.

## Problem Frame

Today the autonomous dev loop has implicit asymmetry. Dispatch is a first-class skill at `mika/skills/bundled/claude-pilot/`. Grooming lives as a slash command in mika-platform. The dispatch skill's name is the **app** it wraps (`claude-pilot` the CLI installed by `claude-pilot-py`), not the **role** it plays. Adding a sibling skill (dev-groom, mika#845) forces the question: does the family read `claude-pilot + dev-groom` (incoherent — one is an app name, the other is a role) or `dev-pilot + dev-groom` (coherent — both name roles)?

This plan implements the rename. It keeps the app identity (`claude-pilot` binary, `claude-pilot-py` repo, `[claude-pilot]` log channel, `/var/log/claude-pilot/*` paths, `tasks.metadata.claude_pilot.*` JSON namespace) untouched. It also keeps the dispatch tool name `run_claude_pilot` (per Decision D1) — the tool names the **launcher**, not the skill, since the launcher will host multiple skills.

## Requirements Trace

- R1. The skill at `mika/skills/bundled/claude-pilot/` is renamed to `mika/skills/bundled/dev-pilot/`. Its `skill.toml` `name` field reads `"dev-pilot"`. (Ticket § In scope, line 1)
- R2. The `run_claude_pilot` tool gains a required `skill:` argument. Existing call sites pass `skill: "dev-pilot"` explicitly. (Ticket § D1, line 1)
- R3. KG self-knowledge eval fixtures gain at least one `skill:`-arg routing assertion: "implement `<ticket-ref>`" → `run_claude_pilot {skill: "dev-pilot", ...}`. (Ticket § D1, eval-fixture coverage gap)
- R4. Sibling-skill prompts and live docs are audited per-line via the grep-and-annotate verification block. App-meaning matches stay; skill-meaning matches rename. (Ticket § D2)
- R5. Open GitHub issue bodies referencing `claude-pilot` in skill-meaning context are edited or addended. (Ticket § D2, open-issue sweep — added per architect concern 2)
- R6. Pre-deploy SQL probe runs against `~/.mika/data/mika.db`; deploy proceeds only if max in-flight task age < 30 minutes. (Ticket § D3, pre-deploy gate)
- R7. The `tasks.metadata.claude_pilot.*` JSON namespace is NOT renamed. (Ticket § Out of scope — load-bearing for mika#838's verdict handler)
- R8. End-to-end smoke: `mika ask --agent mika-dev "implement <test-ticket>"` routes to `skill: "dev-pilot"`. (Ticket § Verification checklist, line 9)

## Scope Boundaries

- The `claude-pilot` binary, its invocation in handlers, its Python package source (`claude-pilot-py/`), or its upstream repo
- Log paths `/var/log/claude-pilot/*` and the `[claude-pilot]` source-prefix channel marker
- `tasks.metadata.claude_pilot.*` JSON namespace (app-level subprocess metadata; load-bearing for mika#838)
- Historical brainstorms (`mika/docs/brainstorms/*`), executed plans (`mika/docs/plans/*`), compounded solutions (`mika/docs/solutions/*`) — point-in-time records
- `mika-skills/docs/*` historical content
- Broader callback-handler eval coverage — that's mika#806's scope (this plan adds only minimal `skill:`-arg assertions)

### Deferred to Separate Tasks

- **dev-groom skill creation** — mika#845 (companion ticket, sprint-bundled with this one).
- **`--verbose` metadata envelope expansion** — mika#843 (cross-cutting; affects mika#845 only, not this plan).
- **Broader callback-handler eval coverage** — mika#806 (parent debt for skill-routing + callback-handler integration tests beyond this plan's two minimal assertions).

## Context & Research

### Relevant Code and Patterns

- `mika/skills/bundled/claude-pilot/{skill.toml,system_prompt.md,tools.json,handlers/run.sh}` — current skill identity surface
- `mika/crates/mika-agent/src/well_known_agents.rs` — well-known agent skill bindings (3 sites for `"claude-pilot"`)
- `mika/crates/mika-agent/src/agent.rs` — agent skill enumeration (2 sites)
- `mika/crates/mika-agent/src/skills/mod.rs` — skill registry tests (~10 sites including dependency wiring `make_entry_with_deps("self-dev", ..., &["claude-pilot"])`)
- `mika/crates/mika-agent/src/skills/executor.rs` — `run_claude_pilot` tool registration
- `mika/crates/mika-agent/src/tools/mod.rs` — tool argument schema + dispatch
- `mika/crates/mika-agent/src/db/kg_schema.rs` — KG seed entries; line 209-210 register `tool:run_claude_pilot` (this entry's text may need updating to mention dispatch role); skill seed entry needs separate audit
- `mika/skills/bundled/{self-dev,self-dev-iterate,self-dev-webhook-ci,self-dev-webhook-qa,qa-review,permission-policy,address-pr-comments,resolve-pr-conflicts}/` — sibling skill prompts (mix of app-meaning and skill-meaning references)
- `mika/skills/bundled/self-dev/system_prompt.md` — most claude-pilot mentions are app-meaning ("Launching claude-pilot," "claude-pilot finishes," `[claude-pilot]` channel marker). Skill-meaning matches are subtle: e.g., line 43 "delegate to claude-pilot" reads as the role.

### Institutional Learnings

- `mika/docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md` — bidirectional pre-filing scope verification discipline that produced this plan's scope.
- `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — plan-on-branch is the contract.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — known prompt-adherence drift on disposition keywords; tolerated.
- `mika/docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md` — skill registry semantics.
- `mika-platform/docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md` — same discipline at meta-repo level.

### Architect Verdict (Pre-Filing Consult + First-Pass Groom Review)

- **Pre-filing session:** `46084b1a-d873-4e8a-92ae-4e76b0396348` (2026-04-27, mika-arch). Disposition: ITERATE. Iterations folded into ticket body before filing:
  1. mika#806 cited as parent debt for broader skill-routing eval coverage; this plan adds two minimal inline `skill:`-arg assertions.
  2. Open-issue body sweep added to D2 audit policy (mika#784 is the named example: title says "claude-pilot handler" in skill-meaning context).
  3. Pre-deploy SQL probe codified in D3 (against `~/.mika/data/mika.db`, 30-minute threshold).
  4. mika#843 sequencing coupling noted in companion ticket (mika#845, not this plan).

- **First-pass groom-ticket session:** `d20ac2fb-bb61-4822-ba0a-4c07d73014a3` (2026-04-27, mika-arch). Disposition: ITERATE with 8 findings + 1 conditional dispatch-blocker. Iterations folded into this plan revision:
  1. F1 — Unit 1 names KG seed discovery command + expected output shape.
  2. F2 — D2 adds explicit three-category classification rubric (app/skill/ambiguous).
  3. F3 — Unit 2 specifies the schema-validation error message format for missing/invalid `skill:` arg.
  4. F4 (conditional dispatch-blocker, **resolved**) — Unit 6 simplified to reference the auto-rebuild mechanism in `kg/domain_builder.rs`. Verified Path A applies (sole-writer + idempotent + per-boot rebuild). No "file follow-up" workaround needed.
  5. F5 — branch renamed to `refactor/844/...` to align with plan filename + work shape; D4 documents the deviation.
  6. F6 — D3 names the rationale for the 1800-second threshold (pragmatic + empirical bounds on typical claude-pilot dispatch length).
  7. F7 — Unit 5 adds edit-vs-comment criteria for issue body sweep (active-spec → edit; historical-record → comment).
  8. F8 — Unit 2 pre-states mika#845 Path A/B branches with cross-ticket dependency note.

## Key Technical Decisions

### D1 — `run_claude_pilot` tool name stays; `skill:` parameter is required

**Position:** keep tool name `run_claude_pilot`. Add a required `skill:` argument with enum `["dev-pilot"]` (extended to `["dev-pilot", "dev-groom"]` when mika#845 ships).

**Rationale:** The tool's job is "spawn a claude-pilot subprocess with a given skill prompt." Under the framing that the app hosts multiple skills, the tool names the **launcher**. One tool registration vs N tools per N skills with surface duplication on shared dispatch parameters (issue ref, repo, branch).

**Coverage gap closure:** Single-tool-with-arg means existing tool-selection fixtures pass on tool name alone without verifying the right `skill:` value gets routed. Resolution per architect verdict: add minimal inline assertions in this ticket; cite mika#806 as parent debt for broader coverage.

### D2 — Per-line judgment + grep-and-annotate verification + open-issue sweep

For every file in the in-scope list (sibling skills, live docs, mika-platform refs), the implementer runs:

```
grep -n "claude-pilot\|claude_pilot\|ClaudePilot" <file>
```

Each match is annotated against the classification rubric below. The annotated grep output is the auditable artifact in the PR description.

**Classification rubric — three categories, named explicitly so per-line judgment is bounded by criteria, not implementer-discretion:**

- **App-meaning (KEEP):** the match references the binary, the log channel `[claude-pilot]`, the log path `/var/log/claude-pilot/`, the metadata namespace `metadata.claude_pilot.*`, the upstream repo (`claude-pilot-py`, `senara-solutions/claude-pilot`), the source-prefix detection from mika#841, or the tool-label `run_claude_pilot`.
- **Skill-meaning (RENAME to `dev-pilot`):** the match references the dispatch role, the bundled skill directory `mika/skills/bundled/claude-pilot/`, the `skill_id` value `"claude-pilot"`, or the skill-descriptor in KG seeds.
- **Ambiguous (FLAG for PR-blocking review thread):** the match could plausibly read as either category; resolution requires reading surrounding prose context. Ambiguous matches are NOT renamed during Unit 4's mechanical pass. They surface as PR-blocking review threads on the PR; the reviewer settles each before merge (architect available for escalation if a thread can't be settled — escalation, not default). This forces explicit disposition, keeps responsibility within the PR review actor, and produces an auditable thread artifact. Without the explicit ambiguous bucket, a low-confidence implementer collapses to one category and silently misclassifies.

**Open-issue body sweep** (architect concern 2): before deploy, run `gh issue list --repo senara-solutions/mika --state open --search "claude-pilot"` and triage each open ticket body. Skill-meaning bodies (e.g. mika#784: "pre-push duplicate-commit guard in claude-pilot handler") get either a body edit or an addendum comment per Unit 5's edit-vs-comment criteria.

### D3 — Clean cutover, gated by SQL pre-commit probe

**Position:** clean cutover. The skill registry rejects unknown skill names; in-flight tasks that reference `skill_id="claude-pilot"` at deploy time fail fast and the operator re-dispatches. No deprecated alias.

**Rationale:** the corrected framing (skill identity is registry-only, app metadata namespace stays) bounds the cutover blast radius to the deploy window. Alias's "skill identity ambiguous for one release" cost is unbounded.

**Pre-deploy gate (mandatory):** the implementer runs the SQL probe documented in Unit 6 against `~/.mika/data/mika.db` immediately before deploy. `max_age_seconds < 1800` → proceed. `max_age_seconds >= 1800` → halt and escalate.

**Rationale for the 1800-second (30-minute) threshold:** pragmatic — an in-flight `tasks` row older than 30 minutes is most likely either (a) a milestone-scope dispatch with hours of work that would lose progress to a fail-fast rejection on unknown skill name, or (b) already stalled/hung and worth operator attention regardless of the cutover. 30 minutes is also approximately the upper bound of a typical claude-pilot dispatch (well-formed runs complete in 20-25 minutes). Future operators tightening or loosening the threshold should anchor on this rationale, not pick a different round number.

### D4 — Plan filename and branch both use `refactor` prefix

**Position:** plan filename and branch both use `refactor` (semantic — this is a refactor, not new functionality). The spec default (`enhancement` label → `feat`) is overridden because the work shape (rename + registry update + no new behavior) is genuinely refactor.

**Rationale:** the branch is the contract; the plan file is the artifact. Both should reflect the work's nature. Per the architect first-pass review (Finding 5), label-derived branch prefixes are a `/mika-groom-ticket` default convention that can deviate when the work shape doesn't match the label. The branch was originally created as `feat/844/...` per spec default and renamed to `refactor/844/rename-claude-pilot-skill-to-dev-pilot` after first-pass review (cheap — no remote tracking established at that point). The worktree directory path on disk stays `feat-844-...` (cosmetic; `git worktree move` would be unnecessary churn).

## Open Questions

### Resolved During Planning

- **Tool name shape:** keep `run_claude_pilot` + add `skill:` arg (D1).
- **Audit policy:** per-line judgment with grep-and-annotate verification block (D2).
- **Backwards-compat alias:** clean cutover gated by SQL probe (D3).
- **Plan/branch type:** branch `feat`, plan filename `refactor` (D4).

### Deferred to Implementation

- **Exact `skill:` enum values** at deploy time: `["dev-pilot"]` if mika#845 hasn't shipped yet, otherwise `["dev-pilot", "dev-groom"]`. The implementer checks mika#845's status before finalizing the enum.
- **Whether `kg_schema.rs` has a separate skill-descriptor seed entry** (beyond the `tool:run_claude_pilot` entry at line 209-210). The implementer audits the file during Unit 1 and updates accordingly.
- **Exact prose rephrasing for self-dev/system_prompt.md skill-meaning matches.** Most matches are app-meaning; the implementer applies per-line judgment per D2.
- **Whether `permission-policy/skill.toml` keyword triggers (e.g. `[claude-pilot]` channel keyword) need updating.** They likely DON'T — the channel marker stays per Vincent's clarification — but the implementer verifies during Unit 4.

## Implementation Units

- [ ] **Unit 1: Skill identity rename + skill registry**

**Goal:** Rename the skill directory and update all skill-ID literal references in Rust source so the registry, well-known-agent bindings, and skill-dependency wiring use `"dev-pilot"` instead of `"claude-pilot"`. Build is green at end.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Rename directory: `skills/bundled/claude-pilot/` → `skills/bundled/dev-pilot/`
- Modify: `skills/bundled/dev-pilot/skill.toml` (`name = "claude-pilot"` → `"dev-pilot"`; description; trigger keywords if any reference the skill name)
- Modify: `skills/bundled/dev-pilot/system_prompt.md` (self-references — apply per-line judgment per D2)
- Modify: `skills/bundled/dev-pilot/tools.json` (description text only — tool name `run_claude_pilot` stays per D1)
- Modify: `crates/mika-agent/src/well_known_agents.rs` (3 sites at lines 105, 140, 768 — verify all are skill-ID literals, not app-meaning; rename)
- Modify: `crates/mika-agent/src/agent.rs` (2 sites at lines 5258, 5470 — same verification, rename)
- Modify: `crates/mika-agent/src/skills/mod.rs` (~10 test fixture sites at lines 934, 974, 1049-1050, 1053, 1078, 1191, 1193 — rename)
- Audit: `crates/mika-agent/src/db/kg_schema.rs` for any skill-descriptor entries (separate from the `tool:run_claude_pilot` entry at line 209-210); rename skill-meaning entries
- Test: existing `cargo test --package mika-agent` suite (no new test files for this unit; existing tests verify registry integrity)

**KG seed discovery command (per architect Finding 1 — converts "implementer audits the file" from ritual into mechanical assertion):**

```
grep -n '"claude-pilot"\|claude_pilot' crates/mika-agent/src/db/kg_schema.rs
```

Expected output: a non-empty list of `(line, kind)` pairs. The implementer classifies each per the D2 rubric:
- **Tool-label entries** (e.g., `format_entity_key("tool", "run_claude_pilot")` at line 209-210): app-meaning. KEEP.
- **Skill-descriptor entries** (e.g., `format_entity_key("skill", "claude-pilot")`, if any): skill-meaning. RENAME to `"dev-pilot"`.
- **Ambiguous entries:** flag for D2's second-pass review.

The same pattern applies to all D2-scoped files in Unit 4. This Unit 1 instance is named here because the KG seed is structurally different (Rust source code with `format_entity_key(...)` call sites, not prose) and the discovery command reflects that shape.

**Approach:**
- The skill directory rename + `skill.toml` `name=` field + Rust skill-ID lookups must move together as one atomic commit. Splitting them creates an interim broken-build state where the registry references a directory that no longer exists.
- Use `git mv` for the directory rename to preserve history.
- After the rename, run the per-file grep-and-annotate verification block (D2) on the four files inside the renamed `dev-pilot/` directory; only skill-meaning matches change in this unit.

**Patterns to follow:**
- Existing skill registrations in `mika/skills/bundled/{self-dev,qa-review,...}/skill.toml` for the canonical `[skill]` block shape.
- `crates/mika-agent/src/skills/mod.rs:1050` `make_entry_with_deps("self-dev", ..., &["claude-pilot"])` — the dependency-wiring pattern; this dependency edge becomes `&["dev-pilot"]`.

**Test scenarios:**
- Happy path: `cargo build` succeeds; `cargo test --package mika-agent` green.
- Edge case: skill-registry tests assert `names.contains(&"dev-pilot")` (not `&"claude-pilot"`).
- Edge case: skill-dependency-resolution tests with `make_entry_with_deps("self-dev", ..., &["dev-pilot"])` resolve correctly.
- Integration: re-run KG self-knowledge tests (covered in Unit 3) — must still pass after this unit lands.

**Verification:**
- `grep -rn '"claude-pilot"' crates/mika-agent/src/` returns only app-meaning matches (e.g., comments referencing the binary or log channel) or zero.
- The bundled skill directory at `skills/bundled/dev-pilot/` exists with the four canonical files; `skills/bundled/claude-pilot/` no longer exists.
- `cargo test --package mika-agent` green.

---

- [ ] **Unit 2: Tool surface — add required `skill:` parameter to `run_claude_pilot`**

**Goal:** Make the `run_claude_pilot` tool's argument schema explicit about which skill the launcher hosts. Add a required `skill:` argument with enum `["dev-pilot"]`. Update internal call sites.

**Requirements:** R2

**Dependencies:** Unit 1 (skill `"dev-pilot"` must be registered before the enum value is valid)

**Files:**
- Modify: `skills/bundled/dev-pilot/tools.json` (add `skill` field to `input_schema.properties`; add to `required`)
- Modify: `crates/mika-agent/src/skills/executor.rs` (line 1867 region — `run_claude_pilot` tool registration; propagate `skill` arg to subprocess invocation in `handlers/run.sh` env or args)
- Modify: `crates/mika-agent/src/skills/mod.rs` (lines 1044, 1186 — tool name `"run_claude_pilot"` declarations; verify these are tool-label declarations; argument schema validation)
- Modify: `skills/bundled/dev-pilot/handlers/run.sh` (consume the new `skill` env var; use it to select the subprocess prompt — currently hardcoded to the dispatch flow)
- Audit: every internal site that builds `run_claude_pilot` invocations from prose (e.g., `crates/mika-agent/src/server/ci_failure_handler.rs` lines 646, 652 — the LLM-instruction strings tell the agent to "dispatch run_claude_pilot"; the prompt should be updated to instruct the agent to pass `skill: "dev-pilot"`)
- Test: `crates/mika-agent/src/skills/executor.rs` tests covering the new arg validation

**Approach:**
- The tool's argument schema is the contract surface. Adding a required field is a breaking change for callers — but in this codebase the only callers are mika-dev's prompts (updated in Unit 4) and the skill executor itself (this unit). No external API consumers.
- The `skill` arg propagates to `handlers/run.sh` as an env var or positional argument. The handler script uses it to load the right system prompt. (Today the handler is implicitly dispatch-only because there's only one skill on this app.)

**Schema-validation error message (per architect Finding 3):** when a caller invokes `run_claude_pilot` without `skill:`, the schema-validation failure must produce an explicit, named error — not a generic schema-mismatch. Required form: `"missing required argument 'skill'; valid values: [<enum-values>]"`. When a caller provides an invalid enum value (e.g., `skill: "claude-pilot"` post-rename), the error must list the valid values. Operators reading logs and prompt authors debugging routing both depend on the named affordance.

**mika#845 (dev-groom) cross-ticket dependency (per architect Finding 8) — pre-stated branches, not implementer-discretion:**

**Amended 2026-04-27 post mika#845 first-pass architect review:** the Path A/B branching below was speculative; mika#845's D7 (skill-not-tool, operator-only enforcement) supersedes. Under mika#845's structural enforcement (D2 Layers 1+3 — agent allowlist + gateway guard), no caller ever passes `skill: "dev-groom"` to `run_claude_pilot` — operators activate dev-groom via skill keyword match, not via the dispatch tool. Adding `"dev-groom"` to the enum produces a structurally-unreachable code path (YAGNI violation). Both branches reduce to **enum is `["dev-pilot"]` permanently; mika#845 registers the skill but adds NO entry to this enum.**

- **Path A — mika#845 already merged at implementation time of this Unit:** enum is `["dev-pilot"]`. Unit 3's inline assertions cover the dispatch routing: "implement <ticket-ref>" → `skill: "dev-pilot"`. dev-groom routing is verified separately via skill-activation eval fixture in mika#845's Unit 4 (not via this enum).
- **Path B — mika#845 not yet merged:** enum is `["dev-pilot"]`. Same as Path A. No coordination required between tickets on this enum's value.

The implementer **must** check mika#845's merge status at the start of Unit 2 and select the path. Do not silently default to one branch.

**Patterns to follow:**
- Existing tool schema declarations in `crates/mika-agent/src/skills/mod.rs` for required-field patterns.
- `crates/mika-agent/src/server/ci_failure_handler.rs` for how prose-instruction strings reference tool names and arguments.

**Test scenarios:**
- Happy path: `run_claude_pilot {prompt: "mika#844", task_id: "<uuid>", skill: "dev-pilot"}` invokes the dispatch flow.
- Error path: `run_claude_pilot {prompt: ..., task_id: ...}` (no `skill:` arg) returns a schema-validation error.
- Error path: `run_claude_pilot {..., skill: "claude-pilot"}` (old name) returns an enum-validation error.
- Integration: end-to-end test that mika-dev's `dispatch <ticket>` prose-instruction routing arrives at `run_claude_pilot {skill: "dev-pilot", ...}`.

**Verification:**
- The tool's `input_schema` declares `skill` as required.
- All internal call sites (test fixtures, handler scripts) provide `skill: "dev-pilot"` explicitly.
- A synthetic test where mika-dev's prompt dispatches to `run_claude_pilot` produces an invocation with the explicit `skill` arg.

---

- [ ] **Unit 3: KG seed update + tool-selection eval fixture additions**

**Goal:** Update the KG corpus's skill descriptor for `dev-pilot` and add minimal `skill:`-arg routing assertions to tool-selection eval fixtures (D1 inline coverage).

**Requirements:** R3

**Dependencies:** Unit 1 (skill must exist), Unit 2 (tool schema must accept `skill:` arg)

**Files:**
- Modify: `crates/mika-agent/src/db/kg_schema.rs` (audit for skill-descriptor entries; rename `"claude-pilot"` → `"dev-pilot"` in skill-meaning entries; the existing `tool:run_claude_pilot` entry at line 209-210 may need its description text updated to mention "the dev-pilot dispatch skill" if it references the role)
- Modify: `crates/mika-agent/tests/eval/kg_self_knowledge/path_a_direct_domain_match.rs` (add assertion: "implement <ticket-ref>" tool-selection returns `run_claude_pilot` with arg `skill: "dev-pilot"`)
- Modify: `crates/mika-agent/tests/eval/kg_self_knowledge/path_c_semantic_via_chunks.rs` (similar assertion)
- Modify: `crates/mika-agent/tests/eval/kg_self_knowledge/tool_selection_query_knowledge_graph.rs` (similar assertion)
- Audit: `crates/mika-agent/tests/eval/{test_callback_turn,test_phantom_retry_guard,test_self_knowledge_kg,test_verdict_handler,test_webhook_queue}.rs` — these tests reference `run_claude_pilot`. If they construct tool-call fixtures, they may need `skill: "dev-pilot"` in the args. Per-file judgment.

**Approach:**
- KG re-index is a deploy-time concern (handled in Unit 6); this unit only updates the seed source-of-truth in `kg_schema.rs`.
- The two new inline assertions in path_a and path_c are minimal — they verify that mika-dev routes "implement..." prose to the right `skill:` arg value. Broader skill-routing coverage is mika#806's scope (cited as parent debt in Sources).
- Avoid bloating the eval fixture with redundant skill-arg assertions across all five test files. Inline coverage in path_a (direct domain match) + path_c (semantic match) is sufficient for D1's stated coverage gap closure. The other three test files only get `skill:` arg added if they construct tool-call fixtures that would otherwise fail schema validation post-Unit 2.

**Patterns to follow:**
- Existing path_a / path_c assertions for tool-name fixtures.
- KG seed shape in `kg_schema.rs:209-210` for the `format_entity_key` pattern.

**Test scenarios:**
- Happy path: `cargo test --package mika-agent --test eval` green.
- Happy path: path_a's new assertion: "implement mika issue#844" routes to `run_claude_pilot` with `skill: "dev-pilot"`.
- Edge case: KG query for "dispatch implementation" returns the `dev-pilot` skill descriptor (not the old `claude-pilot` skill).

**Verification:**
- `cargo test --package mika-agent --test eval` green.
- The new inline assertions pass.
- `grep -n "claude-pilot" crates/mika-agent/src/db/kg_schema.rs` returns only app-meaning matches (e.g., binary references in tool descriptions if any).

---

- [ ] **Unit 4: Sibling-skill prompt + live docs audit (per-line judgment)**

**Goal:** Apply the grep-and-annotate verification block (D2) across sibling-skill prompts, live mika docs, and live mika-platform docs. Rename only skill-meaning matches; leave app-meaning matches.

**Requirements:** R4

**Dependencies:** None (can land in parallel with Unit 1-3 conceptually, but bundle into the same PR for atomicity)

**Files (sibling-skill prompts — mika repo):**
- Modify: `skills/bundled/self-dev/system_prompt.md` (~25 lines reference claude-pilot; per-line audit. Most are app-meaning — `[claude-pilot]` channel marker, `Launching claude-pilot`, `claude-pilot finishes`, `/var/log/claude-pilot/`, `metadata.claude_pilot.branch`. Skill-meaning: "delegate to claude-pilot" if it reads as the role, "claude-pilot is the dispatch skill" if it appears.)
- Modify: `skills/bundled/self-dev-iterate/system_prompt.md`
- Modify: `skills/bundled/self-dev-webhook-ci/system_prompt.md`
- Modify: `skills/bundled/self-dev-webhook-qa/system_prompt.md`
- Modify: `skills/bundled/qa-review/system_prompt.md`
- Modify: `skills/bundled/permission-policy/{skill.toml,system_prompt.md}` (verify the `[claude-pilot]` channel keyword trigger STAYS — per Vincent's clarification, the channel marker is app-level)
- Modify: `skills/bundled/address-pr-comments/{skill.toml,system_prompt.md,tools.json,handlers/run.sh}`
- Modify: `skills/bundled/resolve-pr-conflicts/{skill.toml,system_prompt.md,tools.json,handlers/run.sh}`

**Files (live docs — mika repo):**
- Modify: `CLAUDE.md`
- Modify: `crates/mika-agent/CLAUDE.md`
- Modify: `crates/mika-agent/docs/{architecture,configuration,runtime-structure}.md`
- Modify: `crates/mika-cli/CLAUDE.md`
- Modify: `docs/architecture.md`, `docs/configuration.md`, `docs/runtime-structure.md`
- Modify: `docs/architecture/kg-id-convention.md`
- (Out of scope per Scope Boundaries: `docs/brainstorms/`, `docs/plans/`, `docs/solutions/` — historical records.)

**Files (live docs — mika-platform repo, separate worktree):**
- Modify: `CLAUDE.md` (per-line audit; mix of app, repo, skill meanings)
- Modify: `.claude/settings.local.json` lines 33-34 (symlink permission entries — verify these refer to the bundled skill location, which is now at `skills/bundled/dev-pilot/`)
- Modify: `.claude/commands/mika-issue.md` lines 15, 20 (verify these refer to the upstream `claude-pilot-py` repo for issue routing — if so, leave; if skill-meaning, rename. Architect verdict: most likely upstream-repo references.)

**Approach:**
- For each file, run the grep-and-annotate command from D2. Capture the annotated output as a comment block in the PR description (auditable artifact).
- Apply rename only where annotation says "skill-meaning."
- Where the meaning is genuinely ambiguous, prefer the renamed form `dev-pilot` AND add an explicit clarifying word ("the dev-pilot skill" or "claude-pilot the binary") to disambiguate for future readers.
- mika-platform changes ship in a companion PR on the mika-platform repo (cross-repo per CLAUDE.md "Cross-Repo Development" conventions). Cross-reference both PRs in their bodies.

**Patterns to follow:**
- mika#841's positive-consent guard implementation included a similar per-line audit of `self-dev/system_prompt.md` for source-prefix detection — same shape of "annotate each match, decide per match."

**Test scenarios:**
- Test expectation: none in the unit-test sense — this unit is pure prose audit. Verification is via the grep-and-annotate artifact in the PR description.
- Integration smoke: after this unit lands, `mika ask --agent mika-dev "dispatch mika issue#<test>"` produces an invocation with `skill: "dev-pilot"` (verified end-to-end in Unit 6 deploy smoke).

**Verification:**
- The PR description contains a grep-and-annotate verification block per file in scope. Each match is annotated app-meaning or skill-meaning. The diff matches the annotations.
- `gh pr view <PR-num> --json body | jq -r '.body'` includes the annotation table.

---

- [ ] **Unit 5: Open GitHub issue body sweep**

**Goal:** Identify every open GitHub issue body referencing `claude-pilot` in skill-meaning context. Edit body or add addendum comment so the rename doesn't leave open tickets pointing to a renamed surface by its old name.

**Requirements:** R5

**Dependencies:** None (can be done in parallel with Units 1-4 conceptually; gates Unit 6 deploy)

**Files:**
- No source files modified. GitHub issue bodies on `senara-solutions/mika`.

**Approach:**
- Run `gh issue list --repo senara-solutions/mika --state open --search "claude-pilot" --json number,title,body --limit 100`
- For each result, classify the body's references per the D2 rubric:
  - **App-meaning only** (binary, log path, channel marker): skip; no change.
  - **Skill-meaning** (the dispatch role, "claude-pilot handler", "claude-pilot skill", etc.): apply the edit-vs-comment criteria below.
  - **Ambiguous:** flag for second-pass review; surface in the issue-sweep audit log.
- Mika#784 is the named example from the architect verdict ("pre-push duplicate-commit guard in claude-pilot handler" — skill-meaning). Confirm the example during execution; treat the architect's classification as a hint, not absolute truth.

**Edit-vs-comment criteria (per architect Finding 7) — distinction is contract-vs-history, not preference:**

- **Body edit** (with edit-notice comment): the body's skill-meaning reference is in the **active-spec sections** — proposed solution, acceptance criteria, decisions, scope, requirements. Active spec is reviewed by future implementers and must reflect current naming, not 2026-04-27's pre-rename naming. Edit in place; post a brief edit-notice comment naming the rename and timestamp. Same shape as the issue-as-versioned-contract pattern previously used in mika#654.
- **Addendum comment** (no body edit): the body's skill-meaning reference is in **historical-record sections** — problem statement, root-cause analysis, context, prior-art references, repro steps. Historical record stays accurate to time-of-filing; rewriting falsifies the lineage. Post a comment noting "naming update: post-mika#844, 'claude-pilot' here refers to the dispatch skill, now named `dev-pilot`."
- **Mixed bodies:** apply per-section. Edit active-spec sections; comment on historical-record sections. Reasonably interpret section boundaries when the body lacks explicit headers.

**Patterns to follow:**
- `gh issue comment <n> --body-file <tmpfile>` for addendum comments.
- `gh issue edit <n> --body-file <tmpfile>` for body edits.

**Test scenarios:**
- Test expectation: none in the unit-test sense — this unit is GitHub state management.
- Verification: post-deploy, `gh issue list --search "claude-pilot"` returns only app-meaning matches (or zero).

**Verification:**
- Every open issue with skill-meaning references has either an edited body or an addendum comment.
- Audit log (saved to PR description or to `/tmp/issue-sweep-844.md` and referenced in PR body): list of issue numbers, classification, action taken.

---

- [ ] **Unit 6: Deploy gate, cutover, and end-to-end smoke**

**Goal:** Pre-deploy SQL probe (D3) gates the cutover. Deploy includes KG re-index. Post-deploy smoke verifies end-to-end routing.

**Requirements:** R6, R8

**Dependencies:** Units 1-5 merged

**Files:**
- No new source files. Procedure is captured in the PR description and re-emitted to the deploy log.

**Approach:**

**Step 1 — Pre-deploy SQL probe (mandatory):**

Run against `~/.mika/data/mika.db` immediately before deploy:

```sql
SELECT max(strftime('%s','now') - strftime('%s', updated_at)) AS max_age_seconds
FROM tasks
WHERE status IN ('in_progress','pending');
```

- `max_age_seconds < 1800` (30 min): proceed.
- `max_age_seconds >= 1800`: halt deploy. Escalate to operator. A long-running task (milestone-scope dispatch) would lose hours of work to the fail-fast rejection on unknown skill name.

**Step 2 — Deploy sequence:**

1. `make deploy` (per `mika-platform/CLAUDE.md` § Local Dev Environment) — builds binaries, installs, restarts the agent service.
2. **KG domain graph auto-rebuilds on next server boot — no explicit command needed.** Per `crates/mika-agent/src/kg/domain_builder.rs` (verified during architect Finding 4 resolution): the domain builder is the **sole writer** of `skill:*`, `tool:*`, `agent:*`, and `problem_type:*` entity_keys; runs once per server boot after `SkillRegistry::apply_overrides()`; idempotent — re-running produces the same graph state. The `make deploy` → restart cycle auto-rebuilds from the in-memory `SkillRegistry`/`ToolRegistry` (which read from `kg_schema.rs` constants and the loaded skill manifests). Stale `claude-pilot` skill-descriptor entries in the live `kg_entities` table are overwritten on next boot.
3. **Lexical layer note (per architect second-review): in-scope `CLAUDE.md` and `docs/{architecture,configuration,runtime-structure}.md` edits land as content-hash changes; the lexical ingestor re-chunks on next boot per `source_doc_hash` idempotency. Out-of-scope `docs/solutions/**/*.md` historical mentions stay — historical record by design (per Scope Boundaries).** Same shape as the domain-builder Path A resolution: substrate behaves correctly regardless; naming it makes the behavior reviewable.
4. Confirm the agent service is up: `mika ask "ping"` returns.

**Step 3 — End-to-end smoke (mandatory before declaring deploy done):**

1. Pick a low-stakes test ticket (or use a synthetic one): `mika ask --agent mika-dev "implement mika issue#<test-ticket>"`
2. Verify the invocation reaches `run_claude_pilot` with `skill: "dev-pilot"` (check log in `/var/log/claude-pilot/<task-id>.log` or via dashboard).
3. If the invocation routes correctly, deploy is verified. Cancel the test dispatch (`mika ask --agent mika "cancel task <task-id>"`) to avoid actually executing the test ticket.
4. If the invocation fails (e.g., schema-validation error on missing `skill:` arg), halt and revert. The cutover discipline says fail-fast is intentional, but if mika-dev's prompt didn't get the `skill:` arg in Unit 2/4, that's a regression — fix before completing deploy.

**Patterns to follow:**
- Existing deploy procedures in `mika-platform/CLAUDE.md` § Local Dev Environment (`make deploy`).
- mika#841's positive-consent gate deploy verification used a similar end-to-end smoke.

**Test scenarios:**
- Happy path: SQL probe returns < 1800; deploy proceeds; smoke routes correctly to `skill: "dev-pilot"`.
- Edge case: SQL probe returns >= 1800; deploy halts; operator decides next action.
- Error path: smoke fails (schema validation, wrong skill arg, runtime error); deploy halts; investigation triggered.
- Integration: post-deploy, mika-dev's prompts and the new tool surface produce coherent end-to-end routing.

**Verification:**
- Deploy completes; SQL probe result + threshold logged in PR comment or deploy log.
- End-to-end smoke shows `skill: "dev-pilot"` in the dispatch log line.
- `gh issue list --search "claude-pilot"` (re-run from Unit 5 for completeness) returns only app-meaning matches.

## System-Wide Impact

- **Interaction graph:** mika-dev's prompts (now invoking `run_claude_pilot` with explicit `skill:` arg) → skill executor (validates enum) → skill registry (loads `dev-pilot` system prompt) → handlers/run.sh (selects dispatch flow). This chain replaces the implicit "single skill on the app" assumption.
- **Error propagation:** missing or invalid `skill:` arg surfaces as a tool-schema-validation error to the calling agent. mika-dev's tier-3 escalation handles this. In-flight tasks that pre-date the rename (referencing `skill_id="claude-pilot"`) fail fast on resume — operator re-dispatches per D3 cutover policy.
- **State lifecycle risks:** mid-cutover, an in-flight task in `tasks` table may reference the old skill ID. The SQL probe (Unit 6) bounds exposure to < 30 min. Beyond that threshold, deploy halts.
- **API surface parity:** the `run_claude_pilot` tool's argument schema is a public contract for mika-dev (and any future principal agents that learn the tool). Adding required `skill:` is a breaking change for prompts that don't supply it. mika-dev's prompts are updated in Unit 4; no other principal agents use this tool today.
- **Integration coverage:** the inline `skill:`-arg assertions in Unit 3 cover the routing-arg correctness gap inside this ticket. mika#806 (parent debt) is the canonical home for broader callback-handler coverage.
- **Unchanged invariants:** `claude-pilot` binary, `claude-pilot-py` repo, `[claude-pilot]` log channel marker, `/var/log/claude-pilot/*` log paths, `tasks.metadata.claude_pilot.*` JSON namespace, `run_claude_pilot` tool name. The dispatch tool's task labels (`run_claude_pilot`, `long_running:run_claude_pilot`) stay. Webhook source-prefix detection in mika-gateway (mika#841) is unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| In-flight tasks at deploy time reference old skill ID and fail to resume | SQL probe gate (D3) — halt deploy if max age ≥ 30 min |
| KG domain graph has stale `skill:claude-pilot` entries post-deploy | Auto-resolved: domain builder is sole writer + idempotent + runs on every boot. Verified in architect Finding 4 resolution. The lexical/subject layers (which contain historical `claude-pilot` mentions in `docs/solutions/`) are intentionally out of scope per the historical-records exclusion |
| Per-line audit (D2) misclassifies a match (e.g., reads "claude-pilot finishes" as skill-meaning, renames it, breaks app-meaning prose) | Grep-and-annotate verification block in PR description is the auditable artifact; reviewer cross-checks each annotation against the diff |
| Open-issue body sweep (Unit 5) misses tickets filed during the implementation window | Re-run sweep at deploy time (Unit 6 verification) so the sweep is current as of the cutover, not as of when the PR was opened |
| Tool-schema breaking change (Unit 2) propagates to a call site we didn't audit | Inline assertion (Unit 3) catches mika-dev routing; integration smoke (Unit 6) catches end-to-end. Surfaces from any external caller would manifest as schema-validation errors at runtime, surfaceable via logs |
| The `skill:` enum is `["dev-pilot"]` only at deploy time, but mika#845 lands shortly after and the enum needs `["dev-pilot", "dev-groom"]` | Implementer checks mika#845 status before finalizing enum (deferred-to-implementation question). When mika#845 ships, that PR amends the enum |

## Documentation / Operational Notes

- The PR description must contain the per-file grep-and-annotate verification block (D2) and the issue-sweep audit log (Unit 5).
- The deploy log should record the SQL probe result, the cutover decision, and the end-to-end smoke output.
- After this ticket and mika#845 both ship, file a `/ce:compound` doc capturing the "rename a bundled skill" pattern (touchpoints + gates) for future similar work.
- mika-platform's `CLAUDE.md` § Cross-Repo Relationships should be updated in Unit 4 to mention `dev-pilot` as the dispatch skill name (currently says `claude-pilot`).

## Sources & References

- **Origin ticket:** [senara-solutions/mika#844](https://github.com/senara-solutions/mika/issues/844)
- **Companion ticket:** [senara-solutions/mika#845](https://github.com/senara-solutions/mika/issues/845) (dev-groom skill, sprint-bundled)
- **Parent debt:** [senara-solutions/mika#806](https://github.com/senara-solutions/mika/issues/806) — broader callback-handler eval coverage
- **Cross-cutting (companion only):** [senara-solutions/mika#843](https://github.com/senara-solutions/mika/issues/843) — `--verbose` metadata envelope expansion
- **Open-issue sweep example:** [senara-solutions/mika#784](https://github.com/senara-solutions/mika/issues/784) — body uses "claude-pilot handler" in skill-meaning context
- **Architect verdict:** mika-arch session `46084b1a-d873-4e8a-92ae-4e76b0396348` (Disposition: ITERATE, 2026-04-27)
- **Pre-filing scope verification:** `mika-platform/docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md`
- **Plan-on-branch contract:** `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md`
