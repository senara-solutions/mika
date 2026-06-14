# Plan: `mika logs` subcommand for cross-surface activity inspection (mika#851)

## Summary

Extend the existing `mika logs` command from a simple log-path printer into a subcommand-bearing command with an `activity` subcommand that queries messages, llm_calls, and tool_calls from SQLite, interleaves them chronologically, and renders them as a unified timeline. The existing path-printing behavior moves to `mika logs paths` (backward-compatible bare `mika logs` defaults to `paths`).

## Design decisions

### D1 — Subcommand structure, not flags on the bare command

The ticket proposes `mika logs [OPTIONS]` as a flat command. However, the existing `mika logs` already prints log file paths. Rather than breaking that behavior, we introduce subcommands:

- `mika logs paths` — current behavior (show log file paths). **Default** when no subcommand is given, preserving backward compatibility.
- `mika logs activity` — the new cross-surface activity query.

This follows the pattern established by `mika tasks` (where bare `mika tasks` aliases `mika tasks list`).

### D2 — DB-only by default, no server_log grep in v1

The `--include server_log` option from the ticket sketch requires reading a potentially multi-GB log file. For v1, we scope to DB-only surfaces (messages, llm_calls, tool_calls). Server log integration can be a follow-up. The `--include` flag is not implemented in v1 — all three DB surfaces are always included.

### D3 — Duration parsing with a simple custom parser

No `humantime` dependency. Implement a minimal `parse_duration_to_iso` that handles `30m`, `2h`, `1d`, `today` and converts to an ISO 8601 timestamp by subtracting from `chrono::Utc::now()`. This keeps the dependency footprint small and matches the ticket's proposed syntax.

### D4 — No `--tail` / live streaming in v1

The `--tail` flag requires either polling or inotify/kqueue. Out of scope for v1. The DB query is a point-in-time snapshot.

### D5 — Chronological interleaving, not grouped

Events from all surfaces are merged into a single chronologically-sorted stream. No trace_id grouping in v1 — strictly chronological as the ticket suggests for default behavior.

### D6 — Unified event enum for rendering

Define a `LogEvent` enum that wraps message/llm_call/tool_call rows with a common `timestamp()` method. Sort all events by timestamp, then render each via a formatter that produces the one-line format from the ticket.

## Implementation steps

### Step 1: Restructure `LogsArgs` into subcommand-bearing form

**File:** `crates/mika-cli/src/cli.rs`

Change `LogsArgs` from a flat struct to a subcommand-bearing struct (like `KgArgs`):

```rust
#[derive(clap::Args)]
pub struct LogsArgs {
    #[command(subcommand)]
    pub command: Option<LogsCommand>,

    // Kept for backward compat on bare `mika logs` (delegates to Paths)
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(Subcommand)]
pub enum LogsCommand {
    /// Show resolved log file paths
    Paths {
        #[command(flatten)]
        agent_flag: AgentFlag,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Query cross-surface activity (messages, LLM calls, tool calls)
    Activity(LogsActivityArgs),
}
```

Update `LogsArgs` agent_override to delegate through subcommand variants (same pattern as `KgArgs::agent_override`).

Update `Commands::Logs` match arm in `agent_override()` to call `args.agent_override()`.

### Step 2: Define `LogsActivityArgs`

**File:** `crates/mika-cli/src/cli.rs`

```rust
#[derive(clap::Args)]
pub struct LogsActivityArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Filter by session ID (prefix match supported)
    #[arg(long)]
    pub session: Option<String>,

    /// Filter by task ID
    #[arg(long)]
    pub task: Option<String>,

    /// Filter by trace ID
    #[arg(long)]
    pub trace: Option<String>,

    /// Time window start: "30m", "2h", "1d", "today", or ISO 8601 timestamp.
    /// Default: "1h"
    #[arg(long, default_value = "1h")]
    pub since: String,

    /// Time window end (same format as --since). Default: now.
    #[arg(long)]
    pub until: Option<String>,

    /// Filter content by regex pattern
    #[arg(long)]
    pub grep: Option<String>,

    /// Maximum number of events to display
    #[arg(long, short = 'n', default_value = "200")]
    pub limit: usize,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
```

### Step 3: Create `LogEvent` unified event type

**File:** `crates/mika-cli/src/commands/logs.rs` (extend existing file)

Define:

```rust
enum LogEvent {
    Message {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        role: String,
        content: String,  // truncated to ~120 chars for text mode
    },
    LlmCall {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        provider: String,
        model: String,
        status: String,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        latency_ms: Option<i64>,
        error_message: Option<String>,
    },
    ToolCall {
        timestamp: String,
        session_id: String,
        trace_id: Option<String>,
        tool_name: String,
        success: bool,
        latency_ms: Option<i64>,
        error_message: Option<String>,
    },
}
```

Implement `LogEvent::timestamp() -> &str` for sorting.

### Step 4: Implement duration parsing

**File:** `crates/mika-cli/src/commands/logs.rs`

Add `parse_since(input: &str) -> Result<String>` that:
- Handles `Nm` (minutes), `Nh` (hours), `Nd` (days) by subtracting from `Utc::now()`
- Handles `today` as midnight of the current day (UTC)
- Passes through anything that looks like an ISO 8601 timestamp
- Returns ISO 8601 string for use in SQL `>=` comparison

Similarly `parse_until(input: &str) -> Result<String>` (same logic, defaults to now if None).

### Step 5: Implement the activity query logic

**File:** `crates/mika-cli/src/commands/logs.rs`

Add `pub async fn run_activity(args: &LogsActivityArgs, agent_name: &str) -> Result<()>`:

1. Call `init::init_db_only_for_agent(agent_name)` to get `DbContext`.
2. Parse `--since` and `--until` into ISO 8601 bounds.
3. Build filters and query all three surfaces in parallel (they're independent):
   - `get_messages_since(agent_id, since)` — then apply `until`, `session`, `trace`, `grep` post-filters.
   - `query_llm_calls(LlmCallFilters { agent_id, from, to, session_id, trace_id }, page=1, per_page=limit)`.
   - `query_tool_calls(ToolCallFilters { agent_id, from, to, session_id, trace_id }, page=1, per_page=limit)`.
4. If `--task` is provided: look up the task to find its session_id, then use that as the session filter.
5. Convert all results to `Vec<LogEvent>`.
6. If `--grep` is provided: filter events whose content/error_message matches the regex.
7. Sort all events by timestamp.
8. Truncate to `--limit`.
9. Render based on `--format`.

### Step 6: Implement text formatter

**File:** `crates/mika-cli/src/commands/logs.rs`

For text mode, render each event as one line matching the ticket's proposed format:

```
[HH:MM:SS] msg.<role>     session=<8-char-prefix> <content-preview>
[HH:MM:SS] llm.call       session=<8-char-prefix> provider=<p> model=<m> status=<s>  in=<n> out=<n>
[HH:MM:SS] tool.call      session=<8-char-prefix> tool=<name> status=<success|error>  <latency>ms
```

Time is shown as local time (HH:MM:SS) extracted from the ISO 8601 timestamp. Session IDs are truncated to 8 chars for readability.

Print a header line: `Activity for agent: <name> (since <since> to <until>)` and a footer with total event count.

### Step 7: Implement JSON formatter

**File:** `crates/mika-cli/src/commands/logs.rs`

For JSON mode, serialize the full `Vec<LogEvent>` as a JSON array with `serde_json`. Each event includes all fields (no truncation). Wrap in an envelope:

```json
{
  "agent": "mika-dev",
  "since": "2026-06-14T12:00:00Z",
  "until": "2026-06-14T13:00:00Z",
  "total": 42,
  "events": [...]
}
```

### Step 8: Update dispatch in `main.rs`

**File:** `crates/mika-cli/src/main.rs`

Update the `Commands::Logs` match arm to:
- If `args.command` is `None` or `Some(LogsCommand::Paths { .. })`: call existing `commands::logs::run()` (the path-printing function).
- If `Some(LogsCommand::Activity(ref activity_args))`: call `commands::logs::run_activity(activity_args, &agent_name).await`.

The bare `mika logs` continues to work as before.

### Step 9: Add `--task` session resolution

**File:** `crates/mika-cli/src/commands/logs.rs`

When `--task <id>` is provided:
1. Query `sessions` table for sessions with `task_id = <id>`.
2. Use the found `session_id` as the session filter for all three surface queries.
3. If no sessions found for the task, print an error and exit.

This uses the existing `Session.task_id` column (added in schema v18).

### Step 10: Add `--session` prefix matching

**File:** `crates/mika-cli/src/commands/logs.rs`

When `--session <prefix>` is given:
- For messages: post-filter where `session_id LIKE '<prefix>%'`.
- For llm_calls/tool_calls: use `session_id` in the filter struct. Since `LlmCallFilters.session_id` expects an exact match, implement prefix matching as a post-filter on the query results, or use a direct SQL query with LIKE if the existing filter is too restrictive.

Pragmatic approach: if the prefix is >= 8 chars (UUID-like), do an exact match. For shorter prefixes, query without session filter and post-filter.

### Step 11: Tests

**File:** `crates/mika-cli/src/commands/logs.rs`

Add `#[cfg(test)] mod tests`:
- `test_parse_since_minutes` — "30m" produces a timestamp ~30 minutes ago.
- `test_parse_since_hours` — "2h" produces a timestamp ~2 hours ago.
- `test_parse_since_today` — "today" produces midnight UTC.
- `test_parse_since_iso_passthrough` — ISO 8601 string passes through unchanged.
- `test_log_event_sorting` — mixed events sort chronologically by timestamp.
- `test_text_format_one_line` — a single LogEvent renders as one line in the expected format.

### Step 12: Update CLI CLAUDE.md

**File:** `crates/mika-cli/CLAUDE.md`

Update the "Logs CLI" section to document both subcommands:
- `mika logs` / `mika logs paths` — show log file paths (existing).
- `mika logs activity` — query cross-surface activity with filter options.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-cli/src/cli.rs` | Add `LogsCommand` enum, `LogsActivityArgs` struct, update `LogsArgs` to subcommand-bearing, update `agent_override()` |
| `crates/mika-cli/src/commands/logs.rs` | Add `LogEvent` enum, `parse_since`/`parse_until`, `run_activity()`, text/JSON formatters, tests |
| `crates/mika-cli/src/main.rs` | Update `Commands::Logs` dispatch to route subcommands |
| `crates/mika-cli/CLAUDE.md` | Document new `mika logs activity` subcommand |

## Dependencies

No new crate dependencies. Uses existing:
- `chrono` (already in mika-cli) for timestamp parsing and local time display
- `serde_json` (already in mika-cli) for JSON output
- `regex` (already in mika-agent, needs adding to mika-cli Cargo.toml) for `--grep` pattern matching
- `mika_agent` (already a dependency) for `AsyncDatabase`, `LlmCallFilters`, `ToolCallFilters`, row types

**Dependency check needed:** Verify `regex` is available in mika-cli's Cargo.toml. If not, add it. Alternatively, use simple `str::contains` for v1 and defer regex to a follow-up.

## Risks and mitigations

- **Large result sets:** The `--limit` flag (default 200) caps output. Per-surface queries also use pagination (per_page = limit) so we don't load unbounded data.
- **Session prefix matching performance:** For short prefixes, post-filtering is fine given the --since time window limits the working set. No index needed.
- **Backward compatibility:** Bare `mika logs` continues to print paths via the `Option<LogsCommand>` defaulting to `None` → paths behavior.

## Out of scope (explicit)

- `--tail` / live streaming (requires polling infra)
- `--include server_log` (requires multi-GB file grep)
- `--include claude_pilot` (subprocess log correlation)
- Aggregation / analytics (use dashboard)
- Trace-grouped output mode (follow-up)
