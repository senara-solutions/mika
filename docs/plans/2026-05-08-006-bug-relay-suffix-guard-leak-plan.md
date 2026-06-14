---
ticket: mika#1041
type: bug
title: Required-suffix-line guard (#864) leaks into mika-relay permission turns
created: 2026-05-08
branch: bug/1041/agent-required-suffix-line-guard-864
---

## Problem

PR #864's `Required-suffix-line guard` fires on **mika-relay's permission-decision turns** despite mika-relay's spec at `crates/mika-agent/src/well_known_agents.rs:161-194` denylisting `dev-groom`, `mika-arch-groom-ticket`, `mika-arch-second-review`, and `mika-arch-groom-milestone`. Result: every dev-groom dispatch on v0.10.0 hits a re-prompt loop on relay turns, claude-pilot's relay parser sees prose instead of permission JSON, every tool call auto-denies, the subprocess produces zero commits.

## Pinned API shapes (Phase 0 — pre-implementation verification)

Verbatim from `crates/mika-agent/src/db.rs`:

**`SkillOverride` struct (db.rs:441-451):**

```rust
/// A user override for a skill property (persists across bundled skill re-sync).
#[derive(Debug, Clone, Default)]
pub struct SkillOverride {
    pub skill_name: String,
    pub always_on: Option<bool>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    /// Tri-state: `None` = default (enabled), `Some(false)` = disabled,
    /// `Some(true)` = explicitly enabled.
    pub enabled: Option<bool>,
}
```

**`get_skill_overrides` signature (db.rs:3915):**

```rust
pub fn get_skill_overrides(&self, agent_id: &str) -> Result<Vec<SkillOverride>>
```

**`set_skill_enabled` signature (db.rs:3975-3980):**

```rust
pub fn set_skill_enabled(
    &mut self,
    agent_id: &str,
    skill_name: &str,
    enabled: bool,
) -> Result<()>
```

The reconciliation block uses these shapes verbatim. Comparison against `Some(false)` is well-typed: `enabled` is `Option<bool>`. `set_skill_enabled` takes a plain `bool` (it converts internally via `if enabled { None } else { Some(false) }` so `false` writes the disabled marker). The existing `llm_overrides` reconciliation block at `well_known_agents.rs:514-555` uses these same shapes — the `disabled_skills` reconciliation we add inherits identical typing.

**Pinned dev-groom manifest content** (`skills/bundled/dev-groom/skill.toml`):

```toml
[skill]
name = "dev-groom"
always_on = false
timeout_secs = 600

[triggers]
keywords = ["groom", "groom ticket", "/mika-groom-ticket", "groom issue"]

[output]
required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
```

Confirms the load-bearing element of the causal chain: dev-groom **does** declare `[output] required_suffix_lines`. When `match_skills` keyword-matches `"groom"` in mika-relay's payload, `collect_required_suffix_lines` (`agent.rs:4041-4048`) collects this list into the guard's accept-set. The plan's narrative is accurate.

For symmetry, the other two skills in `MIKA_RELAY.disabled_skills` that declare suffix lines also contribute when they match: `mika-arch-groom-ticket` declares `["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]` and `mika-arch-second-review` declares `["Verdict: GROOMED", "Verdict: ESCALATE"]`. Both have keyword sets less likely to substring-match permission payloads (e.g., `"groom-ticket"`, `"architect-review"`, `"second-review"`), but the seeding-drift fix removes the leak risk for all three uniformly.

## Root cause

The leak is **not** in the guard itself (`crates/mika-agent/src/agent.rs:1472-1530`) and **not** in `collect_required_suffix_lines` (`crates/mika-agent/src/agent.rs:4035-4048`) — both are correctly skill-scoped to `Keyword`/`AlwaysOn` matched skills.

The leak is in **idempotent seeding drift** at `crates/mika-agent/src/well_known_agents.rs:503-567`:

```rust
pub fn seed_well_known_skill_overrides(db: &mut Database, agent_name: &str) {
    let spec = match find_well_known_agent(agent_name) { ... };

    match db.get_skill_overrides(agent_name) {
        Ok(overrides) if !overrides.is_empty() => {
            // Reconcile LLM overrides only — disabled_skills drift is NOT reconciled.
            for llm_ov in spec.llm_overrides { ... }
            return;  // ← early-return on existing overrides
        }
        ...
    }
    // Disabled-skills seeding only runs on first creation.
    for skill_name in spec.disabled_skills {
        db.set_skill_enabled(agent_name, skill_name, false);
    }
    ...
}
```

Sequence that produced the failing state:

1. mika-relay was provisioned at v < 0.10.0 (when `dev-groom` did not exist yet).
2. The first provisioning seeded `enabled=0` rows for the then-current denylist (self-dev, qa-review, ...), satisfying the "overrides exist" branch.
3. PRs #845/#893 added `dev-groom` to `MIKA_RELAY.disabled_skills` (line 181).
4. On every subsequent startup, `seed_well_known_skill_overrides` early-returns at line 556 because the table is non-empty. The `for skill_name in spec.disabled_skills` loop at line 569 never runs.
5. `dev-groom` ends up with `enabled = NULL` (= default-enabled) for mika-relay.

When the autonomous loop dispatches a grooming task:

1. mika-dev calls `run_claude_pilot` with the dev-groom skill, which spawns a `claude-pilot` subprocess executing `/mika-groom-ticket`.
2. Each tool-use inside that subprocess is relayed to mika-relay as a `[claude-pilot] {permission_event_json}` payload.
3. The payload contains the substring `groom` — directly in the parent task label (`Groom mika#854`), the `decision_reason_so_far` text, and frequently in tool inputs (e.g. `gh issue view 854` for a ticket whose body contains "groom").
4. `match_skills` (`crates/mika-agent/src/skills/matcher.rs:38`) does case-insensitive substring matching. `dev-groom`'s keyword `"groom"` matches.
5. `dev-groom` is enabled (per the drift bug), so it is in mika-relay's loaded `SkillRegistry`. It enters `matched` with `MatchReason::Keyword`.
6. `collect_required_suffix_lines` collects `["Verdict: GROOMED", "Verdict: ESCALATE"]` from dev-groom's `[output] required_suffix_lines`.
7. Post-condition #8 in `run_loop` rejects mika-relay's permission JSON because it doesn't end with `Verdict: GROOMED|ESCALATE`. The model objects in prose. claude-pilot's relay parser sees `[fallback] Invalid JSON from command`. Tool call auto-denies. Loop.

## Fix

Extend the existing `llm_overrides` reconciliation block at `crates/mika-agent/src/well_known_agents.rs:514-555` with a parallel `disabled_skills` reconciliation pass. Mirror the same shape: detect drift between `spec.disabled_skills` and the agent's `skill_overrides` rows, write only the deltas, log per-row reconciliation, fail-soft on individual write errors.

The reconciliation must handle two drift directions:

| Drift | Detection | Action |
|-------|-----------|--------|
| Skill in `spec.disabled_skills` but `enabled` is NULL or `1` for that agent | `existing.enabled != Some(false)` for skill name in spec | `db.set_skill_enabled(agent, skill, false)` |
| Skill not in `spec.disabled_skills` but has `enabled = 0` row | Out of scope for this fix (see "Out of scope" below) |

Direction 1 closes the dev-groom leak. Direction 2 (renabling skills removed from the denylist) is deferred — it changes operator-perceived state (an explicit-disable could turn back on across a deploy) and is not required by the acceptance criteria. We leave that as a separate decision.

## Implementation

**File:** `crates/mika-agent/src/well_known_agents.rs`

Insert a `disabled_skills` reconciliation block immediately before the `for llm_ov in spec.llm_overrides` loop at line 516. The block runs in the existing `Ok(overrides) if !overrides.is_empty()` arm (line 511) so that the early-return at line 556 still terminates the function cleanly after both reconciliations have run.

```rust
// Reconcile disabled_skills drift: when a new skill is added to the
// well-known denylist after the agent was first provisioned, the original
// seeding-once path skipped it. We compare spec.disabled_skills against
// the existing rows and write the delta. See mika#1041 for the dev-groom
// leak that motivated this.
//
// Reverse direction (spec removes a skill from denylist while the DB still
// has enabled=false) is intentionally NOT reconciled here. Operator manual
// disables (via `mika skills disable <name>`) take precedence over spec
// changes — re-enabling on deploy could turn a manually-disabled skill back
// on. Operators can re-enable with `mika skills enable <name>`. This is the
// correct asymmetry: positive-direction drift is unintended drift (skill
// added to spec, not yet seeded); negative-direction drift may be intentional
// operator state. See mika#1041 § "Out of scope" for the full reasoning.
let mut disabled_reconciled = 0u32;
for skill_name in spec.disabled_skills {
    let needs_disable = !overrides.iter().any(|existing| {
        existing.skill_name == *skill_name && existing.enabled == Some(false)
    });
    if needs_disable {
        if let Err(e) = db.set_skill_enabled(agent_name, skill_name, false) {
            warn!(
                agent = agent_name,
                skill = skill_name,
                error = %e,
                "failed to reconcile disabled_skills drift for well-known agent"
            );
        } else {
            info!(
                agent = agent_name,
                skill = skill_name,
                "reconciled drifted disabled_skills entry for well-known agent"
            );
            disabled_reconciled += 1;
        }
    }
}
if disabled_reconciled > 0 {
    info!(
        agent = agent_name,
        reconciled_count = disabled_reconciled,
        "reconciled drifted disabled_skills for well-known agent"
    );
}

// (existing llm_overrides reconciliation continues unchanged)
```

The placement inside the existing `Ok(overrides) if !overrides.is_empty()` arm gives us:

- the same fail-soft semantics already used for `llm_overrides` drift,
- a clean rebuild via `cargo build` with no API changes,
- structural symmetry with the llm-override reconciliation (a future reader can see both reconciliations in one block).

No changes to `agent.rs` (the guard) or `matcher.rs` (the substring match). Both are correct given properly-seeded denylist state.

## Tests

Add a unit test alongside the existing `test_seed_skill_overrides_*` tests in the same file (`crates/mika-agent/src/well_known_agents.rs` `#[cfg(test)] mod tests`):

```rust
#[test]
fn test_seed_reconciles_disabled_skills_drift() {
    // Simulate a pre-#845 mika-relay: existing rows for the *original*
    // denylist (e.g. self-dev) but not for dev-groom which was added later.
    let temp_dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(&temp_dir.path().join("test.db")).unwrap();
    db.register_agent("mika-relay", "Relay", "/tmp/mika-relay").unwrap();

    // Seed only the original denylist, simulating an outdated provisioning.
    db.set_skill_enabled("mika-relay", "self-dev", false).unwrap();
    db.set_skill_enabled("mika-relay", "qa-review", false).unwrap();

    // Pre-condition: dev-groom is NOT in the table.
    let pre = db.get_skill_overrides("mika-relay").unwrap();
    assert!(!pre.iter().any(|o| o.skill_name == "dev-groom"));

    // Run reconciliation.
    seed_well_known_skill_overrides(&mut db, "mika-relay");

    // Post-condition: every entry in MIKA_RELAY.disabled_skills now has
    // enabled = Some(false).
    let post = db.get_skill_overrides("mika-relay").unwrap();
    for skill_name in MIKA_RELAY.disabled_skills {
        let row = post.iter().find(|o| o.skill_name == *skill_name);
        assert!(
            row.is_some(),
            "expected disabled-row for {skill_name}, found none"
        );
        assert_eq!(row.unwrap().enabled, Some(false));
    }
}
```

The existing first-creation tests (`test_seed_skill_overrides_mika_relay`, etc.) cover the empty-table path; this new test covers the drift path.

Run with: `cargo test -p mika-agent well_known_agents -- --nocapture`.

### Negative-direction eval regression (`tests/eval/grounding_regressions/required_suffix_line_relay_no_fire.rs`)

The unit test above verifies the reconciliation logic in isolation (DB state shape). It does not verify the end-to-end behavior: that a mika-relay permission turn does not trigger the guard post-fix. Without this paired negative test, a future regression that re-enables a verdict-suffix-declaring skill on mika-relay could ship undetected — the same $2.80 burn shape as today.

Add a new scenario to `tests/eval/grounding_regressions/` paralleling the existing `required_suffix_line_*` positive scenarios:

```rust
// tests/eval/grounding_regressions/required_suffix_line_relay_no_fire.rs
//
// Negative-direction regression for mika#1041. Asserts that a mika-relay
// permission turn does NOT fire the Required-suffix-line guard, even when
// the user message contains the substring "groom" (which would keyword-match
// dev-groom if dev-groom were enabled on relay).
//
// Counterpart to required_suffix_line_caught.rs / required_suffix_line_unconstrained.rs.

use crate::eval::EvalHarness;
use crate::eval::grounding_assertions::*;

#[tokio::test]
async fn required_suffix_line_relay_no_fire() {
    let harness = EvalHarness::builder()
        .agent("mika-relay")  // exercises the well-known-agent denylist
        .seed_well_known_overrides(true)  // applies disabled_skills + reconciliation
        .build();

    let user_message = r#"[claude-pilot] {"event_type":"permission_request","tool_name":"Bash","tool_input":{"command":"gh issue view 854","cwd":"/tmp"},"session_id":"abc","decision_reason_so_far":"Groom mika#854 task started"}"#;

    let response = harness.run_silent_turn(user_message).await;

    // Assertion: assistant's response is permission JSON, not a verdict-shaped suffix.
    assert_response_forbids(&response, "Verdict: GROOMED");
    assert_response_forbids(&response, "Verdict: ESCALATE");
    assert_response_forbids(&response, "Disposition:");

    // Assertion: no "Required-suffix-line guard" warn was emitted in the harness's log buffer.
    assert_no_log_event(&response, "Required-suffix-line guard");
}
```

Register the scenario in `tests/eval/grounding_regressions/mod.rs` alongside the existing `required_suffix_line_*` modules. Tag with `grounding:relay-isolation` (new sub-tag in the `grounding:*` namespace).

The test depends on `EvalHarness` exposing a `seed_well_known_overrides()` builder method. If that method does not exist today, the test scaffolding is added as part of this PR (a small additive method on the builder; mirrors the existing `embedding_client()`, `brave_api_key()`, etc.). If the implementer finds that adding the harness method substantially expands the diff, the test can be filed as a follow-up sibling ticket — the unit test alone suffices to gate the merge, but the eval test is required for full positive/negative symmetry coverage.

## Acceptance criteria

(From issue body, mapped to verification commands.)

1. **mika-relay's permission-decision turns do NOT trigger the verdict-suffix guard.**
   - Verification: after deploy, dispatch any grooming task and grep `~/.mika/agents/mika-relay/logs/mika.log.<date>` for `Required-suffix-line guard` — must return zero hits.
   - Structural verification: run the new `test_seed_reconciles_disabled_skills_drift` test; it must pass.

2. **Re-running today's failed dispatch succeeds end-to-end with at least one commit on the worktree branch.**
   - Verification: `mika ask --agent mika-dev "groom mika issue#854"` produces a populated worktree with at least one `git log` entry on the bug-branch.
   - Smoking-gun signal: claude-pilot log no longer shows `[fallback] Invalid JSON from command: I see you're holding an error about a missing suffix line` warnings.

3. **Existing dev-groom verdict enforcement remains intact.**
   - Verification: dispatch `mika-arch-groom-ticket` on an unrelated ticket via the architect agent. The architect's response must still terminate with a `Disposition: <READY|ITERATE|ESCALATE>` line, and the existing test `tests/eval/grounding_regressions/required_suffix_line_caught.rs` must still pass.
   - Structural verification: no changes to `agent.rs:1472-1530` or `collect_required_suffix_lines` mean the guard's behavior on properly-matched skills is unchanged by construction.

## Rollout

- Single-PR change to `well_known_agents.rs` (one new reconciliation block + one new unit test).
- Deploy via the standard release flow. On startup, every CLI invocation goes through `init_base_for_agent` (`crates/mika-cli/src/init.rs:70`), which calls `seed_well_known_skill_overrides` under `dev_mode`. The reconciliation runs once per CLI invocation; it's idempotent (no work when the table is already in sync).
- No DB migration. The fix uses existing `skill_overrides` table semantics (schema v24, `enabled` column).
- Backwards compatible. Agents whose denylists are already in sync (e.g. freshly-provisioned ones) hit the `needs_disable = false` branch and write nothing.

## Out of scope

- **Reverse-direction reconciliation** (skill removed from denylist → re-enable it). Changing this would alter operator state (re-enabling a skill on deploy could surprise an operator who manually disabled it). Tracked separately if needed.
- **Tighter dev-groom keyword match.** `"groom"` is a known substring-false-positive class (memory: `project_keyword_substring_false_positives.md`). This ticket's "Out of scope" section explicitly says do not change the keywords; the guard is what's wrong, not the matcher. We respect that — the seeding fix removes the leak without touching the matcher.
- **Engine-level guard scoping by agent.** Adding agent-name-aware logic to `collect_required_suffix_lines` would couple the guard to specific agents and bypass the `skill_overrides` table that's the correct policy surface.
- **Dispatch error-recovery.** The cosmetic followup mentioned in the ticket evidence (parent-task-ID path is wrong in mika-dev's pipeline-failure close-out message) is not addressed here.

## Risks and mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| Reconciliation writes block startup if DB is locked | Low — the same path already runs for `llm_overrides` reconciliation. | Per-row error is logged and continues. Total reconciliation finishes in a few writes, no transaction. |
| Future denylist drift in the *opposite* direction (a skill removed from the denylist stays disabled) | Medium — discussed above as out of scope. | Operator can manually re-enable via `mika skills enable <name>`. |
| New test depends on `Database::open` + temp_dir — flaky CI? | Very low — same pattern as the existing `test_seed_skill_overrides_*` tests. | Reuse the same test scaffolding. |
| Race with `migrate_disabled_markers` (legacy `.disabled` files → DB) | Negligible — `migrate_disabled_markers` only writes `enabled = false`, same as the reconciler; both are idempotent. | None needed. |

## Verification commands (smoke)

After landing:

```bash
# Build + unit test the fix
cargo build -p mika-agent
cargo test -p mika-agent well_known_agents

# Confirm post-deploy state on a host with mika-spirit running
sqlite3 ~/.mika/data/mika.db \
  "SELECT skill_name, enabled FROM skill_overrides
   WHERE agent_id = 'mika-relay' AND skill_name IN
   ('dev-groom', 'mika-arch-groom-ticket', 'mika-arch-second-review',
    'mika-arch-groom-milestone');"
# Expect: 4 rows, all with enabled=0.

# Re-run the failed dispatch
mika ask --agent mika-dev "groom mika issue#854"
# Expect: dispatch succeeds, worktree has commits.

# Confirm no guard re-prompts
grep "Required-suffix-line guard" ~/.mika/agents/mika-relay/logs/mika.log.* | tail -10
# Expect: zero hits since deploy timestamp.
```

## Notes for the implementer

- The fix is one new block of ~30 lines plus one unit test. Avoid scope creep.
- Do not introduce a `reconcile_disabled_skills` helper — the symmetry with the inlined `llm_overrides` reconciliation block is the readability win. Future refactor (extracting both) belongs in a separate PR.
- Keep the warn/info log shape parallel to the existing `llm_overrides` reconciliation logs so operators searching `seeded_well_known` / `reconciled` patterns see consistent semantics.
