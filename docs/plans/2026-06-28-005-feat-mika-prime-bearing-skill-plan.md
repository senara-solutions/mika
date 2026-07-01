# Plan: feat(mika-prime): bearing skill with required_tools grounding constraint + Ground watermark + core_memory de-staling

**Ticket:** mika#1405
**Type:** Feature (engine-coupled bundled skill + operational fix)
**Branch:** `feat/1405/mika-prime-bearing-skill-with-required`

## Problem

Mika Prime's first operator-invoked bearing (2026-06-04T15:07Z) produced a well-shaped bearing on stale ground. She used only introspective tools (`list_skills`, `list_agent_files`) — zero calls to external-state readers (`run_gh`, `search_memory`, `query_knowledge_graph`), all of which were available. The bearing cited stale `core_memory` snapshots as ground truth (90 issues vs actual 94, merged PR cited as in-flight, KG "starvation" claim contradicted by 879-entity library).

Root cause is three-fold: (1) no structural enforcement requiring world-state reads before bearing output, (2) `core_memory` slots containing refreshable world-state that decays into misinformation, (3) no operator-glance freshness signal on bearing output.

## Decision Record

- **Required_tools at skill manifest, not prompt-level.** Per `feedback_prompt_enforcement_fragile`, `prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`, and mika#270 founding incident. Structural enforcement via `[constraints] required_tools` is the established surface.
- **Not `always_on = true`.** Per the qa-review plan's always-on pitfalls note. Bearing keywords scope activation; principle-level refusals don't match bearing keywords.
- **One ticket, one PR for the trio.** The three pieces are causally coupled under one contract: *bearings render on fresh ground*. Splitting risks the first shipping without the third (theater-grounding).

## Implementation Steps

### Piece A — New `bearing` bundled skill

**Files to create:**

#### 1. `skills/bundled/bearing/skill.toml`

```toml
[skill]
name = "bearing"
version = "0.1.0"
description = "Mika Prime's bearing-render skill — produces priority calls and verdicts grounded in current operational state."
always_on = false
timeout_secs = 30

[triggers]
keywords = [
  "status", "mika status", "what's next", "what is next",
  "priority", "priorities", "bearing", "what do you see",
  "read the room", "where are we"
]

[constraints]
required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]
```

**Tool name verification (per ticket provenance):**
- `run_gh` — provided by `github` skill in Prime's allowlist (`~/.mika/agents/mika-prime/skills/github/tools.json`)
- `search_memory` — builtin (`crates/mika-agent/src/tools/mod.rs`)
- `query_knowledge_graph` — builtin, available to KG-enabled agents; Prime's `identity.toml` has `[kg].enabled = true`

**Sibling coordination — resolved dependency order (F1):**

The companion ticket **mika#1406** (minimal `gh-read-only` skill replacing `github` in Prime's allowlist) is **CLOSED, `ready`-labelled, and already GROOMED** (session-id `a6a73ac9-d8a4-4176-a15f-f8e9ef149858`, plan committed on branch `feat/1406/mika-prime-minimal-gh-read-only-skill`). Its closure was a deferred-for-Phase-1 refile decision (Vincent, 2026-06-14) — the work is groomed and dispatch-ready, just not yet dispatched. The earlier "vacuous-pass if mika#1406 lands first" phrasing understated this: mika#1406 could be dispatched concurrently or ahead of mika#1405.

The true coupling direction: **mika#1405 depends on the tool `run_gh` remaining resolvable in Prime's tool set.** mika#1406 removes `run_gh` from Prime by swapping the `github` skill for `gh-read-only` (which provides `gh_read` instead). If mika#1406 lands first, mika#1405's `required_tools = ["run_gh", …]` references a tool Prime no longer has → the mika#516 availability filter silently drops it → vacuous gate.

**Resolution — Option (b), decoupled ship-now + tracked migration.** mika#1405 ships **now** with `required_tools = ["run_gh", …]`, because Prime's allowlist currently carries the `github` skill (which provides `run_gh`) and Piece D retains it. mika#1405 does **not** touch Prime's `github`/`gh-read-only` allowlist swap — that is mika#1406's job. The two tickets are decoupled, not same-PR-coupled.

This introduces one hard, ordered obligation: **when mika#1406 is dispatched and its `github` → `gh-read-only` swap lands, a coordinated follow-up MUST swap `run_gh` → `gh_read` in `skills/bundled/bearing/skill.toml` in lockstep.** Because mika#1406 is a closed ticket on a separate branch, this cannot be a same-PR edit here. The implementer of mika#1405 MUST file that follow-up ticket at merge time (title e.g. "feat(bearing): swap `run_gh` → `gh_read` in required_tools when mika#1406 lands") and cross-reference it from mika#1406's body, so the ordering is not lost. Until mika#1406 dispatches, `run_gh` is resolvable and the gate is live (non-vacuous).

Why not (a) block-on-1406 or (c) same-PR bundle: (a) needlessly gates a ready fix on an undispatched ticket, contradicting the "ship the grounded fix now" intent; (c) is structurally impossible — mika#1406 is closed on a different branch, so there is no single PR to bundle into. Option (b) is the only order that ships mika#1405's grounding fix without a vacuous gate and without waiting on mika#1406's dispatch. (Citation: review-guide.md § Dependency Management / "Unordered cross-ticket coupling is a dispatch hazard".)

#### 2. `skills/bundled/bearing/system_prompt.md`

Content — bearing-render instructions with Ground watermark:

```markdown
## Bearing — Operational Ground-Truth Render

You are rendering a bearing: a priority-ordered snapshot of current operational state with verdicts.

### Ground Rule (structural, non-negotiable)

Before composing ANY bearing text, you MUST call all three tools:

1. **`run_gh`** — fetch current issue counts, open PRs, milestone state
2. **`search_memory`** — retrieve recent decisions, commitments, blockers
3. **`query_knowledge_graph`** — check entity counts, resolution coverage, library state

Do NOT use `core_memory` snapshots as ground truth for world state. `core_memory` is for stable identity facts (who, what, why), not refreshable state (issue counts, PR status, KG coverage). The tools above are the only source of current operational reality.

### Output Format

Begin every bearing with a single Ground watermark line:

```
Ground: <ISO 8601 timestamp> · gh ✓ <issue count> open · search_memory ✓ · kg ✓ <entity count>
```

This is an operator-glance freshness signal, not a report. The full tool-call trail is in the trace.

### Bearing Shape

After the Ground line, produce the bearing in your established shape:
- Priority calls with evidence citations from the tool results
- Diverge / converge structure where appropriate
- Named traps and trade-offs grounded in fetched state
- Verdicts with confidence levels tied to evidence freshness

### What NOT to do

- Do not cite `core_memory` issue counts, PR states, or KG statistics as current fact
- Do not skip any of the three required tool calls
- Do not produce a bearing if any required tool call fails — report the failure instead
```

**Note:** The `required_tools` gate in `skill.toml` is the actual structural enforcement. The system prompt describes the ritual and the Ground watermark format. This is Layer-3 documentation per `prompt-enforcement-structural-guards.md` — the engine gate is the control, the prompt is the description.

### Piece B — Ground watermark

Covered by the `system_prompt.md` above. The watermark is prompt-instructed, not engine-enforced. The structural enforcement is the `required_tools` gate — if the tools aren't called, the engine rejects the response before any watermark is evaluated.

### Piece C — core_memory de-staling

**Operational step (not code).** Audit Mika Prime's current `core_memory` slots and strip world-state items:

Target slots to audit (seeded 2026-06-03T21:28Z and 2026-06-04T09:49Z):
- `current_priorities` — strip issue counts ("mika core: 90 ungroomed"), specific PR references with stated states ("PR#1400 in-flight")
- Any slot containing KG/library "starvation" claims — these are world-state, not workflow-state

Keep:
- `self_model` — stable identity facts
- `user_summary` — stable user context
- `key_people` — stable personnel facts
- `workflows` — discipline encoding, not state

**Implementation:** Use `update_core_memory` tool (or direct DB update via mika-spirit API) to:
1. Read current `core_memory` content for each slot
2. Identify refreshable world-state items
3. Rewrite slots to contain only stable facts, removing specific counts, PR references, and temporal state claims

This is a one-time operational cleanup. The bearing skill's `required_tools` constraint prevents future stale-ground recurrence by ensuring fresh tool calls before every bearing response.

**Verification (F2).** Because this is an operational (not CI-gated) step, it needs an explicit success criterion and a verification query so the operator can confirm completion and detect an incomplete cleanup.

Slots to audit (exact names): `current_priorities`, and any slot carrying KG/library "starvation" claims (seeded 2026-06-03T21:28Z and 2026-06-04T09:49Z — grep the slots for the tokens below to locate them; do not assume a fixed slot name for the starvation claim).

Expected post-cleanup state — the audited slots contain **no refreshable world-state tokens**:
- no issue counts (e.g. "90 ungroomed", "94 open")
- no PR references with stated states (e.g. "PR#1400 in-flight", "merged")
- no KG/library entity counts or coverage figures (e.g. "879 entities", "starvation")
- no other temporal state claims (dates-as-fact about current backlog, milestone counts)

Verification query (run against Prime's DB after cleanup):

```sql
SELECT slot, content
FROM core_memory
WHERE agent_id = 'mika-prime'
  AND slot IN ('current_priorities', 'kg_coverage', 'current_state');
```

Success criterion: for every returned row, `content` matches none of the world-state token patterns above (issue counts, PR-state references, entity/coverage counts). A row that still contains any such token means the cleanup is incomplete — re-run the rewrite for that slot. Rollback: `update_core_memory` writes are audited and rewindable, so a bad rewrite can be reverted via the audit trail (no data loss).

### Piece D — Allowlist update for Prime

Add `"bearing"` to Prime's skill allowlist in `~/.mika/agents/mika-prime/identity.toml`:

```toml
[skills]
allowlist = [
  "shell-exec",
  "web-search",
  "file-reader",
  "self-knowledge",
  "tmux",
  "git-ops",
  "github",
  "google-workspace",
  "mcp",
  "browser-control",
  "self-check",
  "agents-teams",
  "bearing",
]
```

**Note:** This is a runtime configuration change, not a code change. The identity.toml is per-agent config, not checked into the repo. The bundled skill itself (`skills/bundled/bearing/`) is the code change.

### Piece E — Verification

1. **`make verify-bundled-skills`** — Verify the new bundle is structurally complete (required files, manifest parses, no `tools.json` needed since all required tools are builtins or provided by other skills)
2. **`cargo build`** — Verify build-time discovery picks up the new skill via `build.rs`
3. **`cargo test -p mika-agent`** — Verify no regressions in skill loading, manifest parsing, required_tools enforcement

### Post-deployment verification checklist — grounding audit (manual, not a gate)

**F3 correction.** An earlier draft framed this SQL as a "Stage 0→1 gate addition (backstop)" that "supplements (does not replace)" the existing parity gate. That framing was theater: a documented query with no automation is not a gate. Per review-guide.md § Theater vs. Structure ("Documentation of a check without automation is theater"), this is reclassified honestly.

**The only structural enforcement in this ticket is the `required_tools` gate in `skill.toml`.** The engine rejects any bearing EndTurn that did not call all three required tools. That is the control. The SQL below is **manual audit / observability only** — an operator-run spot-check, not a CI job and not an engine backstop. It is not wired into any automated pipeline, and this plan makes no claim that it is.

Post-deployment, the operator MAY run this against Prime's DB to confirm the gate is behaving in practice:

```sql
-- Manual audit — for each of Prime's last N wakes on session 00000000-...-000:
-- confirm tool_calls(name IN ('run_gh','search_memory','query_knowledge_graph'))
-- preceded the EndTurn that emitted the bearing.
-- Observability only; the required_tools gate is the actual enforcement.
```

No engine code changes and no CI wiring are in scope — the `tool_calls` and `llm_calls` tables already capture the data for an ad-hoc audit. If Vincent later wants this automated, that is a separate ticket (a scripted query against a test DB), explicitly out of scope here.

**Could not address F3(a) in-scope:** Adding this as an actual CI check would require a test DB seeded with Prime's real wake history (Prime is manually provisioned, not in the eval harness), which is beyond this additive-skill ticket. Per F3's own Option (b), the plan instead moves the check to this manual post-deployment checklist and removes the "supplement not replace" claim — the honest classification, deferring automation to a future ticket if desired.

## Files Changed (code)

| File | Action | Description |
|------|--------|-------------|
| `skills/bundled/bearing/skill.toml` | Create | Skill manifest with `required_tools` constraint |
| `skills/bundled/bearing/system_prompt.md` | Create | Bearing-render instructions with Ground watermark format |

## Files Changed (operational, not code)

| File | Action | Description |
|------|--------|-------------|
| `~/.mika/agents/mika-prime/identity.toml` | Edit | Add `"bearing"` to `[skills].allowlist` |
| `~/.mika/agents/mika-prime/core_memory/*` | Edit | Strip refreshable world-state from `current_priorities` and other affected slots |

## Out of Scope

- Engine changes (`agent.rs`, tool registry) — the `required_tools` surface is already built
- Soul edits — operator-owned
- Skill allowlist changes beyond adding `bearing` — the sibling ticket handles `github` → `gh-read-only` swap
- `captured_at` watermark machinery on `core_memory` entries — YAGNI per ticket
- Cloud transposition adjustments
- Well-known agent provisioning changes — Prime is manually provisioned, not in `well_known_agents.rs`

## Definition of Done

- The `bearing` bundled skill exists at `skills/bundled/bearing/` with `skill.toml` and `system_prompt.md`, discovered at build time by `build.rs`.
- `skill.toml` carries `[constraints] required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]`, `always_on = false`, and the bearing trigger keywords.
- `system_prompt.md` instructs the three required tool calls before any bearing text and documents the Ground watermark line format.
- `make verify-bundled-skills`, `cargo build`, and `cargo test -p mika-agent` all pass.
- Operational steps (Prime allowlist add, `core_memory` de-staling) documented in the plan for the operator to apply post-merge (runtime config, not CI-verified).

## Acceptance criteria

The issue body has no `## Acceptance criteria` section; these are derived from the plan's Implementation Steps and Verification (Piece E). Runtime-config items (Pieces C, D) are operator-applied and not CI-gated. They are still asserted here with operator-verifiable criteria (AC6 for the sibling dependency, AC7 for the `core_memory` de-staling query) — "not CI-gated" does not mean "unverifiable."

1. **Skill manifest exists and is correct.** `skills/bundled/bearing/skill.toml` exists with `[skill].name = "bearing"`, `always_on = false`, `[triggers].keywords` containing the bearing keywords (`status`, `bearing`, `what's next`, `priorities`, `where are we`, …), and `[constraints].required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]`.
2. **System prompt exists with the grounding ritual.** `skills/bundled/bearing/system_prompt.md` exists, mandates calling all three required tools before composing any bearing text, prohibits citing `core_memory` snapshots as world-state ground truth, and specifies the Ground watermark format `Ground: <ISO 8601 timestamp> · gh ✓ <issue count> open · search_memory ✓ · kg ✓ <entity count>` as the first line of a bearing.
3. **Build-time discovery succeeds.** `cargo build` compiles clean; `build.rs` discovers the `bearing` bundle and includes it in `BUNDLED_SKILL_MANIFESTS`.
4. **Structural bundle verification passes.** `make verify-bundled-skills` passes for the new bundle — required files present, manifest parses, `required_tools` tokens resolve, no `tools.json` needed (all required tools are builtins or provided by Prime's existing `github` skill).
5. **No regressions.** `cargo test -p mika-agent` passes with no regressions in skill loading, manifest parsing, or `required_tools` enforcement.
6. **Sibling dependency order is resolved and tracked (F1).** The plan documents that mika#1406 is CLOSED/GROOMED/`ready` on branch `feat/1406/mika-prime-minimal-gh-read-only-skill`, and that mika#1405 ships decoupled with `run_gh` (Option b). At merge time, the implementer files a follow-up ticket to swap `run_gh` → `gh_read` in `skills/bundled/bearing/skill.toml` **in lockstep with mika#1406's `github` → `gh-read-only` allowlist swap**, and cross-references it from mika#1406's body. Verification: the follow-up ticket exists and is linked before this PR merges; until mika#1406 dispatches, `run_gh` remains resolvable in Prime's tool set (gate is non-vacuous).

7. **core_memory de-staling is verifiable (F2).** After the operator applies Piece C, the verification query (`SELECT slot, content FROM core_memory WHERE agent_id = 'mika-prime' AND slot IN ('current_priorities', 'kg_coverage', 'current_state')`) returns rows whose `content` contains **none** of the world-state token classes: issue counts, PR-state references, KG/library entity or coverage counts, or other temporal backlog claims. This is an operator-applied, operator-verified criterion (not CI-gated), but the plan specifies the exact slots, the expected post-cleanup state, the verification query, and the rollback path (audited `update_core_memory` rewind).

8. **Stage 0→1 grounding audit is honestly framed (F3).** The plan classifies the SQL tool-call-precedence check as a **post-deployment manual audit** (operator-run against Prime's DB), not a CI gate or engine backstop, and drops any "supplement not replace" claim that implied automated enforcement. The sole structural enforcement is the `required_tools` gate in `skill.toml`; the SQL is observability-only and labelled as such.

## Risk Assessment

- **Low risk:** The new skill is additive — no existing code paths change. The `required_tools` enforcement mechanism is proven (used by qa-review, mika-arch-groom-ticket, mika-arch-second-review, etc.)
- **Sibling dependency (resolved, F1):** mika#1406 is CLOSED/GROOMED/`ready` on branch `feat/1406/mika-prime-minimal-gh-read-only-skill`. mika#1405 ships decoupled with `run_gh` (Option b in the Sibling coordination note). The residual risk is a **lost migration**: if mika#1406 later swaps Prime's `github` → `gh-read-only` without the lockstep `run_gh` → `gh_read` edit in `bearing/skill.toml`, the mika#516 filter silently drops the unavailable tool and the gate passes vacuously. Mitigated by the mandatory follow-up ticket filed at merge time and cross-referenced from mika#1406's body (see AC6)
- **core_memory cleanup** is a one-time operational step with no rollback risk — `update_core_memory` is audited and rewindable

## Revision history

- rev 2 (2026-07-01): addressed first-pass architect findings (findings-1.md).
  - **F1 (BLOCKING)** — corrected the mika#1406 mischaracterization: it is CLOSED/GROOMED/`ready` on branch `feat/1406/mika-prime-minimal-gh-read-only-skill` (deferred-for-Phase-1 refile, Vincent 2026-06-14), not "filed but pending." Resolved the dependency direction as **Option (b)**: mika#1405 ships decoupled now with `run_gh` (Prime retains the `github` skill), with a mandatory follow-up ticket to swap `run_gh` → `gh_read` in lockstep with mika#1406's `github` → `gh-read-only` allowlist swap. Ruled out (a) block-on-1406 and (c) same-PR bundle (impossible — mika#1406 is closed on a separate branch). Updated the Sibling coordination note, AC6, and the Risk Assessment sibling bullet. (review-guide.md § Dependency Management.)
  - **F2 (BLOCKING)** — added a verification mechanism to Piece C `core_memory` de-staling: exact slots to audit, expected post-cleanup state (no issue counts / PR-state refs / KG counts / temporal claims), the verification SQL query, the success criterion, and the audited-rewind rollback path. Added AC7. (mika#1559 Acceptance-Criteria Gate.)
  - **F3 (sharpening)** — reclassified the "Stage 0→1 gate addition (backstop)" as a **manual post-deployment audit checklist** per F3 Option (b); dropped the "supplement not replace" claim (theater). Documented that the `required_tools` gate is the sole structural enforcement and that CI automation is out of scope (deferred to a future ticket, since Prime is manually provisioned and not in the eval harness). Added AC8. (review-guide.md § Theater vs. Structure.)
