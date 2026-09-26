---
title: "Team Engine Code Review Findings Batch (441-449)"
date: 2026-03-04
status: solved
category: logic-errors
component:
  - mika-agent/teams/engine
  - mika-agent/teams/prompt
  - mika-agent/teams/types
  - mika-agent/db
  - mika-cli/tui
severity: P1-P3 (2 critical, 5 important, 4 nice-to-have)
tags:
  - utf8-safety
  - thread-leak
  - error-handling
  - timestamp-consistency
  - prompt-injection
  - query-performance
  - type-projection
  - path-validation
  - code-duplication
  - exhaustive-matching
symptoms:
  - Panic on UTF-8 boundary truncation in task descriptions
  - OS thread leak per team run (missing AsyncDatabase shutdown)
  - Silent failures in team message persistence (7 sites)
  - Timestamp format divergence between struct fields and DB columns
  - Unbounded history injection into orchestrator system prompt
  - Full table scan on every task assignment lookup
  - TeamEvent data silently dropped in TUI display pipeline
  - Null bytes and backslashes bypassing output_file validation
  - Duplicate iteration calculation in decompose()
  - CLI teams command dropping structured events via wildcard match
related_issues:
  - todos/441-p1-utf8-truncation-panic.md
  - todos/442-p1-team-db-shutdown-thread-leak.md
  - todos/443-p2-silent-error-discard-insert-team-message.md
  - todos/444-p2-dual-timestamp-representation.md
  - todos/445-p2-unbounded-history-prompt-injection.md
  - todos/446-p2-full-table-scan-assignment-lookup.md
  - todos/447-p2-team-response-event-impedance-mismatch.md
  - todos/448-p3-incomplete-output-file-path-validation.md
  - todos/449-p3-iteration-calculation-duplication.md
  - todos/435-complete-p2-double-logging-in-emit-event.md
  - todos/440-complete-p3-teams-run-cli-drops-events.md
related_docs:
  - docs/adr/004-multi-agent-teams-orchestration.md
  - docs/plans/2026-03-03-feat-team-tui-mode-plan.md
  - docs/plans/2026-03-03-feat-team-graph-persistence-verbose-mode-plan.md
  - docs/solutions/integration-issues/team-tui-mode-cli-integration.md
  - docs/solutions/database-issues/team-graph-persistence-replacing-toml-history.md
  - docs/solutions/runtime-errors/team-agent-max-steps-exhaustion-no-output.md
---

# Team Engine Code Review Findings Batch (441-449)

## Problem Statement

A multi-agent code review of the `feat/team-tui-mode` branch (19 commits, 53 files, ~3900 insertions) identified 11 findings across the team orchestration engine, prompt assembly, type system, database layer, and CLI integration. Two were critical (crash + resource leak), five were important (silent failures, type mismatches, security, performance), and four were nice-to-have (validation gaps, duplication, logging).

All findings share a common theme: logic-level correctness gaps in a rapidly-developed feature branch.

## Root Cause Analysis

### P1-441: UTF-8 Truncation Panic

`task[..5000].to_string()` panics if byte 5000 falls mid-character. Multi-byte UTF-8 characters (emojis, CJK, accented letters) spanning the boundary trigger `byte index is not a char boundary`.

### P1-442: Missing team_db.shutdown() — Thread Leak

`TeamEngine::execute()` shuts down per-agent `AsyncDatabase` instances but not `self.team_db`. Each `AsyncDatabase` spawns a dedicated OS thread; without `shutdown()`, that thread persists indefinitely. One leaked thread per team run.

### P2-443: Silent `let _ =` on insert_team_message (7 sites)

Seven call sites used `let _ = self.team_db.insert_team_message(...)`, silently discarding database insert errors. Zero observability when persistence fails — messages simply vanish from the execution graph.

### P2-444: Dual Timestamp Representation

`TeamRun.started_at` was `String` (RFC 3339), while the DB schema stored `i64` (Unix timestamp). Separate `chrono::Utc::now()` calls at struct creation vs. DB insert could produce divergent values.

### P2-445: Unbounded History Prompt Injection

Up to 10 goals (no length limit) and 10 deliverables (500 char limit) injected into the orchestrator system prompt without delimiters or a total character budget. A single long goal could inject ~100K chars, drowning out decomposition instructions.

### P2-446: Full Table Scan for Assignment Lookup

`load_team_messages(&run_id)` fetched ALL messages for a run, then filtered in Rust for assignment messages matching the current iteration. O(n) per iteration where n is total messages.

### P2-447: TeamResponse/TeamEvent Impedance Mismatch

`TeamResponse` (4 variants) was a lossy projection of `TeamEvent` (7 variants). `Deliverable` and `RunFailed` were silently dropped in the callback bridge. `CriticReview` lost its structured `iteration` field. The TUI could never display rich structured events.

### P3-448: Incomplete Path Validation

`output_file` validation checked for `..` and leading `/` but missed null bytes (`\0`) and backslashes (`\`), which could escape canonicalization.

### P3-449: Iteration Calculation Duplication

Identical `if feedback.is_some() { iteration + 1 } else { 1 }` block duplicated at two sites in `decompose()`.

### P3-435: Double Logging in emit_event

`AgentCompleted`, `AgentFailed`, and `RunFailed` events were logged both at their call sites and again inside `emit_event()`, producing duplicate log lines.

### P3-440: CLI Events Dropping

The CLI `teams run` callback used a wildcard `_ => {}` match, silently dropping `TasksAssigned`, `CriticReview`, and other structured events.

## Solution

### P1-441: UTF-8 Safe Truncation

```rust
// Before: panics on multi-byte boundary
task[..5000].to_string()

// After: finds valid UTF-8 boundary
task[..task.floor_char_boundary(5000)].to_string()
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P1-442: Add Team DB Shutdown

```rust
// After agent DB shutdown loop:
for resources in self.agents.values() {
    resources.db.shutdown();
}
self.team_db.shutdown();  // Added — prevents thread leak
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P2-443: Error Logging on DB Inserts

Replaced `let _ =` with `warn!` logging. Used collapsed `if let Some(..) && let Err(..) =` pattern for clippy compliance:

```rust
if let Some(goal_id) = self.goal_msg_id
    && let Err(e) = self.team_db
        .insert_team_message(&self.run.run_id, Some(goal_id), ...)
        .await
{
    warn!(error = %e, "failed to persist team message");
}
```

**File:** `crates/mika-agent/src/teams/engine.rs` (7 sites)

### P2-444: Unified Timestamp to i64

```rust
pub struct TeamRun {
    pub started_at: i64,       // Was: String
    pub ended_at: Option<i64>, // Was: Option<String>
}
```

Format only at display time via `format_unix_ts()` helper in `db.rs`.

**Files:** `types.rs`, `engine.rs`

### P2-445: Bounded History with Delimiters

- Truncate each goal and deliverable to 500 chars using `floor_char_boundary()`
- Wrap in `<context type="history_goal">` / `<context type="history_deliverable">` delimiters
- Enforce 5000-char total budget with saturation subtraction

**File:** `crates/mika-agent/src/teams/prompt.rs`

### P2-446: Targeted SQL for Assignment Lookup

Added `load_assignment_msg_ids(run_id, iteration) -> HashMap<String, i64>`:

```sql
SELECT id, agent_name FROM team_messages
WHERE run_id = ?1 AND message_type = 'assignment' AND iteration = ?2
```

Replaces fetching all messages and filtering in Rust.

**Files:** `db.rs`, `async_db.rs`, `engine.rs`

### P2-447: Eliminate TeamResponse, Use TeamEvent Directly

- Removed `TeamResponse` enum entirely
- Channel type changed to `mpsc::unbounded_channel::<TeamEvent>()`
- Callback simplified to `let _ = tx.send(event);`
- `tick_team_mode()` handles all 7 `TeamEvent` variants exhaustively

**Files:** `tui/app.rs`, `commands/chat.rs`

### P3-448: Extended Path Validation

```rust
if output_file.contains("..")
    || output_file.starts_with('/')
    || output_file.contains('\0')  // Added
    || output_file.contains('\\')  // Added
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P3-449: Extract Iteration Calculation

Moved `let iteration = ...` before the match block so both `Conversational` and `Tasks` branches share the same calculation.

**File:** `crates/mika-agent/src/teams/engine.rs`

### P3-435: Remove Double Logging

Skip logging in `emit_event()` for variants already logged at call sites:

```rust
TeamEvent::AgentCompleted { .. }
| TeamEvent::AgentFailed { .. }
| TeamEvent::RunFailed(_) => {
    // Already logged at the call site; skip here to avoid duplicates.
}
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P3-440: Exhaustive CLI Event Match

Replaced `_ => {}` with explicit match arms for all `TeamEvent` variants, including `TasksAssigned` (prints agent names) and `CriticReview` (prints iteration number and verdict).

**File:** `crates/mika-cli/src/commands/teams.rs`

## Prevention Strategies

### 1. UTF-8 String Truncation

Enable `clippy::string_slicing` to warn on direct byte-indexing:

```toml
[lints.clippy]
string_slicing = "warn"
```

Always use `floor_char_boundary()` before slicing user-controlled or LLM-generated text. The pattern already exists at `prompt.rs:41` and `get_team_status.rs` — standardize it.

### 2. AsyncDatabase Resource Cleanup

Every `AsyncDatabase::new()` call must have a corresponding `shutdown()`. Consider an RAII wrapper:

```rust
impl Drop for AsyncDatabaseGuard {
    fn drop(&mut self) { self.db.shutdown(); }
}
```

Mark `AsyncDatabase` with `#[must_use]` to catch unbound instances.

### 3. Silent Error Discard

Enable `clippy::let_underscore_drop` to flag `let _ = fallible_op()`:

```toml
[lints.clippy]
let_underscore_drop = "warn"
```

Reserve `let _ =` only for infallible operations (e.g., `writeln!` to `String`). All database operations must use `warn!` logging or `?` propagation.

### 4. Type Representation Consistency

Use a single timestamp representation (`i64` Unix seconds) throughout. Format only at display boundaries. Consider a `UnixTimestamp` newtype for compile-time safety.

### 5. Prompt Injection Defense

All user-controlled data injected into LLM prompts must be:
- **Length-bounded** (per-entry and total budget)
- **Delimiter-wrapped** (`<context type="...">` tags)
- **Truncated safely** (UTF-8 boundary-aware)

### 6. Lossy Type Projections

Avoid creating subset enum types that project from a richer source. Use the canonical type (`TeamEvent`) everywhere. If different consumers need different handling, use exhaustive `match` at the consumption site.

### 7. Wildcard Match Arms

Enable `clippy::wildcard_enum_match_arm`:

```toml
[lints.clippy]
wildcard_enum_match_arm = "warn"
```

This forces explicit handling of all enum variants, ensuring new variants trigger compile errors at all consumption sites.

## Verification

- **Tests:** 837 passing (578 mika-agent + 116 mika-cli + 91 mika-common + 52 mika-gateway)
- **Clippy:** Clean (no warnings)
- **Formatting:** Clean (`cargo fmt --check` passes)
- **Commit:** `f8469e6` on `feat/team-tui-mode` branch
