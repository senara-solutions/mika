---
title: "feat: Unify session/run continuation under -c/--continue flag"
date: 2026-06-14
type: feat
issue: mika#462
origin: null
depth: Standard
---

# feat: Unify session/run continuation under `-c`/`--continue` flag

## Summary

The CLI has two semantically identical "resume where I left off" features with different names and flags: `--session-id` for chat sessions and `--last-run`/`--run-id` for team runs. This plan adds `-c`/`--continue` as a universal "resume last" flag that works for both solo sessions and team runs, deprecates `--last-run` in favor of `-c`, and keeps `--session-id`/`--run-id` for explicit ID targeting.

---

## Problem Frame

Users who want to continue a previous conversation must remember different flags depending on whether they're in solo or team mode. `--session-id` requires knowing the UUID; there's no "resume last" shortcut for solo sessions. Team mode has `--last-run` but the name doesn't suggest it belongs to a family of continuation flags. Unifying under `-c` makes the CLI more consistent and discoverable.

---

## Requirements

- **R1.** `mika -c` / `mika chat -c` resumes the most recent CLI chat session for the active agent.
- **R2.** `mika ask -c "follow up"` sends a message in the most recent CLI session for the active agent.
- **R3.** `mika chat --team dev -c` resumes the last finished team run (same semantics as current `--last-run`).
- **R4.** `mika ask --team dev -c "iterate"` resumes the last finished team run (same semantics as current `--last-run`).
- **R5.** `-c` is mutually exclusive with `--session-id` and `--run-id`.
- **R6.** `--last-run` is deprecated with a warning pointing users to `-c`.
- **R7.** `--session-id` and `--run-id` remain unchanged for explicit ID targeting.

---

## Key Technical Decisions

**KTD-1. `-c` placement: per-subcommand, not global.**
`--session-id` is a global flag, but `-c` should be per-subcommand on `ChatArgs` and `AskArgs` because its resolution semantics differ by context (solo vs team) and it conflicts with subcommand-specific flags (`--run-id`, `--session-id`). Making it global would require resolving conflicts across unrelated subcommands.

**KTD-2. "Last session" scope: CLI-originated (`channel_type = 'cli'`) conversations only.**
The `sessions` table contains sessions from multiple channel types (cli, telegram, webhook, system, delegate). Resuming a webhook or system session via `-c` would be confusing and likely broken. The query scopes to `channel_type = 'cli'` and excludes system sessions (`id NOT LIKE 'system-%'`), delegate sessions (`id NOT LIKE 'delegate-%'`), and sessions with a `parent_session_id` (child sessions from delegation).

**KTD-3. Session ordering: `started_at DESC` with `ended_at IS NOT NULL` filter.**
Resume the most recently *started* session that has been ended (completed). Resuming a session that's still active would cause concurrency issues. If no ended session exists, error with a clear message.

**KTD-4. Deprecation strategy: runtime `eprintln!` warning, no removal.**
`--last-run` remains functional but prints a deprecation notice to stderr on use: `Warning: --last-run is deprecated; use -c instead.` The flag is not hidden from help text yet — hiding happens in a follow-up after one release cycle.

---

## Scope Boundaries

### In Scope

- Adding `-c`/`--continue` flag to `ChatArgs` and `AskArgs`
- New DB query `get_last_cli_session_for_agent(agent_id)`
- Resolution logic in `main.rs` and `commands/chat.rs`/`commands/ask.rs`
- Deprecation warning for `--last-run`
- Tests for the new flag, DB query, and conflict rules

### Deferred to Follow-Up Work

- Hiding `--last-run` from `--help` output (after one release cycle)
- Removing `--last-run` entirely (after two release cycles)
- Adding `-c` to `mika teams log` or other subcommands

---

## Implementation Units

### U1. Add `get_last_cli_session_for_agent` DB query

**Goal:** Provide a database method to retrieve the most recent ended CLI session for an agent.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/db.rs` (modify — add query method)

**Approach:**
Add a method on `Database`:
```
get_last_cli_session_for_agent(agent_id: &str) -> Result<Option<Session>>
```
Query: select from `sessions` where `agent_id = ?`, `channel_type = 'cli'`, `ended_at IS NOT NULL`, `id NOT LIKE 'system-%'`, `id NOT LIKE 'delegate-%'`, `parent_session_id IS NULL`, ordered by `started_at DESC`, limit 1.

**Patterns to follow:** `get_last_finished_team_run` at db.rs:8524 — same single-row lookup pattern with `query_row` + `optional()`.

**Test scenarios:**
- Returns `None` when no CLI sessions exist for the agent
- Returns the most recent ended CLI session when multiple exist
- Excludes sessions with `channel_type != 'cli'` (e.g., telegram, webhook)
- Excludes system sessions (`system-*` prefix)
- Excludes delegate sessions (`delegate-*` prefix)
- Excludes sessions with a `parent_session_id` (child/delegated sessions)
- Excludes sessions where `ended_at IS NULL` (still active)

**Verification:** `cargo test -p mika-agent` passes with the new tests covering all filter conditions.

---

### U2. Add `-c`/`--continue` flag to `ChatArgs` and `AskArgs`

**Goal:** Define the CLI flag with correct conflict rules via clap attributes.

**Requirements:** R5, R7

**Dependencies:** None

**Files:**
- `crates/mika-cli/src/cli.rs` (modify — add flag to both structs)

**Approach:**
Add to `ChatArgs`:
```rust
#[arg(short = 'c', long = "continue", conflicts_with_all = ["run_id", "session_id"])]
pub continue_session: bool,
```
Note: `continue` is a Rust keyword, so the field name is `continue_session` with `long = "continue"` to get the `--continue` CLI flag. The `session_id` conflict is against the global `Cli.session_id` — clap's `conflicts_with` resolves by field name across the flattened arg group. Since `session_id` is a global arg on `Cli` (not flattened into `ChatArgs`), the conflict must be validated at resolution time in `main.rs` rather than via clap attributes. Add `conflicts_with = "run_id"` only; validate `session_id` conflict manually.

Add the same flag to `AskArgs` with `conflicts_with_all = ["run_id", "last_run"]`. Same manual validation needed for `session_id`.

**Patterns to follow:** `last_run` flag definition at cli.rs:167 — same boolean flag pattern with `requires`/`conflicts_with`.

**Test scenarios:**
- `mika chat -c` parses successfully
- `mika chat --continue` parses successfully
- `mika chat -c --run-id <uuid>` fails at parse time (clap conflict)
- `mika ask -c "msg"` parses successfully
- `mika ask -c --last-run --team dev "msg"` fails at parse time (clap conflict)

**Verification:** `cargo test -p mika-cli` passes; `cargo build -p mika-cli && ./target/debug/mika chat --help` shows `-c`/`--continue` in help output.

---

### U3. Wire `-c` resolution into the team-mode branch

**Goal:** When `-c` is used with `--team`, resolve to the last finished team run (same as `--last-run`).

**Requirements:** R3, R4

**Dependencies:** U2

**Files:**
- `crates/mika-cli/src/main.rs` (modify — team-mode branch at lines 77-138)

**Approach:**
In the team-mode early-exit section of `main()`, extend the `run_id` resolution logic for both `Chat` and `Ask` arms:
- Current: `if last_run { resolve_last_run() } else { explicit_run_id }`
- New: `if last_run || continue_session { resolve_last_run() } else { explicit_run_id }`

Extract `continue_session` from `ChatArgs`/`AskArgs` alongside `last_run` and `run_id`.

**Patterns to follow:** Existing `last_run` resolution at main.rs:87-91.

**Test scenarios:**
- `mika chat --team dev -c` resolves to the last team run (same as `--last-run`)
- `mika ask --team dev -c "msg"` resolves to the last team run
- Error message when no finished team run exists

**Verification:** Manual: `mika chat --team dev -c` behaves identically to `mika chat --team dev --last-run`.

---

### U4. Wire `-c` resolution into the solo-mode branch

**Goal:** When `-c` is used without `--team`, resolve to the last CLI session for the active agent.

**Requirements:** R1, R2

**Dependencies:** U1, U2

**Files:**
- `crates/mika-cli/src/main.rs` (modify — solo-mode `None | Chat | Ask` branches at lines 229-303)

**Approach:**
For the `None` (bare `mika`) and `Chat` branches:
- If `continue_session` is set (from `ChatArgs` or inferred from bare `mika -c`), open the agent DB, call `get_last_cli_session_for_agent`, and pass the session ID to `commands::chat::run`.
- Validate manually that `cli.session_id` is `None` when `continue_session` is true (the global `--session-id` can't use clap `conflicts_with` against subcommand fields).

For the bare `mika` branch (no subcommand), `-c` needs to be accessible. Since bare `mika` has no subcommand args, `-c` must be added as a top-level field on `Cli` as well (similar to `--team` and `--session-id`). Add `#[arg(short = 'c', long = "continue")]` to `Cli` and merge top-level + subcommand values (same pattern as `team_override`).

For the `Ask` branch:
- Same resolution: call `get_last_cli_session_for_agent`, pass session ID to `commands::ask::run`.
- Add manual `session_id` conflict check.

**Patterns to follow:** `team_override` merging at main.rs:60-64 — same top-level + subcommand merge pattern.

**Files (additional):**
- `crates/mika-cli/src/cli.rs` (modify — add `-c` to `Cli` struct, add `continue_override()` to `Commands`)

**Test scenarios:**
- `mika -c` resumes the last CLI session in chat mode
- `mika chat -c` resumes the last CLI session
- `mika ask -c "follow up"` sends a message in the last CLI session
- Error when no ended CLI session exists: "No previous session found for agent '<name>'. Start a chat first before using -c."
- `mika -c --session-id <uuid>` fails with conflict error

**Verification:** Manual: `mika chat -c` opens the TUI with the last session's messages loaded.

---

### U5. Deprecate `--last-run` with a warning

**Goal:** Print a deprecation notice when `--last-run` is used.

**Requirements:** R6

**Dependencies:** U3

**Files:**
- `crates/mika-cli/src/main.rs` (modify — add warning before resolution)

**Approach:**
In both team-mode branches (Chat and Ask), when `last_run` is true, emit to stderr:
```rust
eprintln!("Warning: --last-run is deprecated; use -c instead.");
```
Place this before the `resolve_last_run` call. The flag continues to work — the deprecation is informational only.

**Patterns to follow:** Standard Rust CLI deprecation via `eprintln!` to stderr (does not interfere with stdout-based output for `mika ask --format json`).

**Test scenarios:**
- `mika chat --team dev --last-run` prints deprecation warning to stderr and still works
- `mika ask --team dev --last-run "msg"` prints deprecation warning to stderr and still works
- `mika chat --team dev -c` does NOT print the deprecation warning

**Verification:** `mika chat --team dev --last-run 2>&1 | grep -c "deprecated"` returns 1.

---

### U6. Add `AsyncDatabase` wrapper for `get_last_cli_session_for_agent`

**Goal:** Expose the new DB query through the async database wrapper used by the CLI.

**Requirements:** R1, R2

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/db.rs` (modify — add async wrapper on `AsyncDatabase`)

**Approach:**
Add an async method on `AsyncDatabase` that delegates to the sync `Database::get_last_cli_session_for_agent` through the existing `with_db` channel pattern. Follow the exact pattern of other async wrappers like `get_session`.

**Patterns to follow:** `AsyncDatabase::get_session` — same `with_db` + closure delegation shape.

**Test scenarios:**
- Test expectation: none — thin async wrapper over a sync method already tested in U1.

**Verification:** Compiles and is callable from `main.rs` in U4.

---

## Open Questions

**Q1. Should `-c` without `--team` require an ended session, or allow resuming an active one?**
Decision: Require ended session only (KTD-3). Resuming an active session risks two concurrent writers to the same session. If needed in the future, add `--attach` for live session attachment.

---

## System-Wide Impact

- **CLI help text:** New `-c`/`--continue` flag appears in `mika --help`, `mika chat --help`, and `mika ask --help`. `--last-run` remains visible but gains deprecation mention in its description.
- **CLI reference:** Auto-generated `cli-reference.md` (via `clap_markdown`) updates automatically on next run.
- **No server/gateway/dashboard impact:** This is purely a CLI-side change.
- **No schema migration:** Uses existing `sessions` table with existing columns.

---

## Sources & Research

- Ticket: mika#462
- Existing `--last-run` implementation: `crates/mika-cli/src/main.rs:338-348`
- Existing `--session-id` usage: `crates/mika-cli/src/cli.rs:14-17`, `crates/mika-cli/src/commands/chat.rs:68-88`
- `Session` struct: `crates/mika-agent/src/db.rs:128-137`
- `get_last_finished_team_run`: `crates/mika-agent/src/db.rs:8524-8535`
