---
title: "TUI polish: skill visibility and /clear cleanup"
type: feat
status: completed
date: 2026-04-03
issues: ["#409", "#391", "#390", "#347"]
---

# TUI Polish: Skill Visibility and /clear Cleanup

## Overview

Group of TUI improvements addressing three gaps: (1) skipped always_on skills are invisible at startup, (2) the `/skills` command hides failed skills entirely, and (3) `/clear` leaves stale state that can cause ghost responses and semantic bugs. All changes are in `mika-cli` and `mika-agent/skills` — no agent loop impact.

## Problem Statement

1. **Silent skill failures (#391, #390):** `scan_skills_dir()` logs skipped skills via `tracing::warn!` but the TUI user sees nothing. `ScanResult` only tracks `skipped_count: usize` — no names, no reasons. Users discover broken skills only by reading log files or running `mika skills validate`.

2. **Incomplete `/clear` (#347):** `/clear` resets session/messages/scroll but misses `pending_response`, `reveal_index`, `has_new_message`, `selection_state`, `pending_images`, `pending_command`, `pending_task_count`, and `status`. The most dangerous gap: if `/clear` runs while the agent is thinking, the in-flight response arrives and creates a ghost message in the new session.

## Proposed Solution

### Part 1: Data model — `SkippedSkill` struct

**File:** `crates/mika-agent/src/skills/index.rs`

Add a `SkippedSkill` struct alongside `ScanResult`:

```rust
/// A skill that was found but could not be loaded.
#[derive(Debug, Clone)]
pub struct SkippedSkill {
    /// Directory name (not manifest name — manifest may be unreadable).
    pub name: String,
    /// Human-readable reason for skipping.
    pub reason: String,
}
```

Extend `ScanResult`:

```rust
pub struct ScanResult {
    pub entries: Vec<SkillEntry>,
    pub skipped_count: usize,
    pub skipped: Vec<SkippedSkill>,
}
```

Update all 7 skip paths in `scan_skills_dir()` to push to `skipped`:

| Line range | Skip reason | Name source |
|---|---|---|
| 190-208 | Broken symlink | `dir_entry.file_name()` |
| 218-228 | Oversized skill.toml | `dir_entry.file_name()` |
| 232-237 | Unreadable manifest | `dir_entry.file_name()` |
| 240-248 | Legacy format | `dir_entry.file_name()` |
| 250-257 | Invalid TOML | `dir_entry.file_name()` |
| 287-299 | Oversized always_on prompt | `manifest.skill.name` |
| 311-319 | Unreadable always_on prompt | `manifest.skill.name` |

**File:** `crates/mika-agent/src/skills/mod.rs`

Extend `SkillRegistry`:

```rust
pub struct SkillRegistry {
    skills: Vec<SkillEntry>,
    skipped_count: usize,
    skipped: Vec<SkippedSkill>,  // NEW
}
```

Add accessor: `pub fn skipped(&self) -> &[SkippedSkill]`.

Update `apply_overrides()` to push to `self.skipped` when removing post-override skills with empty prompts (around line 156), with reason `"removed: always_on override but prompt is empty"`.

### Part 2: Startup warning (#391)

**File:** `crates/mika-cli/src/commands/chat.rs`

After `App::new()` (around line 490), check `skill_registry.skipped()`. If non-empty, inject a `ChatRole::System` message into `app.messages`:

```
⚠ {N} skill(s) skipped at startup:
  • {name}: {reason}
  • {name}: {reason}
  ... and {M} more. Run `mika skills validate` for details.
```

- Show up to 5 skipped skills inline.
- If more than 5, truncate with "... and N more" summary.
- Use `ChatRole::System` — consistent with team mode's previous-run context pattern.
- Only at startup — hot-reload changes are visible via `/skills`.

### Part 3: `/skills` skipped section (#390)

**File:** `crates/mika-cli/src/tui/commands/handlers.rs`

Update `handle_skills()` to append a "SKIPPED" section after the existing "ALWAYS ON" and "ON DEMAND" sections:

```
─── SKIPPED ───
✗ broken-skill    broken symlink → /path/to/target
✗ bad-manifest    invalid TOML: expected `=` at line 3
```

- Use `✗` (or `x`) symbol instead of the `●` bullet used for loaded skills.
- Show name and reason on each line.
- If no skipped skills, omit the section entirely.
- Update header: `"Loaded skills (12, 2 skipped):"` when skipped > 0.

### Part 4: `/clear` state reset (#347)

**File:** `crates/mika-cli/src/tui/commands/handlers.rs`

Add these resets to `handle_clear()`:

```rust
// Session state (already done)
// app.session_id = new_session_id;
// app.messages.clear();
// app.scroll_offset = 0;
// app.last_seen_msg_id = None;
// app.context_tokens = None;
// app.messages_layout = None;

// Response state (NEW)
app.pending_response = None;
app.reveal_index = 0;
app.status = AgentStatus::Idle;

// Input state (NEW)
app.pending_images.clear();
app.pending_command = None;

// UI state (NEW)
app.has_new_message = false;
app.selection_state = SelectionState::None;
app.pending_task_count = 0;
```

**In-flight response handling:** The worker thread may have an in-flight response for the old session. Two-part solution:

1. **Set `status = AgentStatus::Idle`** — stops tick-driven progressive reveal.
2. **Add a `clear_generation: u64` counter to `App`** — increment in `handle_clear()`. In `poll_responses()`, before applying an `AgentResponse`, check that the response's generation matches. If stale, discard it.

Implementation: Add `clear_generation: u64` field to `App` (default 0). Pass current generation via a new `AgentRequest::NewSession { generation: u64 }` variant (or embed in existing). Worker echoes generation back in `AgentResponse`. `poll_responses()` discards responses where `response.generation < app.clear_generation`.

**Alternative (simpler):** Since `AgentRequest::NewSession` is already sent, and the worker processes requests sequentially, the worker will see `NewSession` before processing the next user message. The in-flight response for the *old* request is already queued in `agent_rx`. Drain the channel in `handle_clear()` after sending `NewSession`:

```rust
// Drain any pending responses from the old session
while app.agent_rx.try_recv().is_ok() {}
```

This is simpler and correct because the agent worker runs synchronously per turn — when `/clear` is invoked, the worker has either (a) already sent its response (drain catches it) or (b) is still processing (response will arrive later, but `pending_response = None` + `status = Idle` means `poll_responses()` will set them fresh — no ghost since messages list is cleared). The drain approach is sufficient.

**Fields intentionally preserved across `/clear`:**
- `thinking_level` — user preference, not session state
- `model` / `provider` — user preference
- `skills` — loaded from disk, not session-scoped
- `team_dashboard` — team mode only, separate lifecycle
- `tick_count` — cosmetic animation counter

### Part 5: Tests (#347)

**File:** `crates/mika-cli/src/tui/commands/handlers.rs` (test module)

New tests to add:

1. **`test_clear_resets_all_state_fields`** — Set `pending_response`, `reveal_index`, `has_new_message`, `selection_state`, `pending_images`, `pending_command`, `pending_task_count`, `status` to non-default values. Call `handle_clear`. Assert all reset.

2. **`test_clear_while_thinking`** — Set `status = AgentStatus::Thinking`. Call `handle_clear`. Assert `status == AgentStatus::Idle`.

3. **`test_clear_while_responding`** — Set `status = AgentStatus::Responding(42)`, `pending_response = Some(...)`, `reveal_index = 10`. Call `handle_clear`. Assert all reset.

4. **`test_clear_with_pending_images`** — Push images to `pending_images`. Call `handle_clear`. Assert empty.

5. **`test_clear_preserves_preferences`** — Set `thinking_level` to non-default. Call `handle_clear`. Assert preserved.

6. **`test_skills_shows_skipped_section`** — Create a `SkillRegistry` with skipped entries. Call `handle_skills`. Assert output contains "SKIPPED" section with skill names and reasons.

7. **`test_skills_hides_skipped_when_none`** — Create a `SkillRegistry` with no skipped entries. Call `handle_skills`. Assert output does NOT contain "SKIPPED".

## Acceptance Criteria

- [x] `SkippedSkill` struct with name + reason in `skills/index.rs`
- [x] `ScanResult.skipped: Vec<SkippedSkill>` populated by all 7 skip paths
- [x] `SkillRegistry.skipped` field + `skipped()` accessor
- [x] `apply_overrides()` pushes to `skipped` on post-override removal
- [x] TUI startup shows system message when skills are skipped (max 5 inline)
- [x] `/skills` shows "SKIPPED" section with `✗` badge and reason
- [x] `/skills` header shows skipped count when > 0
- [x] `/clear` resets: `pending_response`, `reveal_index`, `status`, `pending_images`, `pending_command`, `has_new_message`, `selection_state`, `pending_task_count`
- [x] `/clear` drains stale responses from `agent_rx`
- [x] `/clear` preserves: `thinking_level`, model/provider, skills
- [x] Tests: 7 new tests covering state reset, preferences preservation, skipped skills display
- [x] All existing tests pass (`cargo test`)
- [x] `cargo clippy` clean

## Key Files

| File | Changes |
|---|---|
| `crates/mika-agent/src/skills/index.rs` | `SkippedSkill` struct, `ScanResult.skipped`, update 7 skip paths |
| `crates/mika-agent/src/skills/mod.rs` | `SkillRegistry.skipped` field + accessor, `apply_overrides()` |
| `crates/mika-cli/src/commands/chat.rs` | Startup warning injection |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `handle_clear()` resets, `handle_skills()` skipped section, tests |
| `crates/mika-cli/src/tui/app.rs` | (possibly) `clear_generation` field if drain approach insufficient |

## Sources

- Related issues: #409 (umbrella), #391, #390, #347
- Learnings: `docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md`
- Learnings: `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md`
- Learnings: `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md`
