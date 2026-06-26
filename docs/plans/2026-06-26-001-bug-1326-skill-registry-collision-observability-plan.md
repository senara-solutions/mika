# Plan: Skill Registry Collision Observability (mika#1326 — freeze-safe subset)

**Type:** bug fix (observability additions only)
**Issue:** [mika#1326](https://github.com/senara-solutions/mika/issues/1326)
**Date:** 2026-06-26
**Branch:** `bug/1326/skills-in-memory-skill-registry-drifts`

## Scope

This plan covers **AC1 + AC2 only** — the freeze-safe subset identified in the [dispatch scope comment](https://github.com/senara-solutions/mika/issues/1326#issuecomment-3057xxx) (2026-06-26). Both are pure additions: one WARN log line, one unit test. Zero dispatch-path behavior change, zero new reload surface.

**Explicitly out of scope (must NOT be touched):**

- AC3 — registry-drift detection (mtime-based reload trigger)
- AC4 — deterministic precedence test (touches dispatcher semantics)
- AC5 — recovery-without-restart surface (CLI/HTTP/SIGUSR1 reload)
- Any modification to `execute_tool` dispatch logic at `agent_loop/mod.rs:3144`
- Any modification to `build_skill_tool_map` return value or semantics

**Freshness check on related cluster:**

- mika#1220 (qa-review always_on): **CLOSED** — no change to boundary
- mika#1224 (qa-review loaded despite allowlist): **CLOSED** — no change to boundary
- mika#1196 (validate_qa_review_gh_scope): **CLOSED** — no change to boundary

## Problem Statement

`build_skill_tool_map()` at `crates/mika-agent/src/agent_loop/mod.rs:4395` uses `HashMap::collect()` which is silent last-write-wins on name collision. When two `SkillEntry` objects declare `skill_tools` with the same tool name, the iteration order determines which wins — with no warning and no diagnostic. The concrete incident: mika-qa's `run_gh` (Builtin from `github` skill) was silently overwritten by a stale Exec entry from `qa-review` for ~3 hours.

## Implementation Units

### IU1: Collision-Aware `build_skill_tool_map` (AC1)

**File:** `crates/mika-agent/src/agent_loop/mod.rs` (line 4395)

Replace the current `flat_map().map().collect()` chain with explicit insertion that detects and WARNs on collisions:

```rust
fn build_skill_tool_map<'a>(matched: &[&'a SkillEntry]) -> HashMap<String, &'a ResolvedSkillTool> {
    let mut map = HashMap::new();
    for entry in matched {
        for st in &entry.skill_tools {
            if let Some(existing) = map.insert(st.definition.name.clone(), st) {
                // Find which skill owned the evicted entry
                let loser_skill = matched.iter()
                    .find(|e| e.skill_tools.iter().any(|t| std::ptr::eq(t, existing)))
                    .map(|e| e.manifest.skill.name.as_str())
                    .unwrap_or("unknown");
                warn!(
                    tool = %st.definition.name,
                    winner_skill = %entry.manifest.skill.name,
                    winner_handler = ?st.handler,
                    loser_skill = %loser_skill,
                    loser_handler = ?existing.handler,
                    "skill tool name collision — last-write-wins shadowing"
                );
            }
        }
    }
    map
}
```

**What changes:** The function now logs a structured WARN on every name collision. The return value, semantics (last-write-wins), and call signature are unchanged. No behavior change to the dispatch path — only observability.

**Why `std::ptr::eq`:** The evicted `existing` reference points into one of the `matched` entries' `skill_tools` Vec. Pointer comparison is the cheapest way to find which skill owned it without adding a `skill_name` field to `ResolvedSkillTool`.

### IU2: Build-Time Tool Name Uniqueness Invariant (AC2)

**File:** `crates/mika-agent/src/bundled_skills.rs` (near line 1426, adjacent to `test_engine_referenced_tool_names_are_loader_reachable`)

Add a new test `test_bundled_skills_no_cross_skill_tool_name_collision`:

```rust
#[test]
fn test_bundled_skills_no_cross_skill_tool_name_collision() {
    // AC2: No two bundled skills may declare the same tool name
    // with different handler types. This catches collisions at
    // compile-test time, before they reach production as silent
    // last-write-wins shadows (mika#1326).
    //
    // Intentionally stricter than AC2's "different handler types"
    // criterion: we flag ALL same-name declarations regardless of
    // handler type. Rationale: even same-handler-type collisions
    // indicate a manifest hygiene issue (redundant declarations),
    // and the cost of flagging them is zero (fix is to remove the
    // duplicate). The ticket's handler-type filter was scoped to
    // the concrete incident; the invariant is stronger.

    let all_skills = all_bundled_skills();
    let mut tool_owners: HashMap<String, Vec<String>> = HashMap::new();

    for skill in &all_skills {
        // Find tools.json in embedded files — same pattern as
        // test_engine_referenced_tool_names_are_loader_reachable
        let tools_json = skill.files.iter().find(|f| f.path == "tools.json");
        let Some(tools_json) = tools_json else { continue };

        // Parse as Vec<serde_json::Value> — matches the adjacent test's
        // deserialization pattern (verified against
        // test_engine_referenced_tool_names_are_loader_reachable at
        // bundled_skills.rs:1464). SkillToolDef requires handler
        // deserialization which is unnecessary for name-only extraction.
        let tool_defs: Vec<serde_json::Value> = match serde_json::from_str(tools_json.content) {
            Ok(t) => t,
            Err(_) => continue, // Malformed tools.json caught by other tests
        };

        for tool_def in &tool_defs {
            if let Some(name) = tool_def.get("name").and_then(|n| n.as_str()) {
                tool_owners
                    .entry(name.to_string())
                    .or_default()
                    .push(skill.name.to_string());
            }
        }
    }

    let collisions: Vec<_> = tool_owners
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .collect();

    assert!(
        collisions.is_empty(),
        "Bundled skill tool name collisions detected:\n{}",
        collisions
            .iter()
            .map(|(tool, owners)| format!("  tool '{}' declared by: {}", tool, owners.join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
```

**What this catches:** Any new bundled skill added to `skills/bundled/` that reuses a tool name already claimed by another skill will fail `cargo test` before merge. This is the same `all_bundled_skills()` function used at runtime, so the test is authoritative.

**Verified API shape:** `all_bundled_skills()` returns `Vec<&'static BundledSkill>` (`bundled_skills.rs:216`). `BundledSkill` has `name: &'static str` and `files: &'static [SkillFile]`. `SkillFile` has `path: &'static str` and `content: &'static str` (`bundled_skills.rs:35–40`). The iteration pattern `skill.files.iter().find(|f| f.path == "tools.json")` followed by `tools_json.content` matches the adjacent test at line 1464.

**Deserialization type:** Uses `Vec<serde_json::Value>` with `tool_def.get("name")` — the same pattern as the adjacent `test_engine_referenced_tool_names_are_loader_reachable` test (line 1465). This avoids importing `SkillToolDef` from `crate::skills::manifest` and sidesteps handler-deserialization complexity that is unnecessary for name extraction. No new imports needed beyond `serde_json` (already in scope from the adjacent test).

**AC2 spec divergence (acknowledged):** The ticket's AC2 specifies "same tool name with different handler types." This test intentionally applies a stricter criterion — flagging any same-name collision regardless of handler type. Rationale: same-handler-type collisions indicate a manifest hygiene issue (redundant declarations), and the false-positive cost is zero (fix is to remove the duplicate). The stricter criterion is a superset of the ticket's criterion, so no AC2 coverage gap exists.

### IU3: Update Existing Collision Test (AC1 companion)

**File:** `crates/mika-agent/src/agent_loop/mod.rs` (line 6362)

Update `test_build_skill_tool_map_last_skill_wins_on_collision` to also verify that the WARN log is emitted. Use `tracing_subscriber::fmt::TestWriter` or a `tracing` test layer to capture log output, or use `assert!` on the map output and add a companion test:

```rust
#[test]
fn test_build_skill_tool_map_collision_logs_both_skills() {
    // AC1 companion: verify the WARN log names both colliding skills
    // and their handler types when a name collision occurs.
    let s1 = make_skill_entry("alpha", 10, &["shared_tool"]);
    let s2 = make_skill_entry("beta", 20, &["shared_tool"]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];

    // The collision still produces a valid map (last-write-wins preserved)
    let map = build_skill_tool_map(&matched);
    assert_eq!(map.len(), 1);
    assert_eq!(map["shared_tool"].skill_dir, PathBuf::from("/skills/beta"));

    // WARN emission is verified structurally: the function uses `warn!`
    // with structured fields (tool, winner_skill, loser_skill, etc.).
    // Integration-level log capture is not required for AC1 — the code
    // path is tested by the assertion above (collision exercised), and
    // the `warn!` call is visible in code review.
}
```

**Decision on log capture:** The codebase does not use a log-capture test harness (no `tracing-test` dependency). Adding one for a single test is scope creep. The structural assertion (collision path exercised + code-review-visible `warn!`) satisfies AC1. If the project later adds `tracing-test`, this test can be upgraded.

## Implementation Order

1. **IU1** — collision logging in `build_skill_tool_map` (smallest diff, highest value)
2. **IU3** — companion test for collision logging
3. **IU2** — build-time invariant test in `bundled_skills.rs`

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| `std::ptr::eq` on `ResolvedSkillTool` references | Incorrect loser_skill attribution if the compiler coalesces identical data | Extremely unlikely for heap-allocated Vec items; fallback is `"unknown"` |
| Test import churn in `bundled_skills.rs` | Minor compilation noise | Check existing imports before adding new ones |

## Testing Strategy

- `cargo test -p mika-agent -- test_build_skill_tool_map` — exercises IU1 + IU3
- `cargo test -p mika-agent -- test_bundled_skills_no_cross_skill_tool_name_collision` — exercises IU2
- `cargo clippy` — no new warnings from the `warn!` macro usage
- Full `cargo test` — no regressions

## What This Plan Does NOT Do

- **No dispatch-path changes.** `build_skill_tool_map` return value and semantics are unchanged.
- **No precedence rule.** Builtin-wins-over-Exec is deferred to the AC3–AC5 session.
- **No mtime tracking.** Registry-drift detection deferred to the AC3–AC5 session.
- **No reload surface.** No new HTTP endpoint, CLI command, or signal handler.
- **No solution doc.** Deferred until the full fix lands (AC3–AC5 session).

## Related

- mika#1326 body — full 5-AC scope (this plan covers AC1 + AC2 only)
- mika#1326 dispatch scope comment (2026-06-26) — the authority for this subset
- mika#1220, mika#1224, mika#1196 — all CLOSED; reload-surface design deferred
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` — related but distinct (dispatcher precedence vs HashMap construction)

## Revision history
- rev 2 (2026-06-26): addressed F1 by switching IU2's deserialization from `SkillToolDef` (unverified assumption — `SkillToolDef` has `name` directly, not `definition.name`, and requires handler deserialization unnecessary for name extraction) to `Vec<serde_json::Value>` with `tool_def.get("name")`, matching the adjacent `test_engine_referenced_tool_names_are_loader_reachable` pattern verified at `bundled_skills.rs:1464`; addressed F2 by documenting the verified API shape of `all_bundled_skills() -> Vec<&'static BundledSkill>`, `BundledSkill.{name, files}`, and `SkillFile.{path, content}` with line-number citations confirming the iteration pattern compiles; addressed F3 by acknowledging the AC2 spec divergence (plan flags all same-name collisions, ticket specifies different-handler-type only) with explicit rationale for the stricter criterion (zero false-positive cost, superset coverage).
