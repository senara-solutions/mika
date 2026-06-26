# Plan: Fix In-Memory Skill Registry Drift (mika#1326)

**Type:** bug fix
**Issue:** [mika#1326](https://github.com/senara-solutions/mika/issues/1326)
**Date:** 2026-05-29
**Branch:** `bug/1326/skills-in-memory-skill-registry-drifts`

## Problem Statement

The in-memory `SkillRegistry` can drift from on-disk skill manifests, causing `build_skill_tool_map()` to produce a `HashMap<String, &ResolvedSkillTool>` with stale entries. The concrete incident: mika-qa's `run_gh` tool was silently shadowed for ~3 hours when the `github` skill's Builtin entry for `run_gh` was overwritten by a stale Exec entry from `qa-review` in the skill_tools HashMap. On-disk state was never inconsistent — the drift was purely in-memory.

**Root cause:** `build_skill_tool_map()` at `agent.rs:4418` uses `HashMap::collect()` which is silent last-write-wins on name collision. When two `SkillEntry` objects have `skill_tools` with the same tool name but different handler types, the iteration order of `matched_entries` determines which wins — with no warning, no deterministic precedence, and no diagnostic.

## Acceptance Criteria (from ticket)

1. **AC1 — HashMap collision detection:** `build_skill_tool_map` emits a `WARN` log on every name collision
2. **AC2 — Build-time invariant check:** Unit test or CI gate verifying no two bundled skills' `tools.json` files declare the same tool name with different handler types
3. **AC3 — Registry-drift detection:** Periodic invariant check verifying in-memory `SkillEntry.skill_tools` matches what `load_tools_json(skill_dir)` would return
4. **AC4 — Regression test:** Test with name-colliding tools asserting WARN emission and deterministic precedence
5. **AC5 — Recovery without restart:** Operator can force registry reload without restarting mika-server

## Technical Approach

### Implementation Unit 1: Collision-Aware `build_skill_tool_map` (AC1, AC4)

**File:** `crates/mika-agent/src/agent.rs`

Replace the current silent `HashMap::collect()` pattern with explicit insertion that detects and logs collisions:

```rust
fn build_skill_tool_map<'a>(matched: &[&'a SkillEntry]) -> HashMap<String, &'a ResolvedSkillTool> {
    let mut map = HashMap::new();
    for entry in matched {
        for st in &entry.skill_tools {
            if let Some(existing) = map.insert(st.definition.name.clone(), st) {
                let existing_skill = matched.iter()
                    .find(|e| e.skill_tools.iter().any(|t| std::ptr::eq(t, existing)))
                    .map(|e| e.manifest.skill.name.as_str())
                    .unwrap_or("unknown");
                warn!(
                    tool = %st.definition.name,
                    winner_skill = %entry.manifest.skill.name,
                    winner_handler = ?st.handler,
                    loser_skill = %existing_skill,
                    loser_handler = ?existing.handler,
                    "skill tool name collision — last-write-wins shadowing"
                );
            }
        }
    }
    map
}
```

**Precedence rule:** For this unit, keep last-write-wins but make it observable. The precedence is determined by `matched_entries` ordering (which comes from `match_skills()` — keyword-matched first, then always_on, then dependencies). This is the minimum-risk fix; AC1 and AC4 are satisfied by making collisions visible.

**Considered alternative — Builtin-wins-over-Exec:** A rule where `ToolHandler::Builtin` always takes precedence over `ToolHandler::Exec` on name collision would have prevented the specific incident. However, this adds complexity (what about Exec-vs-Exec? two Builtins?) and the real fix is preventing collisions in the first place (IU2) + detecting drift (IU3). Defer to a follow-up if collision logging reveals this is needed.

**Tests (AC4):** Update `test_build_skill_tool_map_last_skill_wins_on_collision` at `agent.rs:6137` to also assert the WARN log is emitted. Add a new test `test_build_skill_tool_map_collision_logs_both_skills` that creates two skills with a shared tool name (one Builtin, one Exec) and verifies the structured log fields.

### Implementation Unit 2: Build-Time Tool Name Uniqueness Invariant (AC2)

**File:** `crates/mika-agent/src/bundled_skills.rs`

Add a new test `test_bundled_skills_no_cross_skill_tool_name_collision` near the existing `test_engine_referenced_tool_names_are_loader_reachable` (line ~1425). This test:

1. Iterates all entries from `all_bundled_skills()`
2. For each skill, parses its embedded `tools.json` content (from `BundledSkillEntry.tools_json`)
3. Builds a `HashMap<String, Vec<(String, ToolHandler)>>` of tool_name → [(skill_name, handler_type)]
4. Asserts no tool name has entries from more than one skill
5. On failure, prints the conflicting skills and handler types

This is a compile-time invariant (runs as `cargo test`) that catches tool name collisions introduced by new bundled skills before they reach production. The test uses the same embedded manifest data that the runtime loads, so it's authoritative.

**Scope:** Bundled skills only. Community/marketplace skills are user-installed and validated at install time (see IU2b below).

**IU2b — Install-time collision detection (stretch):**

**File:** `crates/mika-agent/src/skills/index.rs` or `crates/mika-cli/src/commands/skills.rs`

When `scan_skills_dir` builds the `ScanResult`, add a post-scan pass that detects tool name collisions across all loaded skills (bundled + community). Emit a `WARN` for each collision. This catches community skills that collide with bundled skills at install/reload time, not just at tool-dispatch time.

Implementation: Add a `detect_tool_name_collisions(entries: &[SkillEntry])` function to `skills/index.rs` that builds the same multi-map and logs collisions. Call it at the end of `scan_skills_dir` before returning `ScanResult`. Lightweight — O(total_tools) with a HashMap.

### Implementation Unit 3: Registry-Drift Detection (AC3)

**File:** `crates/mika-agent/src/skills/index.rs` (new method on `SkillRegistry`) + `crates/mika-agent/src/agent.rs` (call site)

**Approach: mtime-based staleness check.** On each registry load (both initial and hot-reload), record the `mtime` of each skill's `tools.json` file. On subsequent turns, before building `skill_tool_map`, compare current `mtime` values against recorded ones. If any `tools.json` has a newer mtime, set `skills_dirty` and force a reload before the turn proceeds.

**Data model:**

```rust
// In SkillEntry (index.rs)
pub struct SkillEntry {
    // ... existing fields ...
    /// mtime of tools.json at load time (None if no tools.json)
    pub tools_json_mtime: Option<std::time::SystemTime>,
}
```

Record `tools_json_mtime` in `scan_skills_dir` when loading each skill's `tools.json`.

**Check site:** Add `SkillRegistry::check_tools_json_freshness(&self) -> bool` method that `stat()`s each loaded skill's `tools.json` and compares against stored mtime. Returns `true` if any file changed. Call this:

1. **Per-turn in server mode** (`handlers.rs:734`): Before using the registry, call `check_tools_json_freshness()`. If stale, force a reload (same path as `skills_dirty`). This is cheap — one `stat()` per skill per turn, no file reads.

2. **On the hot-reload path** (`handlers.rs:734`, `a2a.rs:117`): After reload, verify the new registry's mtimes are fresh. If still stale (file changed during reload), log an error and set `skills_dirty` again.

**Why mtime, not content hash:** The incident shows the in-memory state diverged from disk, but disk was always correct. mtime comparison is O(1) per file (a single `stat()` syscall), doesn't require reading file content, and catches any modification regardless of cause. Content hashing would require reading every `tools.json` on every turn — too expensive.

**Why not just rely on `skills_dirty`:** The `skills_dirty` flag is only set by Mika's own tool calls (create/delete/update/toggle skill, variant write). If a file changes outside these paths (e.g., `make deploy` copies new skill files, or a race during `seed_bundled_skills`), the flag is never set. The mtime check catches these external mutations.

### Implementation Unit 4: Forced Registry Reload Without Restart (AC5)

**File:** `crates/mika-agent/src/server/handlers.rs` (new endpoint) + `crates/mika-cli/src/commands/skills.rs` (new subcommand)

**4a — HTTP endpoint:**

Add `POST /api/v1/skills/reload` to the mika-server router, requiring `MIKA_INTERNAL_TOKEN` auth (mutation endpoint). Handler:

```rust
async fn handle_skills_reload(
    State(app): State<AppState>,
) -> impl IntoResponse {
    for (agent_id, agent_state) in &app.agents {
        agent_state.skills_dirty.store(true, Ordering::Release);
        info!(agent_id = %agent_id, "skills reload requested via API");
    }
    Json(json!({"status": "reload_queued", "agents": app.agents.len()}))
}
```

This sets `skills_dirty` on all agents, so the next inbound message for each agent triggers a full reload from disk. It doesn't block — the reload happens lazily on the next turn.

**4b — CLI command:**

Add `mika skills reload` subcommand. Two modes:

- **Server mode** (default when `MIKA_SERVER_URL` is set): `POST /api/v1/skills/reload` to the running mika-server
- **Direct mode** (fallback): Not applicable — CLI doesn't hold a persistent registry. Print guidance to use the server endpoint.

**4c — SIGUSR1 handler (stretch):**

Register a `SIGUSR1` handler in `server/mod.rs` that sets `skills_dirty` on all agents. This allows `kill -USR1 $(pidof mika-server)` for operators who prefer signals over HTTP. Use `tokio::signal::unix::signal(SignalKind::user_defined1())`.

### Implementation Unit 5: Solution Documentation

**File:** `docs/solutions/logic-errors/skill-registry-drift-diagnosis-2026-05-29.md`

Document:
- The failure mode (in-memory drift from on-disk state)
- How to diagnose (check for `skill tool name collision` WARN in logs, then verify on-disk `tools.json` content matches expectations)
- The fix (collision detection + mtime-based staleness + forced reload endpoint)
- Prevention (build-time invariant test, install-time collision check)
- Cross-reference to `builtin-skill-tool-name-shadowing.md` (related but distinct — that doc covers builtin-tool-vs-skill-tool dispatch priority; this covers skill-tool-vs-skill-tool HashMap construction)

## Implementation Order

1. **IU1** (collision logging) — smallest diff, highest immediate value. Makes future occurrences visible in logs.
2. **IU2** (build-time invariant) — prevents the class of bug from being introduced by new skills.
3. **IU2b** (install-time collision detection) — extends protection to community skills.
4. **IU3** (mtime-based drift detection) — catches drift from any source, not just name collisions.
5. **IU4a** (HTTP reload endpoint) — recovery mechanism for operators.
6. **IU4b** (CLI reload command) — ergonomic wrapper for the endpoint.
7. **IU5** (solution doc) — compound the learning.
8. **IU4c** (SIGUSR1 handler) — stretch, low priority.

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| mtime comparison across NFS/container filesystems may have lower resolution | Missed reload on rapid writes | Accept: the primary defense is collision logging (IU1); mtime is defense-in-depth |
| `stat()` per skill per turn adds latency | Slight per-turn overhead | Bounded: ~30 skills × 1 stat each = ~30 syscalls, ~0.5ms total. Negligible vs LLM call latency |
| New HTTP endpoint expands attack surface | Unauthorized reload | Mitigated: requires `MIKA_INTERNAL_TOKEN` (same as other mutation endpoints) |
| SIGUSR1 could conflict with other signal handlers | Signal handling race | Mitigated: SIGUSR1 is conventionally user-defined; no existing handler in mika |

## Testing Strategy

- **IU1:** Unit test for collision WARN emission; update existing `test_build_skill_tool_map_last_skill_wins_on_collision`
- **IU2:** New test in `bundled_skills.rs` — runs as part of `cargo test`
- **IU2b:** Inline test in `index.rs` with two fixture skills sharing a tool name
- **IU3:** Unit test creating a `SkillRegistry`, modifying a `tools.json` file on disk, and asserting `check_tools_json_freshness()` returns `true`
- **IU4a:** Integration test (or manual verification) of `POST /api/v1/skills/reload`
- **IU4b:** Manual verification of `mika skills reload`

## Related Issues

- mika#1220 — qa-review loaded as always_on; same skill-loading family
- mika#1224 — qa-review loaded on mika-dev despite identity allowlist; skill-loading layer
- mika#1196 — `validate_qa_review_gh_scope` machinery; active_skill_paths reconstruction on reload
- mika#1325 — static parity assertion for engine-referenced tool names (committed 2026-05-28)
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` — related (dispatcher precedence) but distinct (this is HashMap construction)
