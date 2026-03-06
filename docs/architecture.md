# Mika Architecture

## 1. Overview

Mika is a conversation-first AI executive assistant built in Rust. It operates in two
modes:

- **CLI mode (embedded):** The `mika` binary runs locally. User input flows through a
  ratatui TUI, the agent loop runs in-process, and SQLite stores all data on the local
  filesystem. No network services are required beyond the Claude API.

- **Hosted mode (per-customer containers):** Each customer gets their own
  agent container running `mika-server` (Axum HTTP), with an isolated SQLite database
  on a persistent volume. A shared gateway (`crates/mika-gateway/` in this repo)
  routes messages from Telegram to the correct container.

Both modes use the same agent loop, tools, memory model, and prompt assembly code
from the `mika-agent` crate.


## 2. Architecture Diagram

### CLI Mode

```
+-----------+     +----------------+     +-------------+
|  Terminal  |---->|  mika binary   |---->|  Claude API  |
|  (ratatui) |<----|  (agent loop)  |<----|  (Messages)  |
+-----------+     +-------+--------+     +-------------+
                          |
                          v
                    +----------+
                    |  SQLite   |
                    |  (local)  |
                    +----------+
```

### Hosted Mode

```
+----------+     +--------------------+     +----------------------------+
| Telegram  |---->|  Gateway           |     |  Per-customer container    |
| Bot API   |<----|  (mika-gateway)    |     |  mika-server (Axum)        |
+----------+     +--------+-----------+     |                            |
                           |                 |  SQLite (persistent vol)   |
                           |   POST /message |  Agent loop + tools        |
                           +---------------->|                            |
                           |                 +-------------+--------------+
                           |   POST /send                  |
                           |<------------------------------+
                           |                        +------+------+
                           |                        |  Claude API  |
                           |                        +-------------+
```


## 3. Crate Structure

| Crate | Path | Responsibility |
|-------|------|---------------|
| `mika-common` | `crates/mika-common/` | Shared library: config (config-rs with `MIKA_` prefix), Claude API client (`ClaudeClient` with typed `ClaudeApiError`), logging (tracing), telemetry (feature-gated OTel export), home directory resolution |
| `mika-agent` | `crates/mika-agent/` | Agent container: SQLite database (`Database`, `AsyncDatabase`), agent loop (`run_agent`, `run_silent_agent`), 23 builtin tools + 6 conditional management tools, prompt assembly, conversation compaction, unified task engine, HTTP server binary (`mika-server`) |
| `mika-cli` | `crates/mika-cli/` | TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (`status`, `memory`, `reminders`, `config`, `setup`, `tasks`) |
| `mika-gateway` | `crates/mika-gateway/` | Telegram webhook router: Postgres customer registry, message routing to per-customer containers, pairing flow, outbound relay to Telegram. Stateless, env-var-only config. |


## 4. Agent Loop

The agent loop is an explicit Rust async function -- no framework. It executes the
following steps for each inbound user message:

Source: `crates/mika-agent/src/agent.rs` -- `run_agent()` / `run_agent_inner()`

1. **Save user message** to the `conversations` table via `AsyncDatabase::save_message()`.

2. **Load context** for system prompt assembly:
   - `soul.md` (agent personality, read from `home_dir`)
   - Identity configuration (`identity.toml` or inline defaults)
   - All core memory blocks (`get_all_core_memory()`)
   - Customer timezone (`get_customer_config("timezone")`)
   - Existing conversation summary, if any (`load_conversation_summary()`)

3. **Match skills** against the user message via `SkillRegistry::match_message()`.
   For matched skills, lazy-load prompt snippets and inject them into the system
   prompt. Resolve the final set of tool definitions (builtin + skill-provided).
   If no skills directory exists, fall back to all builtin tools.

4. **Load recent messages** -- the last 20 conversation messages
   (`load_recent_messages(20, None)`).

5. **Send request to Claude API** with system prompt, message history, and tool
   definitions.

6. **Match `stop_reason`** from the Claude response:
   - `EndTurn` or `MaxTokens` -- save assistant text to DB, return response.
   - `StopSequence` -- save assistant text to DB, return response.
   - `ToolUse` -- execute each tool call with per-tool timeout, push assistant
     message and tool results onto the request, strip images from prior turns
     to prevent memory accumulation, loop back to step 5.

   **Step-awareness nudge:** At step 8 of 10 (conversation mode only), a nudge
   is appended to the system prompt telling the model to prioritize completing
   or summarizing its work.

   **Max-steps exceeded:** If the loop exhausts all 10 steps without producing
   text, a continuation turn is attempted: tools are disabled, thinking is
   disabled, and one final API call (60s timeout) forces the model to produce
   a text summary of what it accomplished. If the continuation fails (API error,
   timeout, empty response), a structured fallback shows the last 5 tool names
   with status and invites the user to ask for continuation.

   **Multi-modal tool results:** Tools can return images alongside text via
   `ToolOutput::success_with_images()`. When images are present, the tool result
   is sent as a multi-block content array (`[{type: "text"}, {type: "image"}]`)
   matching the Claude API spec. Prior-turn images are replaced with
   `[image(s) from previous turn omitted]` text before each API call.

7. **Post-turn compaction** -- after the agent returns, check if conversation
   compaction is needed (`compaction::maybe_compact()`). In CLI mode this runs
   inline. In server mode (`skip_compaction: true`), compaction is spawned
   outside the agent lock.

### Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_TOOL_STEPS` | 10 | Maximum tool-use iterations per agent turn |
| `TOOL_TIMEOUT_SECS` | 30 | Default per-tool execution timeout (overridable via `Tool::timeout_secs()`) |
| `AGENT_TOTAL_TIMEOUT_SECS` | 300 | Total agent loop timeout (5 minutes) |
| `CONTINUATION_TIMEOUT_SECS` | 60 | Continuation turn timeout after max steps |


## 5. Memory Model

Mika uses a three-layer memory hierarchy. Each customer has their own isolated SQLite
database.

### Layer 1: Core Memory

Always present in the system prompt. The agent can edit these blocks via the
`update_core_memory` tool.

| Block | Default Value |
|-------|--------------|
| `user_summary` | "No information about the user yet." |
| `self_model` | "I am {agent_id}. No interaction history yet." |
| `current_priorities` | "No priorities set yet." |
| `key_people` | "No people tracked yet." |

**Constraints:**
- Per-block limit: `MAX_TOKENS_PER_BLOCK = 500` (~2000 characters at 4 chars/token)
- Per-session edit limit: `MAX_CORE_MEMORY_EDITS_PER_SESSION = 3` (onboarding sessions exempt)
- Actions: `replace`, `append`, `remove_line`, `reset`
- All mutations are recorded in the `memory_events` audit table

### Layer 2: Structured Facts

Stored in dedicated SQLite tables. Managed by the agent via `store_fact`,
`update_fact`, and `search_memory` tools.

| Category | Table | Key Columns |
|----------|-------|-------------|
| People | `people` | `canonical_name` (UNIQUE COLLATE NOCASE), `relationship`, `notes` |
| Commitments | `commitments` | `description` (UNIQUE COLLATE NOCASE), `status` (pending/completed/cancelled), `due_date`, `person_id` FK |
| Preferences | `preferences` | `category` (UNIQUE COLLATE NOCASE), `value` |
| Events | `events` | `description`, `event_date`, `context` |

### Layer 3: Hybrid Search

FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion.
Optional OpenAI embeddings (`text-embedding-3-small`, 512 dims). Graceful
degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on `store_fact`/
`update_fact`, backfilled on startup.

See [ADR-003](adr/003-layer3-hybrid-vector-search.md) for implementation details.


## 6. Tools

### Builtin Tools

All 23 builtin tools, registered in `crates/mika-agent/src/tools/mod.rs` via
`default_tools()`:

| Tool | Description | Category |
|------|-------------|----------|
| `update_core_memory` | Update persistent core memory blocks (Layer 1). Actions: replace, append, remove_line, reset. Rate limited to 3 edits/session. | Memory |
| `store_fact` | Store a new structured fact (person, commitment, preference, or event) into Layer 2 tables. | Memory |
| `search_memory` | Search across all Layer 2 categories (people, commitments, preferences, events). | Memory |
| `update_fact` | Update an existing Layer 2 fact (e.g., change commitment status, update person notes). | Memory |
| `create_reminder` | Schedule a future reminder with ISO 8601 `fire_at` timestamp and message text. Outputs full UUID. | Reminders |
| `list_reminders` | List pending and future reminders. Outputs full UUIDs for use with `cancel_reminder`. | Reminders |
| `cancel_reminder` | Cancel a pending reminder by full UUID. Delegates to `CancelTaskTool` (alias for backwards compatibility). | Reminders |
| `list_tasks` | List scheduled tasks with optional status filter. Shows full UUID, trigger_type, action_type, status, timeout_at. | Tasks |
| `create_task` | Create a scheduled task (time, recurring, or callback trigger; any action type). Returns full UUID. Validates trigger_type and action_type against `trigger_type::*` / `action_type::*` constants. timeout_secs capped at 90 days. | Tasks |
| `cancel_task` | Cancel a pending task by full UUID (36-char validation). | Tasks |
| `complete_task` | Mark an agent's own callback task complete with a result string. Validates trigger_type=callback and ownership via agent_id. | Tasks |
| `get_task` | Inspect a task by full UUID. Returns all fields including status, trigger_type, action_type, result, timeout_at. | Tasks |
| `send_message` | Send a message to the user out-of-band. In CLI mode, prints to stdout. In server mode, POSTs to the routing URL. Required for silent mode (heartbeat/reminders). | Messaging |
| `create_skill` | Create a new custom skill with prompt snippets and tool definitions. | Skills |
| `delete_skill` | Delete a custom or marketplace skill. Built-in skills cannot be deleted. | Skills |
| `list_skills` | List all skills with their origin, status, and keywords. | Skills |
| `toggle_skill` | Enable or disable a skill. | Skills |
| `update_skill` | Update an existing skill's description, keywords, prompts, or always_on setting. | Skills |
| `get_config` | Read customer config values (timezone, chat_id, thinking_level). | Config |
| `set_config` | Update customer config values. | Config |
| `write_file` | Write content to a file in the agent's home directory. Requires `confirm: true` to overwrite existing files — returns current content first. | Files |
| `read_home_file` | Read a file from the agent's home directory. Uses `validate_and_resolve_path` with `create_parents: false`. Reports resolved absolute path. | Files |
| `list_home_files` | List files in a directory within the agent's home directory. Includes sizes and modification ages. Uses `spawn_blocking` for I/O. | Files |

### Management Tools

6 tools for multi-agent and team workflows, registered conditionally via
`management_tools_if_needed()` only when `agents.len() > 1 || !teams.is_empty()`:

| Tool | Description | Timeout |
|------|-------------|---------|
| `list_agents` | List all configured agents with their identities and role hints. | default (30s) |
| `delegate_task` | Delegate a task to another agent and get their response. Runs with `default_tools()` only (no management tools, no MCP) to prevent recursion. | 120s |
| `list_teams` | List all configured teams with agent counts. | default (30s) |
| `run_team` | Run a team workflow with a specified goal. Team agents collaborate to decompose, execute, review, and deliver results. | 300s |
| `get_team_status` | Get the status of a team's most recent run, or a specific run by ID. | default (30s) |
| `get_team_history` | List recent runs for a team with IDs, status, goals, and timestamps. | default (30s) |

Management tools are NOT registered for team sub-agents or delegated agents,
preventing infinite delegation chains.

**Tool trait:** `#[async_trait]` with `Send + Sync` bounds (required for `tokio::spawn`
in server handlers). Each tool validates inputs: empty string check + 10,000 character
maximum (`MAX_INPUT_LEN`). Per-tool timeout override via `timeout_secs()` default method
(returns `None` to use the 30s default).


## 7. Conversation Compaction

When conversation history grows beyond a threshold, older messages are summarized via
a Claude API call and replaced with a summary row. The summary is injected into the
system prompt (not into message history).

### Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `COMPACTION_THRESHOLD` | 50 | Minimum message count before compaction triggers |
| `CONTEXT_WINDOW` | 20 | Number of recent messages to keep (not compacted) |
| `MAX_COMPACTION_BATCH` | 100 | Maximum messages per summarization call |
| `MAX_SUMMARY_CHARS` | 4000 | Truncation limit for generated summaries |
| `MAX_COMPACTION_INPUT_CHARS` | 50,000 | Character budget for messages sent to summarizer |

### Flow

1. After each agent turn, `maybe_compact()` checks `count_messages()`.
2. If total messages <= 50, skip.
3. Load all messages outside the context window.
4. Cap the batch at 100 messages.
5. Call Claude API with a summarization system prompt and the message batch.
6. Truncate the generated summary to 4000 characters if needed.
7. Delete old messages up to `compacted_through_id`, insert or update the summary row.
8. Recent 20 messages remain untouched.

Compaction is incremental -- subsequent rounds merge the existing summary with
newly compacted messages.


## 8. AsyncDatabase

Source: `crates/mika-agent/src/async_db.rs`

`AsyncDatabase` wraps the synchronous `Database` (rusqlite) with a dedicated OS
thread and an `mpsc` channel, making it Send+Sync and compatible with `tokio::spawn`.

```
Caller (any tokio task)                  Dedicated OS thread ("mika-db")
        |                                        |
        |-- mpsc::send(closure) ----------------->|
        |                                        |-- closure(&Database)
        |<-- oneshot::send(Result<T>) ------------|
```

Properties:
- **Clone-able:** Wraps `Arc<AsyncDatabaseInner>` — clones share the same connection.
- **Panic-resilient:** Each closure wrapped in `catch_unwind()`.
- **Graceful shutdown:** `shutdown()` drops the sender, joins the background thread.


## 9. Unified Task Engine

The task engine is a single SQLite-backed scheduler that handles all proactive
behaviors (heartbeat, reflection, reminders) via a unified `tasks` table.

Source: `crates/mika-agent/src/task_engine/`

### Components

| Module | Responsibility |
|--------|---------------|
| `engine.rs` | `TaskEngine` — min-heap `BinaryHeap<QueuedTask>` + 1-second tick loop |
| `dispatcher.rs` | `TaskDispatcher` — matches `action_type` and executes tasks |
| `queue.rs` | `QueuedTask` — heap entry with `next_fire_at` for ordering |
| `types.rs` | Constants: `task_status::*` and `action_type::*` (no magic strings) |
| `mod.rs` | `ensure_recurring_task()` — idempotently registers built-in tasks at startup |

### Action Types

| Action | Description |
|--------|-------------|
| `send_message` | Deliver a message to the user (capped at 50,000 chars) |
| `run_skill` | Invoke a named skill by `skill_name`; dispatches via `SilentTrigger::SkillRun` for correct framing |
| `inject_context` | Inject a context block into the next agent turn (stays `in_progress`) |
| `resume_agent` | Re-invoke the agent with a callback result injected as `SilentTrigger::Callback` |
| `invoke_orchestrator` | Re-invoke the team orchestrator when all sibling tasks complete |

### Recurring Tasks (registered at startup)

| Label | Cron | Purpose |
|-------|------|---------|
| `heartbeat` | `0 0 * * * *` | Hourly proactive check-in |
| `reflection` | `0 0 2 * * *` | Daily memory reflection at 02:00 |

### Tick Loop

- 1-second tick: fires tasks where `next_fire_at <= now` via `claim_and_fire_task` (single atomic UPDATE)
- Every 60 ticks: DB scan picks up tasks created by tools during agent runs
- `startup_recovery()` on init: expires timed-out tasks, marks orphaned `in_progress` tasks as failed, loads heap
- CLI aborts the tick loop `JoinHandle` on agent switch
- Server stores `tick_handles: Vec<JoinHandle<()>>` and aborts them on graceful shutdown

### Callback/Resume Lifecycle

Agent-created callback tasks follow this end-to-end pattern:

1. Agent calls `create_task` with `trigger_type="callback"` and `action_type="resume_agent"`. Receives full UUID.
2. Agent or background script does long-running work.
3. External process completes the task via one of:
   - **CLI:** `mika ask --agent <name> --task-id <uuid> "<result>"` — agent runs first with result injected, then task marked complete
   - **HTTP:** `POST /tasks/{id}/complete` with Bearer auth and `{"result": "..."}` body
4. `update_task_completed` validates status (`AND status IN ('pending','in_progress')`) before writing — returns `false` if already completed (TOCTOU guard).
5. `TaskDispatcher::dispatch_resume_agent` fires via `SilentTrigger::Callback { label, result }`. The result is wrapped in `<callback_result trust="untrusted">` delimiters before LLM injection to mitigate prompt injection.

### SilentTrigger Variants

`SilentTrigger` controls the system-prompt framing for background agent runs:

| Variant | Used by | Prompt framing |
|---------|---------|---------------|
| `Heartbeat` | Hourly heartbeat recurring task | Scheduled check-in, review commitments |
| `Reflection` | Daily reflection recurring task | Memory reflection and consolidation |
| `Callback { label, result }` | `resume_agent` dispatcher | Background task completed, inject result |
| `SkillRun { skill_name }` | `run_skill` dispatcher | Run the named skill |

### Startup Maintenance

`prune_old_tasks()` runs at startup and deletes completed, failed, and cancelled tasks older than 30 days to prevent unbounded table growth.

### Composite Partial Index

```sql
CREATE INDEX idx_tasks_schedulable ON tasks(agent_id, next_fire_at ASC)
WHERE status IN ('pending', 'recurring_active');
```


## 10. Silent Mode

Silent mode is used for background tasks (heartbeat check-ins and reminders) where
the agent's text output is NOT automatically delivered to the user.

| Aspect | Normal Mode | Silent Mode |
|--------|-------------|-------------|
| User message | Actual user input | Synthetic trigger |
| Text output | Returned to caller | NOT delivered (saved to DB for audit) |
| How to reach user | Automatic | Must use `send_message` tool |
| Message history | Last 20 messages | Single trigger message only |
| Compaction | Runs post-turn | Does not run |

Heartbeat mode uses `safe_always_on_skills()` which filters out exec/http-handler
skills for security — only builtin-handler skills are available in autonomous
background runs.

Each background run is framed by a `SilentTrigger` variant (see Section 9 §
SilentTrigger Variants). Callback results are wrapped in
`<callback_result trust="untrusted">` XML-like delimiters before LLM injection
to mitigate prompt injection from external result payloads.


## 11. HTTP Server (mika-server)

The per-customer agent container runs an Axum HTTP server:

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/health` | GET | None | Liveness/readiness probe |
| `/message` | POST | Bearer | Receives messages (202 async processing, 10MB body limit) |
| `/tasks/{id}/complete` | POST | Bearer | Completes a callback task (200 sync; 409 if already completed; 100KB result cap; echoes `task_id` in error bodies) |

`AppState` is Clone via Arc-wrapped dependencies. Agent lock
(`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock`
(429 if busy).

See [ADR-001](adr/001-axum-http-server-architecture.md) for design decisions.


## 12. Failed Sends (Durable Outbox Pattern)

When the outbound routing endpoint is unreachable, messages are not lost.

### Write Path

`GatewayMessageSender::send()` implements retry-then-persist:
1. First attempt: POST to routing URL with 10s timeout.
2. On failure: Wait 2 seconds, retry once.
3. On second failure: Save to `failed_sends` SQLite table. Return `Ok(())` so
   the agent loop does not see a tool error.

### Read Path (Flush)

At the start of each `/message` handler, the server flushes up to 5 pending failed
sends in a background task (does not block message processing).


## 13. Multi-Agent Support

- Global home directory: `~/.mika/`
- Agent homes: `~/.mika/agents/{name}/` (each with skills/, logs/)
- Active agent tracked in `~/.mika/active_agent`
- CLI `--agent` flag overrides active agent
- CLI `--team` flag launches TUI in team mode (mutually exclusive with `--agent`)
- Server discovers all agents on startup
- `AgentParams` carries `global_home_dir: Option<&Path>` (global `~/.mika/`) distinct from
  per-agent `home_dir` (e.g. `~/.mika/agents/main/`) for agent/team discovery

### Agent Delegation

When multiple agents are configured, the primary agent can delegate tasks to other
agents via the `delegate_task` tool. The delegate runs with its own personality,
memory, and skills but receives only `default_tools()` (no management tools, no MCP)
to prevent infinite delegation chains. The system prompt includes an "Agents & Teams"
section listing available agents with their identities (emoji + name).

### Team Workflows

Teams are defined in `~/.mika/teams/{name}/team.toml` and orchestrated by the
`run_team` tool. Team runs and message graphs are persisted to the shared container
database (`~/.mika/data/mika.db`) with graph-structured messages linked via
`parent_id`. Queryable via `get_team_status` and `get_team_history` tools.

See [ADR-004](adr/004-multi-agent-teams-orchestration.md) for team orchestration.


## Appendix: Database Schema

**Schema version:** 1 (consolidated clean-slate baseline)

### Tables

| Table | Purpose |
|-------|---------|
| `schema_version` | Migration tracking |
| `agents` | Agent registry (name, display_name, model, active flag) |
| `teams` | Team registry (name, display_name, orchestrator) |
| `conversations` | Message history (user, assistant, summary rows; `channel_type`, `agent_id` FK) |
| `core_memory` | Layer 1 persistent memory blocks (`agent_id` FK) |
| `people` | Layer 2 people/contacts (`agent_id` FK) |
| `commitments` | Layer 2 tasks/promises with status tracking (`agent_id` FK) |
| `preferences` | Layer 2 user preferences (`agent_id` FK) |
| `events` | Layer 2 notable events (`agent_id` FK) |
| `memory_events` | Audit log for all memory mutations (`agent_id` FK) |
| `customer_config` | Key-value store (timezone, chat_id) |
| `failed_sends` | Durable outbox for failed outbound messages |
| `memory_event_summaries` | Tiered retention summaries (monthly) |
| `skills` | Skill metadata (name, description, builtin flag, enabled) |
| `skill_tools` | Tool definitions per skill |
| `search_content` | Unified search content for Layer 3 hybrid search |
| `fts_search` | FTS5 virtual table for full-text search |
| `vec_search` | sqlite-vec virtual table (vec0) for vector similarity |
| `tasks` | Unified task scheduler — replaces `reminders`; all proactive behaviors (`agent_id`, `action_type`, `status`, `cron_expression`, `next_fire_at`, `fired_at`, `completed_at`) |
| `team_runs` | Team run metadata (goal, status, iterations, deliverable) |
| `team_messages` | Graph-structured team messages with `parent_id` links; `agent_name` column (not FK) |

### SQLite Pragmas

The database is initialized with WAL journal mode, NORMAL synchronous level, foreign
keys enabled, a 5-second busy timeout, and incremental auto-vacuum.
