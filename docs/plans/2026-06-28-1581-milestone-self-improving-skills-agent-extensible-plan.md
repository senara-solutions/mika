---
title: "Milestone #1581 sequencing record"
type: milestone-sequencing
milestone: senara-solutions/mika#1581
date: 2026-06-28
status: active
---

# Milestone #1581 — Self-Improving Skills: Agent-Extensible Skill Set Under Staged-Then-Promote Lifecycle

## Issue
https://github.com/senara-solutions/mika/issues/1581

## Summary

Mika extends its own skill set at runtime via a three-sub-issue sequence: (1) an authoring tool + lifecycle state column, (2) a nudge that suggests the agent author skills, (3) a curator that proposes archiving stale skills. All authored skills land `staged` and require operator promotion — the guard chain is never agent-authored.

## Sub-issues

- #1582: `skill_manage` builtin authoring tool + `lifecycle_state` column on `skill_overrides` (priority: p0, plan: docs/plans/2026-06-28-1582-feat-skill-manage-builtin-lifecycle-state-plan.md, branch: feat/1582/skill-manage-lifecycle-state)
- #1583: Nudge-driven skill creation — turn-end advisory injection, identity-gated default off (priority: p1, plan: docs/plans/2026-06-28-1583-feat-nudge-driven-skill-creation-plan.md, branch: feat/1583/nudge-driven-skill-creation)
- #1584: Curator background task — archive + rollback, never auto-promote (priority: p1, plan: docs/plans/2026-06-28-1584-feat-curator-background-task-plan.md, branch: feat/1584/curator-background-task)

## Dependencies

- #1582 → #1583: nudge presupposes the `skill_manage` tool and `lifecycle_state` column exist
- #1583 → #1584: curator acts on skills accumulated via the nudge; curator's usage tracking depends on skills being authored and injected
- #1580 (external spike) → #1583 autonomous-loop expansion: identity allowlist expansion to mika-dev/qa/arch blocked by guard-check assertability finding

## Recommended GitHub `blockedBy` edits

- #1583 blockedBy #1582: `skill_manage` tool + `lifecycle_state` column are prerequisites for the nudge
- #1584 blockedBy #1583: curator needs skills accumulated via nudge to have data to curate

## Order

1. #1582 — foundation (skill_manage + lifecycle_state)
2. #1583 — nudge (turn-end advisory injection)
3. #1584 — curator (archive + rollback)

Strict serial. No parallelism. Each ships as its own PR.

## Cross-cutting concerns

- **Schema migration ordering**: #1582 adds `lifecycle_state TEXT` to `skill_overrides` (v42 → v43). #1584 adds `use_count INTEGER NOT NULL DEFAULT 0` and `last_used_at TEXT` to the same table (v43 → v44). The v43 → v44 migration must reference the v43 schema shape. Since sub-issues ship serially with PR merges between, each migration sees the prior one's committed state — no coordination issue beyond merge ordering.
- **`apply_overrides()` eviction predicate** (`crates/mika-agent/src/skills/mod.rs:644`): #1582 extends the Phase 0 eviction predicate to also evict when `lifecycle_state IN ('staged', 'archived')`. #1583 and #1584 do not further modify this site — they read the lifecycle_state but don't change the eviction logic.
- **`SkillsIdentityConfig`** (`crates/mika-agent/src/prompt.rs:120`): #1582 adds `allow_authoring: Option<bool>`. #1583 adds `nudge_enabled: Option<bool>` and `nudge_interval: Option<u32>`. Both are additive fields on the same struct — no merge conflict risk since they ship serially.
- **Well-known agent identity templates** (`crates/mika-agent/src/well_known_agents.rs`): #1582 adds `allow_authoring = false` to all four well-known agents. #1583 adds `nudge_enabled = false` and `nudge_interval` defaults. Both touch the TOML string literals for the same agents but at different `[skills]` keys — serial merge prevents conflicts.
- **Surface-hierarchy bright line**: None of the three sub-issues may author EndTurn guards, modify `[constraints] required_tools` on bundled skills, or change identity-template `[tools].disabled`. The guard chain (Surface 3) is structurally off-limits. AC3 of the milestone enforces this.

## Open milestone-level questions

- **Guard-check automation horizon**: The paired finding-spike (#1580) determines what fraction of "authored skill doesn't weaken the chain" is mechanically checkable. Until resolved, promotion is operator-only (Phase 1 design). This does not block any of the three sub-issues — it blocks the *expansion* of #1583's nudge identity-allowlist to autonomous-loop agents.
- **Curator auto-archive threshold**: #1584 ships propose-only. Whether to add auto-archive at 90+ days idle with `use_count = 0` is a follow-on tuning ticket gated by observation data from Phase 1 rollout.

---

## Per-Sub-Issue Plans

### Sub-issue #1582: `skill_manage` builtin authoring tool + `lifecycle_state` column

#### Sites

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` (schema) | Schema migration v42 → v43: `ALTER TABLE skill_overrides ADD COLUMN lifecycle_state TEXT CHECK(lifecycle_state IN ('staged', 'active', 'archived'))`. NULL = active (no backfill). |
| `crates/mika-agent/src/db.rs` (SkillOverride) | Add `lifecycle_state: Option<String>` to `SkillOverride` struct (line ~337). Extend `get_skill_overrides()` SELECT to include `lifecycle_state`. |
| `crates/mika-agent/src/db.rs` (new methods) | `set_skill_lifecycle_state(agent_id, skill_name, state) -> Result<()>` — atomic UPDATE on `skill_overrides`. `get_skill_lifecycle_state(agent_id, skill_name) -> Result<Option<String>>`. |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for the two new DB methods. |
| `crates/mika-agent/src/skills/mod.rs` | Extend `apply_overrides()` (line ~644) Phase 0 eviction predicate: evict when `enabled == Some(false)` OR `lifecycle_state.as_deref() == Some("staged")` OR `lifecycle_state.as_deref() == Some("archived")`. The eviction already uses `retain()` — add the lifecycle_state check to the same closure. |
| `crates/mika-agent/src/tools/skill_manage.rs` (new) | New builtin tool implementing `create`, `update`, `inspect` actions. `create`: validate manifest via `validate_skill()`, write files atomically under `<agent_home>/skills/<name>/`, insert `skill_overrides` row with `lifecycle_state = 'staged'`. `update`: same but replaces existing files, resets lifecycle_state to `'staged'`. `inspect`: read-only, returns `{lifecycle_state, files, validation_warnings, last_updated}`. Identity-gated: checks `identity.skills.allow_authoring` before executing any action. |
| `crates/mika-agent/src/tools/mod.rs` | Register `skill_manage` in `default_tools()` (line ~777). Add to `BUILTIN_TOOL_NAMES`. |
| `crates/mika-agent/src/prompt.rs` | Add `allow_authoring: Option<bool>` to `SkillsIdentityConfig` (line ~120), default `None` (= false). |
| `crates/mika-agent/src/well_known_agents.rs` | Add `allow_authoring = false` to all four well-known agent identity TOML templates (MIKA_DEV_IDENTITY, MIKA_QA_IDENTITY, MIKA_RELAY, and the `build_mika_arch_identity()` computed template). |
| `crates/mika-cli/src/cli.rs` | Add `Promote { name }` and `Archive { name }` variants to `SkillsCommand` enum (line ~460). |
| `crates/mika-cli/src/commands/skills.rs` | Implement `promote` and `archive` subcommand handlers calling `set_skill_lifecycle_state()`. |
| `crates/mika-agent/src/server/handlers.rs` | Add `POST /api/v1/skills/{name}/promote` and `POST /api/v1/skills/{name}/archive` endpoints. Operator-tier auth (internal token). Structured JSON response. |

#### Atomicity of file writes

`skill_manage(action="create")` writes files under `<agent_home>/skills/<name>/`. To prevent partial writes:

1. Write to a temp directory `<agent_home>/skills/.<name>.tmp/`
2. Validate the skill via `validate_skill()`
3. If validation passes, rename (atomic on same filesystem) to `<agent_home>/skills/<name>/`
4. Insert the `skill_overrides` row
5. Set `skills_dirty` flag so the registry reloads on next turn

If the DB insert fails after rename, the skill directory exists but has no activation row — it will be discovered by `scan_skills_dir` but filtered out by `apply_overrides()` since no `lifecycle_state = 'active'` row exists. Safe failure mode.

#### Tests

- **Migration test**: v42 → v43 migration on a fixture with existing `skill_overrides` rows. Assert all rows have `lifecycle_state IS NULL` post-migration.
- **Eviction predicate equivalence test**: Given a fixture where all `skill_overrides` rows have NULL `lifecycle_state`, assert `apply_overrides()` produces the same eviction set as pre-change behavior.
- **Eviction predicate extension test**: Given rows with `lifecycle_state = 'staged'` and `'archived'`, assert those skills are evicted. Given `lifecycle_state = 'active'` or NULL, assert those skills are NOT evicted.
- **`skill_manage` tool test**: Create a skill via the tool, verify files on disk, verify `skill_overrides` row with `lifecycle_state = 'staged'`, verify resolver does NOT inject it. Promote via `set_skill_lifecycle_state`, verify resolver injects it. Archive, verify resolver stops injecting.
- **Identity gate test**: Agent with `allow_authoring = false` (default) gets an error from `skill_manage`. Agent with `allow_authoring = true` succeeds.
- **CLI test**: `mika skills promote` and `mika skills archive` produce correct DB state transitions.

---

### Sub-issue #1583: Nudge-driven skill creation

#### Sites

| File | Change |
|------|--------|
| `crates/mika-agent/src/prompt.rs` | Add `nudge_enabled: Option<bool>` (default `None` = false) and `nudge_interval: Option<u32>` (default `None` = 10) to `SkillsIdentityConfig` (line ~120). Add validation: `nudge_interval == Some(0)` is rejected at identity load time with an error. |
| `crates/mika-agent/src/agent_loop/mod.rs` | Add two per-turn atomics to the loop state (local to `run_loop`, not on `AgentState`): `iters_since_skill_nudge: u32` and `pending_skill_nudge: bool`. Increment `iters_since_skill_nudge` after each step that executed at least one tool call. At EndTurn acceptance, check: `nudge_enabled && nudge_interval > 0 && iters_since_skill_nudge >= nudge_interval && tool_registry.has_tool("skill_manage")` → set `pending_skill_nudge = true`, reset counter to 0. |
| `crates/mika-agent/src/prompt.rs` or `agent_loop/mod.rs` | When `pending_skill_nudge` is true at prompt assembly (the system prompt string is built before the LLM call at `inject_skills_and_resolve_tools` line ~4901), append the `<skill-nudge priority="advisory">` block. Clear `pending_skill_nudge` after injection. |
| `crates/mika-agent/src/well_known_agents.rs` | Add `nudge_enabled = false` to all four well-known agent identity TOML templates. |

#### Nudge block content

```xml
<skill-nudge priority="advisory">
You have completed roughly {interval} tool-invoking turns since the last
skills review. If a recent task pattern is worth extracting into a reusable
skill, consider calling `skill_manage(action="create" | "update" | "inspect")`
this turn. The skill will land `staged` and require operator promotion before
it activates — your authoring is advisory, not load-bearing. If no pattern
stands out, ignore this nudge and proceed normally.
</skill-nudge>
```

#### Design notes

- Counter and nudge state are **per-`run_loop` invocation**, not cross-session. Each conversation turn or silent trigger starts fresh. This is intentional — the nudge counts within a single agent session, not across sessions.
- The nudge fires **after** EndTurn acceptance, affecting the **next** turn's prompt. It never fires mid-turn.
- `nudge_interval == Some(0)` is invalid configuration. The `nudge_enabled` boolean is the sole on/off gate. Validated at identity load time (deserialization).
- Counter-reset on no-action is intentional. The agent can decline; the counter resets and waits another interval.

#### Tests

- **Nudge off by default**: Agent with `nudge_enabled = false` (or absent), counter at 100 → no `<skill-nudge>` block in system prompt.
- **Nudge fires at threshold**: Agent with `nudge_enabled = true`, `nudge_interval = 3`. After 3 tool-invoking steps, EndTurn → next prompt has `<skill-nudge>` block. Counter resets to 0.
- **Nudge requires skill_manage**: Agent with `nudge_enabled = true` but `allow_authoring = false` (so `skill_manage` is not in the tool registry) → no nudge even if counter >= interval.
- **nudge_interval = 0 rejected**: Identity with `nudge_interval = 0` fails deserialization with a validation error.
- **Non-tool-invoking turns don't count**: Steps with zero tool calls do not increment the counter.

---

### Sub-issue #1584: Curator background task

#### Sites

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` (schema) | Schema migration v43 → v44: `ALTER TABLE skill_overrides ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0` and `ALTER TABLE skill_overrides ADD COLUMN last_used_at TEXT`. |
| `crates/mika-agent/src/db.rs` (SkillOverride) | Add `use_count: i64` and `last_used_at: Option<String>` to `SkillOverride` struct. |
| `crates/mika-agent/src/db.rs` (new methods) | `increment_skill_use_counts(agent_id, skill_names: &[String]) -> Result<()>` — batch UPDATE within one transaction, sets `use_count = use_count + 1` and `last_used_at = now()` for each named skill. `get_curator_candidates(agent_id, max_idle_days: u32) -> Result<Vec<CuratorCandidate>>` — selects `lifecycle_state = 'active'` AND (`last_used_at IS NULL OR last_used_at < now - max_idle_days`). Excludes NULL `lifecycle_state` rows (bundled/marketplace). |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for the new DB methods. |
| `crates/mika-agent/src/skills/mod.rs` | At the site where skill prompts are injected into the system prompt, collect the set of injected skill names. Emit them at turn-end via the async DB batch update. This is the `use_count` increment site. |
| `crates/mika-agent/src/skills/curator.rs` (new) | `CuratorReview` logic: query candidates, build structured proposals, emit `curator_proposal` structured log event. Snapshot capture: tar.gz of skill directory to `<agent_home>/skills/.archived/<name>-<timestamp>.tar.gz` before archival. Rollback: extract tarball, update `lifecycle_state` to `'staged'`. |
| `crates/mika-agent/src/agent_loop/mod.rs` | Add `SilentTrigger::CuratorReview` variant. `max_steps()` returns `MAX_TOOL_STEPS` (same as heartbeat/reflection). Register in the task engine with configurable interval (default: 86400s = 24h). |
| `crates/mika-cli/src/cli.rs` | Add `Restore { name }` and `CuratorStatus` variants to `SkillsCommand`. |
| `crates/mika-cli/src/commands/skills.rs` | Implement `restore` and `curator status` subcommand handlers. `restore` extracts the most recent snapshot, UPDATEs `lifecycle_state` to `'staged'`. `curator status` queries the most recent `curator_proposal` structured log event. |
| `crates/mika-agent/src/server/handlers.rs` | Add `POST /api/v1/skills/{name}/restore` endpoint. |

#### Usage tracking debounce

Per-turn batching: collect all skill names injected during prompt assembly into a `Vec<String>`, then at turn-end (after `save_tool_call` and before the next iteration), call `increment_skill_use_counts(agent_id, &skill_names)` once. This produces one DB transaction per turn regardless of how many skills were injected.

#### Snapshot storage

- Location: `<agent_home>/skills/.archived/<skill_name>-<YYYY-MM-DD-HHMMSS>.tar.gz`
- `.archived/` is both dot-prefixed and structurally invisible to `is_bundled_skill_dir()` (line 460: rejects dot and underscore prefixes)
- Snapshots accumulate without auto-pruning in Phase 1. Operator can `rm` manually. A `max_archived_snapshots_per_skill` config is a follow-on.

#### Curator candidate query

```sql
SELECT skill_name, use_count, last_used_at
FROM skill_overrides
WHERE agent_id = ?
  AND lifecycle_state = 'active'
  AND (last_used_at IS NULL OR last_used_at < datetime('now', '-' || ? || ' days'))
```

The `lifecycle_state = 'active'` clause is the structural exclusion for bundled/marketplace skills (which have NULL `lifecycle_state`). This is the load-bearing safety predicate.

#### Tests

- **Zero candidates on fresh agent**: No authored skills → zero candidates.
- **Staged skill not a candidate**: Authored skill with `lifecycle_state = 'staged'` → zero candidates (curator only considers active).
- **Active + idle skill is a candidate**: Authored, promoted, `last_used_at` 31+ days ago → exactly one candidate.
- **Bundled/marketplace excluded**: NULL `lifecycle_state` + NULL `last_used_at` → zero candidates regardless of age.
- **use_count increments on injection**: Skill injected into system prompt → `use_count` incremented, `last_used_at` updated.
- **Snapshot capture and restore**: Archive a skill → verify tar.gz exists. Restore → verify files back in place, `lifecycle_state = 'staged'`.
- **Curator proposal structured output**: Curator tick emits JSON proposal list (not free-text).

---

## Verification checklist (milestone-level AC)

- [ ] AC1: All three sub-issues land in order, each in its own PR, passing autonomous loop verdict + CI.
- [ ] AC2: End-to-end: authorized agent authors skill → lands `staged` → operator promotes → resolver injects → curator can archive → resolver stops injecting.
- [ ] AC3: No sub-issue PR adds an agent-authorable path to the guard chain (EndTurn guards, `[constraints]`, identity `[tools].disabled`).
- [ ] AC4: Nudge identity-allowlist defaults off; mika-dev/qa/arch blocked by #1580 spike outcome.
