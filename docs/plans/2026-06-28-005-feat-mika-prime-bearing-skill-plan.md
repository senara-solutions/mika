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

**Sibling coordination note:** The companion ticket (feat: minimal gh_read-only skill replacing github in allowlist) MUST swap `run_gh` → `gh_read` in this file and update Prime's allowlist to include `gh-read-only` instead of `github`. Without this swap, when the allowlist change lands, the `#516` availability filter would silently drop `run_gh` from the effective required set (vacuous gate). The implementer should verify that the sibling's changes land in the same PR or that `run_gh` is still resolvable in Prime's tool set at merge time.

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

### Stage 0→1 gate addition (backstop)

The ticket specifies a SQL-queryable backstop check for Prime's wakes:

```sql
-- For each of Prime's last N wakes on session 00000000-...-000:
-- assert tool_calls(name IN ('run_gh','search_memory','query_knowledge_graph'))
-- preceded the EndTurn that emitted the bearing.
```

This supplements (does not replace) the existing Stage 0→1 parity gate. Implementation is a query pattern for operator verification, not engine code — the `required_tools` gate IS the engine enforcement; this SQL is the observability/audit layer.

**Implementation:** Add this as a documented query pattern in the bearing skill's system prompt or in `docs/solutions/` as a compound doc. No engine code changes needed — the `tool_calls` and `llm_calls` tables already capture the necessary data.

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

The issue body has no `## Acceptance criteria` section; these are derived from the plan's Implementation Steps and Verification (Piece E). Runtime-config items (Pieces C, D) are operator-applied and not CI-gated — they are documented, not asserted here.

1. **Skill manifest exists and is correct.** `skills/bundled/bearing/skill.toml` exists with `[skill].name = "bearing"`, `always_on = false`, `[triggers].keywords` containing the bearing keywords (`status`, `bearing`, `what's next`, `priorities`, `where are we`, …), and `[constraints].required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]`.
2. **System prompt exists with the grounding ritual.** `skills/bundled/bearing/system_prompt.md` exists, mandates calling all three required tools before composing any bearing text, prohibits citing `core_memory` snapshots as world-state ground truth, and specifies the Ground watermark format `Ground: <ISO 8601 timestamp> · gh ✓ <issue count> open · search_memory ✓ · kg ✓ <entity count>` as the first line of a bearing.
3. **Build-time discovery succeeds.** `cargo build` compiles clean; `build.rs` discovers the `bearing` bundle and includes it in `BUNDLED_SKILL_MANIFESTS`.
4. **Structural bundle verification passes.** `make verify-bundled-skills` passes for the new bundle — required files present, manifest parses, `required_tools` tokens resolve, no `tools.json` needed (all required tools are builtins or provided by Prime's existing `github` skill).
5. **No regressions.** `cargo test -p mika-agent` passes with no regressions in skill loading, manifest parsing, or `required_tools` enforcement.
6. **Sibling-coordination note is honored.** The plan documents that the companion `gh_read`-only ticket must swap `run_gh` → `gh_read` in this `required_tools` line within the same PR, else the mika#516 availability filter drops the unavailable tool and the gate passes vacuously.

## Risk Assessment

- **Low risk:** The new skill is additive — no existing code paths change. The `required_tools` enforcement mechanism is proven (used by qa-review, mika-arch-groom-ticket, mika-arch-second-review, etc.)
- **Sibling dependency:** The `run_gh` → `gh_read` swap in the companion ticket is a hard dependency if Prime's allowlist changes from `github` to `gh-read-only`. Without coordination, the `#516` filter silently drops the unavailable tool
- **core_memory cleanup** is a one-time operational step with no rollback risk — `update_core_memory` is audited and rewindable
