# Plan: feat(agent-core,skills): curator background task — archive + rollback, never auto-promote

**Issue:** senara-solutions/mika#1584
**Type:** feature
**Blocked by:** senara-solutions/mika#1583 (which is blocked by senara-solutions/mika#1582)
**Parent:** senara-solutions/mika#1581 (self-improving-skills milestone)

## Dependency assumption

This plan assumes #1582 has landed, which provides:
- `lifecycle_state TEXT` column on `skill_overrides` with values `'staged'`, `'active'`, `'archived'`
- `skill_manage` builtin tool for agent-authored skill lifecycle
- CLI `mika skills archive <name> --agent <agent>` command (basic, without snapshot)

If #1582's column shape differs from what's assumed here, adjust the migration and queries accordingly.

## Overview

Add usage tracking (`use_count`, `last_used_at`) to `skill_overrides`, a periodic `CuratorReview` background task that proposes archival of idle agent-authored skills, snapshot-based rollback for archived skills, and a `restore` command to recover archived skills.

## Implementation steps

### Step 1: Schema migration v43 — add usage tracking columns

**File:** `crates/mika-agent/src/db.rs`

1. Bump `CURRENT_SCHEMA_VERSION` from 42 to 43.
2. Add migration v42→v43:
   ```sql
   ALTER TABLE skill_overrides ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;
   ALTER TABLE skill_overrides ADD COLUMN last_used_at TEXT;
   ```
   Use the additive `ALTER TABLE` pattern with `column_exists` guard (same as v30→v31, v37→v38) for crash-recovery safety.
3. Update `SkillOverride` struct (line ~337) to add:
   ```rust
   pub use_count: i64,
   pub last_used_at: Option<String>,
   ```
4. Update `get_skill_overrides()` and all query sites that read from `skill_overrides` to include the new columns.
5. Add `increment_skill_usage(agent_id: &str, skill_names: &[String])` DB method:
   ```sql
   UPDATE skill_overrides
   SET use_count = use_count + 1, last_used_at = ?
   WHERE agent_id = ? AND skill_name = ?
   ```
   This is a batch update in a single transaction. If no row exists for a skill (bundled/marketplace skills without overrides), use `INSERT OR IGNORE` with default fields then UPDATE, or an UPSERT pattern. The timestamp parameter uses `crate::timestamp::now()`.

**Tests:**
- Migration applies cleanly from v42.
- `increment_skill_usage` correctly increments existing rows and creates rows for skills without prior overrides.

### Step 2: Usage tracking at skill-injection site

**File:** `crates/mika-agent/src/agent_loop/mod.rs`

The injection site is `inject_skills_and_resolve_tools()` (line ~4901). This function already iterates over matched skills and writes their prompts into the system prompt. It returns per-skill bytes in `per_skill_bytes: HashMap<String, usize>`.

1. Change the return type to also include the list of injected skill names: add the skill names from `per_skill_bytes.keys()` (or `variant_map.keys()` — same set) to the return value.
2. In the caller (the site that calls `inject_skills_and_resolve_tools` — there are three: conversation, silent, team), after the function returns and before the LLM call, collect the injected skill names into a `Vec<String>`.
3. After the agent loop turn completes (at turn-end, not per-step), batch-increment usage via `db.increment_skill_usage(agent_id, &injected_skill_names)`.

**Turn-end batching rationale:** A single turn injects N skill prompts but makes one LLM call. The usage signal is "this skill was part of the context for a turn" — one increment per turn, not per step. Place the call after `run_loop` returns in each of the three entry points (`run_agent`, `run_silent_agent`, `run_team_agent`).

**Compact-provider gate:** When `is_compact_provider` is true, no skills are injected (line ~4923), so the injected list is empty and no usage tracking fires. Correct by construction.

**Tests:**
- Eval harness test: after a turn that matches a skill, `use_count` increments by 1 and `last_used_at` is populated.
- After a turn with no matched skills, no usage writes.

### Step 3: `SilentTrigger::CuratorReview` variant and recurring task registration

**File:** `crates/mika-agent/src/agent_loop/mod.rs` (SilentTrigger enum, line ~3212)

1. Add new variant:
   ```rust
   /// A periodic curator review to surface archival candidates for idle skills.
   CuratorReview,
   ```
2. In `SilentTrigger::max_steps()`, add `Self::CuratorReview` to the `Heartbeat | Reflection | SkillRun` arm (uses `MAX_TOOL_STEPS`).

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

3. In `dispatch_run_skill()` match (line ~206), add:
   ```rust
   "curator_review" => Ok(self.dispatch_curator_review(task).await?),
   ```
4. Implement `dispatch_curator_review()` following the `dispatch_heartbeat` pattern:
   - Create a session with metadata `{"trigger": "curator_review"}`.
   - Call `run_silent_agent` with `SilentTrigger::CuratorReview`.
   - The curator logic runs inside the silent agent turn via skill matching or inline prompt.

**File:** `crates/mika-agent/src/server/mod.rs`

5. Add a `CURATOR_REVIEW_CRON` constant:
   ```rust
   /// Daily at 03:00 UTC — curator review for skill archival candidates.
   const CURATOR_REVIEW_CRON: &str = "0 0 3 * * *";
   ```
6. In the per-agent recurring task registration loop (line ~1335), add curator review registration. Gate it on the agent having `lifecycle_state`-bearing skills (agent-authored skills exist). For simplicity in Phase 1, register for all agents — the curator query itself returns zero candidates for agents with no authored skills:
   ```rust
   task_engine::ensure_recurring_task(
       &db,
       "curator_review",
       CURATOR_REVIEW_CRON,
       r#"{"trigger":"curator_review"}"#,
   ).await;
   ```

**Configurable interval:** Read `[curator].interval_hours` from agent identity.toml. If set, compute cron from it (e.g., `interval_hours = 12` → `0 0 */12 * * *`). If unset, use the default daily cron. Add `CuratorConfig` to identity parsing in `crates/mika-agent/src/prompt.rs`:
```rust
#[derive(Debug, Deserialize, Default)]
pub struct CuratorConfig {
    pub interval_hours: Option<u32>,
    pub max_idle_days: Option<u32>,
}
```

### Step 4: Curator candidate query and proposal generation

**File:** `crates/mika-agent/src/skills/curator.rs` (new)

1. Create the curator module with:

```rust
pub struct CuratorProposal {
    pub skill_name: String,
    pub days_idle: u64,
    pub use_count: i64,
    pub last_used_at: Option<String>,
    pub recommendation: CuratorRecommendation,
}

pub enum CuratorRecommendation {
    Archive,
    Review,
}
```

2. `get_archival_candidates(db, agent_id, max_idle_days)` DB query:
   ```sql
   SELECT skill_name, use_count, last_used_at
   FROM skill_overrides
   WHERE agent_id = ?
     AND lifecycle_state = 'active'
     AND (
       last_used_at IS NULL AND use_count = 0
       OR last_used_at < ?  -- now - max_idle_days
     )
   ```
   **Structural exclusion of bundled/marketplace skills:** Bundled and marketplace skills have `lifecycle_state IS NULL` (they don't go through the staged→active promotion path). The `lifecycle_state = 'active'` predicate excludes them by construction.

3. Build proposals:
   - `use_count = 0` and `last_used_at IS NULL` (never used) → `Archive` recommendation.
   - `last_used_at < now - max_idle_days` (idle) → `Review` recommendation if `use_count > 5`, `Archive` if `use_count <= 5`.

4. `emit_curator_proposal(proposals)` — emit a structured log event:
   ```rust
   info!(
       event = "curator_proposal",
       agent_id = %agent_id,
       candidate_count = proposals.len(),
       proposals = %serde_json::to_string(&proposals).unwrap_or_default(),
       "curator review completed"
   );
   ```

5. Store the latest proposal in `audit_events` for CLI retrieval:
   ```rust
   db.insert_audit_event(AuditEvent {
       tool_name: "curator_review".to_string(),
       target_key: "curator_proposal".to_string(),
       after_value: Some(serde_json::to_string(&proposals)?),
       ..
   }).await?;
   ```

**File:** `crates/mika-agent/src/skills/mod.rs`

6. Add `pub mod curator;` to the skills module.

**Tests (AC9, AC10, AC11, AC12):**
- AC9: Fresh agent with no authored skills → zero candidates.
- AC10: Agent with one authored skill in `staged` state (not promoted) → zero candidates (curator only queries `lifecycle_state = 'active'`).
- AC11: Agent with one promoted+used skill where `last_used_at < now - 30d` → one candidate.
- AC12: Bundled skill with `NULL lifecycle_state` and `NULL last_used_at` → zero candidates.

### Step 5: Curator dispatch wiring in the silent agent turn

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

In `dispatch_curator_review()`:

```rust
async fn dispatch_curator_review(&self, task: &Task) -> Result<()> {
    let agent_id = &task.agent_id;
    let identity = crate::prompt::load_identity_async(&self.home_dir).await;
    let max_idle_days = identity.curator
        .as_ref()
        .and_then(|c| c.max_idle_days)
        .unwrap_or(30);

    let candidates = crate::skills::curator::get_archival_candidates(
        &self.db, agent_id, max_idle_days
    ).await?;

    if candidates.is_empty() {
        debug!(agent = %agent_id, "curator review: no archival candidates");
        return Ok(());
    }

    crate::skills::curator::emit_curator_proposal(&self.db, agent_id, &candidates).await?;

    // Notify operator if message_sender is available
    if let Some(sender) = &self.message_sender {
        let summary = format!(
            "[Curator] {} skill(s) idle >{}d for agent {}. Run `mika skills curator status --agent {}` for details.",
            candidates.len(), max_idle_days, agent_id, agent_id
        );
        let _ = sender.send(&summary).await;
    }

    Ok(())
}
```

**Design choice:** The curator runs as a direct dispatcher function (like heartbeat/reflection) rather than as a full silent agent loop turn. This avoids LLM costs for a deterministic query+notification task. The `SilentTrigger::CuratorReview` variant exists for the enum completeness but the dispatcher short-circuits before calling `run_silent_agent`.

### Step 6: Archive snapshot capture (extends #1582's archive command)

**File:** `crates/mika-agent/src/skills/curator.rs`

1. Add `capture_archive_snapshot(agent_home: &Path, skill_name: &str) -> Result<PathBuf>`:
   ```rust
   pub fn capture_archive_snapshot(agent_home: &Path, skill_name: &str) -> Result<PathBuf> {
       let skill_dir = agent_home.join("skills").join(skill_name);
       anyhow::ensure!(skill_dir.is_dir(), "skill directory not found: {}", skill_dir.display());

       let archived_dir = agent_home.join("skills").join(".archived");
       std::fs::create_dir_all(&archived_dir)?;

       let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
       let archive_path = archived_dir.join(format!("{}-{}.tar.gz", skill_name, timestamp));

       // Create tar.gz using flate2 + tar crates
       let file = std::fs::File::create(&archive_path)?;
       let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
       let mut tar = tar::Builder::new(enc);
       tar.append_dir_all(skill_name, &skill_dir)?;
       tar.finish()?;

       Ok(archive_path)
   }
   ```

2. The `.archived/` directory is dot-prefixed, structurally invisible to the resolver per `is_bundled_skill_dir` at `crates/mika-agent/src/skills/index.rs:460` (rejects names starting with `.`).

**File:** `crates/mika-cli/src/commands/skills.rs` (or wherever #1582 places the archive command)

3. Integrate `capture_archive_snapshot` as a pre-step in the existing `archive` command flow — call it before the `lifecycle_state` UPDATE.

**Dependencies:** Add `flate2` and `tar` crates to `crates/mika-agent/Cargo.toml`:
```toml
flate2 = "1"
tar = "0.4"
```

**Tests (AC6):**
- Archiving a skill creates a `.tar.gz` in `<agent_home>/skills/.archived/`.
- The archive contains the skill's full directory structure.
- `.archived/` is invisible to `is_bundled_skill_dir`.

### Step 7: CLI `restore` subcommand

**File:** `crates/mika-cli/src/cli.rs`

1. Add `Restore` variant to `SkillsCommand`:
   ```rust
   /// Restore an archived skill from its most recent snapshot
   Restore {
       /// Skill name to restore
       name: String,
   },
   ```

**File:** `crates/mika-cli/src/commands/skills.rs`

2. Implement `run_skill_restore(name, agent_home, db)`:
   - Find the most recent snapshot in `<agent_home>/skills/.archived/<name>-*.tar.gz` by lexicographic sort (timestamps are ISO-formatted, so lexicographic = chronological).
   - Extract the tarball to `<agent_home>/skills/<name>/`.
   - Update `skill_overrides` row: `SET lifecycle_state = 'staged'` (operator must re-promote).
   - Print confirmation with the restored snapshot path.

3. Add `restore_skill_from_snapshot(agent_home, skill_name) -> Result<PathBuf>` to `crates/mika-agent/src/skills/curator.rs`:
   ```rust
   pub fn restore_skill_from_snapshot(agent_home: &Path, skill_name: &str) -> Result<PathBuf> {
       let archived_dir = agent_home.join("skills").join(".archived");
       // Find most recent snapshot
       let pattern = format!("{}-", skill_name);
       let mut snapshots: Vec<_> = std::fs::read_dir(&archived_dir)?
           .filter_map(|e| e.ok())
           .filter(|e| e.file_name().to_string_lossy().starts_with(&pattern)
                    && e.file_name().to_string_lossy().ends_with(".tar.gz"))
           .collect();
       snapshots.sort_by_key(|e| e.file_name());
       let latest = snapshots.last()
           .ok_or_else(|| anyhow::anyhow!("no archived snapshot found for skill '{}'", skill_name))?;

       let skill_dir = agent_home.join("skills").join(skill_name);
       // Remove existing directory if present
       if skill_dir.exists() {
           std::fs::remove_dir_all(&skill_dir)?;
       }

       // Extract tar.gz
       let file = std::fs::File::open(latest.path())?;
       let dec = flate2::read::GzDecoder::new(file);
       let mut archive = tar::Archive::new(dec);
       archive.unpack(agent_home.join("skills"))?;

       Ok(latest.path())
   }
   ```

**Tests (AC7):**
- Restoring an archived skill extracts the snapshot.
- `lifecycle_state` transitions to `'staged'`.
- Missing snapshot returns an error.

### Step 8: CLI `curator status` subcommand

**File:** `crates/mika-cli/src/cli.rs`

1. Add `Curator` variant to `SkillsCommand` with a nested subcommand:
   ```rust
   /// Curator operations
   Curator {
       #[command(subcommand)]
       action: CuratorAction,
   },
   ```
   ```rust
   #[derive(Subcommand)]
   pub enum CuratorAction {
       /// Show the most recent curator review results
       Status,
   }
   ```

**File:** `crates/mika-cli/src/commands/skills.rs`

2. Implement `run_curator_status(db, agent_id)`:
   - Query `audit_events` for the most recent `curator_proposal` event:
     ```sql
     SELECT after_value, created_at FROM audit_events
     WHERE tool_name = 'curator_review' AND target_key = 'curator_proposal'
       AND agent_id = ?
     ORDER BY created_at DESC LIMIT 1
     ```
   - Parse the JSON proposal list and display as a formatted table.
   - If no proposals found, print "No curator review data available."

**Tests (AC5, revised):**
- After a curator tick produces candidates, `mika skills curator status` shows the proposals.
- With no prior curator ticks, shows "no data" message.

### Step 9: HTTP `POST /api/v1/skills/{name}/restore` endpoint

**File:** `crates/mika-agent/src/server/mod.rs` (route registration)

1. Add route:
   ```rust
   .route("/api/v1/skills/{name}/restore", post(handle_skill_restore))
   ```

**File:** `crates/mika-agent/src/server/dashboard.rs` (or `handlers.rs`)

2. Implement `handle_skill_restore`:
   - Extract `name` from path.
   - Resolve agent home from `AppState`.
   - Call `curator::restore_skill_from_snapshot(agent_home, &name)`.
   - Update `skill_overrides.lifecycle_state` to `'staged'`.
   - Return `200 OK` with `{"restored": true, "skill": name, "lifecycle_state": "staged"}`.
   - Requires internal token auth (mutation endpoint).

**Tests (AC8):**
- HTTP POST to restore endpoint extracts snapshot and returns success.

### Step 10: Integration and verification

**Files:** Various test files

1. **AC1 test:** Schema migration v43 applies, columns exist with correct defaults.
2. **AC2 test:** After a turn with skill injection, `use_count` is incremented and `last_used_at` is set.
3. **AC3 test:** `SilentTrigger::CuratorReview` variant exists and is registered with `ensure_recurring_task`.
4. Run `cargo clippy` and `cargo test` to verify no regressions.

## File change summary

| File | Change type | Description |
|------|-------------|-------------|
| `crates/mika-agent/src/db.rs` | Modify | Schema v43 migration, `SkillOverride` struct, `increment_skill_usage()`, curator query |
| `crates/mika-agent/src/agent_loop/mod.rs` | Modify | `SilentTrigger::CuratorReview` variant, usage tracking at turn-end |
| `crates/mika-agent/src/skills/mod.rs` | Modify | Add `pub mod curator;` |
| `crates/mika-agent/src/skills/curator.rs` | New | Curator logic: proposals, snapshot capture, restore |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Modify | `dispatch_curator_review()`, trigger match arm |
| `crates/mika-agent/src/server/mod.rs` | Modify | `CURATOR_REVIEW_CRON`, recurring task registration, restore route |
| `crates/mika-agent/src/server/dashboard.rs` | Modify | `handle_skill_restore` handler |
| `crates/mika-agent/src/prompt.rs` | Modify | `CuratorConfig` in identity parsing |
| `crates/mika-cli/src/cli.rs` | Modify | `Restore` and `Curator` subcommands |
| `crates/mika-cli/src/commands/skills.rs` | Modify | `run_skill_restore()`, `run_curator_status()` |
| `crates/mika-agent/Cargo.toml` | Modify | Add `flate2`, `tar` dependencies |

## Failure-disposition coverage

1. **Schema migration v42→v43** — existing rows get `use_count = 0` by DEFAULT, `last_used_at = NULL`. The curator candidate query has `lifecycle_state = 'active'` which structurally excludes bundled/marketplace skills (NULL lifecycle_state). Unit test asserts zero candidates on all-NULL-lifecycle-state fixture.

2. **Curator proposal emission** — first-tick after deployment fires on every agent-authored `active` skill. Since only operator-promoted skills have `lifecycle_state = 'active'`, the candidate set is bounded by deliberate operator action. Proposals are structured JSON queryable via CLI, not free-text notifications.

## Acceptance criteria

1. **AC1 — Schema migration:** Migration adds `use_count INTEGER NOT NULL DEFAULT 0` and `last_used_at TEXT` columns to `skill_overrides`. Existing rows get safe defaults (0 / NULL). (Steps 1, 10)
2. **AC2 — Usage tracking:** Skill-prompt injection emits usage events; turn-end batch increments `use_count` and sets `last_used_at` for all injected skills. (Steps 2, 10)
3. **AC3 — Curator recurring task:** `SilentTrigger::CuratorReview` is registered via `ensure_recurring_task` with a configurable interval (default daily at 03:00 UTC, overridable via `[curator].interval_hours` in identity.toml). (Step 3)
4. **AC4 — Candidate query:** Curator candidate query selects skills with `lifecycle_state = 'active'` that are idle beyond the threshold. Bundled/marketplace skills (`lifecycle_state IS NULL`) and staged skills are excluded by construction. (Step 4)
5. **AC5 — Structured proposal:** Curator emits a structured JSON proposal (not free-text) stored in `audit_events` and queryable via `mika skills curator status`. The curator does not auto-archive — it proposes only. (Steps 4, 5, 8)
6. **AC6 — Archive snapshot (DEFERRED to follow-up PR):** `capture_archive_snapshot()` is implemented in `crates/mika-agent/src/skills/curator.rs` (tested via round-trip test). Integration into a CLI `archive` command flow is **deferred** because the `archive` CLI subcommand lives in mika#1582 (PR #1623), which ships first in the milestone sequence. After PR #1623 lands, a follow-up PR wires `capture_archive_snapshot()` as the pre-step in the `Archive` subcommand handler. The capture function itself is shipped here so #1623's `archive` handler has the API ready to call. (Step 6)
7. **AC7 — CLI restore:** CLI `mika skills restore <name>` extracts the most recent snapshot and sets `lifecycle_state = 'staged'` (operator must re-promote). Missing snapshot returns an error. (Step 7)
8. **AC8 — HTTP restore:** `POST /api/v1/skills/{name}/restore` mirrors the CLI restore behavior — extracts snapshot, sets `lifecycle_state = 'staged'`, returns structured JSON response. Requires internal token auth. (Step 9)
9. **AC9 — Test: fresh agent:** Fresh agent with no agent-authored skills → curator returns zero candidates. (Step 4 tests)
10. **AC10 — Test: staged skill:** Agent with a staged (not promoted) skill → curator returns zero candidates (curator only considers `lifecycle_state = 'active'`). (Step 4 tests)
11. **AC11 — Test: idle promoted skill:** Agent with a promoted+active skill where `last_used_at` is older than 30 days → curator returns one candidate. (Step 4 tests)
12. **AC12 — Test: bundled/marketplace:** Bundled or marketplace skill with `NULL lifecycle_state` and `NULL last_used_at` → curator returns zero candidates. (Step 4 tests)

## Out of scope

- Auto-archive without operator action (Phase 2).
- Auto-promote restored skills to `active` (Phase 1 restores to `staged`).
- Snapshot pruning / `max_archived_snapshots_per_skill` config.
- `view_count` / `patch_count` tracking (deferred to follow-on).
- Cross-tenant skill usage aggregation.

## Revision history

- rev 2 (2026-06-28): addressed F1 by adding `## Acceptance criteria` section transcribing all 12 ACs from the issue body (AC1–AC12), with cross-references to the plan steps that implement each. Citation: mika#1559 Acceptance-Criteria Gate; mika#1585 qa-review `block[pipeline]`.
