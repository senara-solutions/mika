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
| `mika-common` | `crates/mika-common/` | Shared library: config (config-rs with `MIKA_` prefix, `ConfigKeyInfo` registry with `ConfigBackend` enum for key metadata), validation (`validation.rs` — API key format, file permissions, binary-in-PATH, config value validation), dotenv (`~/.mika/.env` secrets via dotenvy), Claude API client (`ClaudeClient` with typed `ClaudeApiError`), logging (tracing), telemetry (feature-gated OTel export), home directory resolution |
| `mika-agent` | `crates/mika-agent/` | Agent container: SQLite database (`Database`, `AsyncDatabase`), agent loop (`run_agent`, `run_silent_agent`), 26 builtin tools + 10 management tools (3 always-on + 7 conditional), prompt assembly, conversation compaction, conversation rewind engine, unified task engine, skills system, MCP client, HTTP server binary (`mika-server`) |
| `mika-cli` | `crates/mika-cli/` | TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (`status`, `memory`, `reminders`, `config`, `setup`, `tasks`, `doctor`) |
| `mika-gateway` | `crates/mika-gateway/` | Telegram webhook router: Postgres customer registry, message routing to per-customer containers, pairing flow, outbound relay to Telegram. Stateless, env-var-only config. |


## 4. Agent Loop

The agent loop is an explicit Rust async function -- no framework. It executes the
following steps for each inbound user message:

Source: `crates/mika-agent/src/agent.rs` -- `run_agent()` / `run_agent_inner()`

1. **Generate trace_id** via `trace::generate_trace_id()` (OTel extraction or UUID v4 hex fallback). **Save user message** to the `messages` table via `AsyncDatabase::save_message()` with trace_id.

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
| `workflows` | "Delegate-then-forget is not allowed. Any work sent to Claude Code must have a corresponding work item created first (via create_work_item). No exceptions." |

**Constraints:**
- Per-block limit: `MAX_TOKENS_PER_BLOCK = 500` (~2000 characters at 4 chars/token)
- Per-session edit limit: `MAX_CORE_MEMORY_EDITS_PER_SESSION = 3` (onboarding sessions exempt)
- Actions: `replace`, `append`, `remove_line`, `reset`
- All mutations are recorded in the `audit_events` table (with `trace_id` for correlation)

### Layer 2: Structured Facts

Stored in dedicated SQLite tables. Managed by the agent via `store_fact`,
`update_fact`, and `search_memory` tools.

| Category | Table | Key Columns |
|----------|-------|-------------|
| People | `people` | `canonical_name` (UNIQUE COLLATE NOCASE), `relationship`, `notes`, `mention_count` |
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
| `create_reminder` | Schedule a one-shot reminder (`fire_at` ISO 8601 UTC) or periodic reminder (`cron_expr` 6-field cron with seconds first, e.g. `0 0 9 * * 1`). Minimum interval: 1 minute. Outputs full UUID. | Reminders |
| `list_reminders` | List pending and future reminders. Shows cron expression for periodic reminders. Outputs full UUIDs for use with `cancel_reminder`. | Reminders |
| `cancel_reminder` | Cancel a pending reminder by full UUID. Delegates to `CancelTaskTool` (alias for backwards compatibility). | Reminders |
| `list_tasks` | List scheduled tasks with optional status filter. Shows full UUID, trigger_type, action_type, status, timeout_at. | Tasks |
| `create_task` | Create a scheduled task (time, recurring, or callback trigger; any action type). Returns full UUID. Validates trigger_type and action_type against constants. timeout_secs capped at 90 days. | Tasks |
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
| `write_agent_file` | Write content to a file in the agent's home directory. Requires `confirm: true` to overwrite existing files — returns current content first. | Files |
| `read_agent_file` | Read a file from the agent's home directory. Uses `validate_and_resolve_path` with `create_parents: false`. Reports resolved absolute path. | Files |
| `list_agent_files` | List files in a directory within the agent's home directory. Includes sizes and modification ages. Uses `spawn_blocking` for I/O. | Files |
| `query_timeline` | Query the unified timeline of events across all subsystems (messages, audit events, tasks). Returns recent activity sorted by time. Non-orchestrator agents scoped to own agent_id. | Introspection |
| `get_session_messages` | Retrieve messages from a past conversation session. Useful for replaying or summarizing old conversations. Non-orchestrator agents can only access their own sessions. | Introspection |
| `list_audit_events` | List recent memory mutation audit events (fact stores, updates, core memory edits). Useful for self-introspection. Non-orchestrator agents scoped to own events. | Introspection |

### Management Tools

10 tools for multi-agent and team workflows, registered via
`management_tools_if_needed()`. `create_agent`, `list_agents`, and `create_team` are always
registered (enabling agent and team bootstrapping from a single-agent setup). The
remaining 7 tools are added conditionally when `agents.len() > 1 || !teams.is_empty()`:

| Tool | Description | Timeout | Always registered |
|------|-------------|---------|-------------------|
| `create_agent` | Create a new agent with name, display name, soul (personality), and optional model override. | default (30s) | Yes |
| `list_agents` | List all configured agents with their identities and role hints. | default (30s) | Yes |
| `create_team` | Create a new team definition with specified agents and flow. All referenced agents must exist. | default (30s) | Yes |
| `delegate_task` | Delegate a task to another agent and get their response. Requires `work_item_id` (must create a work item first). Runs with `default_tools()` only (no management tools, no MCP) to prevent recursion. | 120s | No |
| `list_teams` | List all configured teams with full configuration (roles, mandates, max_iterations). | default (30s) | No |
| `run_team` | Run a team workflow with a specified goal. Team agents collaborate to decompose, execute, review, and deliver results. | 300s | No |
| `get_team_status` | Get the status of a team's most recent run, or a specific run by ID. | default (30s) | No |
| `get_team_history` | List recent runs for a team with IDs, status, goals, and timestamps. | default (30s) | No |
| `delete_team` | Delete a team definition and all its data (workspace, config). Irreversible. | default (30s) | No |
| `update_team` | Update an existing team definition. Only provided fields are changed. | default (30s) | No |

Management tools are NOT registered for team sub-agents or delegated agents,
preventing infinite delegation chains.

**Tool trait:** `#[async_trait]` with `Send + Sync` bounds (required for `tokio::spawn`
in server handlers). Each tool validates inputs: empty string check + 10,000 character
maximum (`MAX_INPUT_LEN`). Per-tool timeout override via `timeout_secs()` default method
(returns `None` to use the 30s default).


## 7. Skills System

Source: `crates/mika-agent/src/skills/`

Skills extend Mika's capabilities with prompt snippets, tool definitions, and handler
scripts. Each skill lives in its own directory under `{agent_home}/skills/{name}/`.

### Skill Directory Structure

```
{agent_home}/skills/{skill_name}/
  skill.toml              # Manifest (required, max 64KB)
  tools.json              # Tool definitions for exec/http handlers (optional, max 256KB)
  system_prompt.md        # Prompt snippet injected on match (optional, max 8KB)
  .disabled               # Marker file to disable skill (presence = disabled)
  handlers/
    run.sh                # Exec handler scripts (executable)
```

### Manifest (`skill.toml`)

```toml
[skill]
name = "web-search"
description = "Search the web for current information"
version = "0.1.0"
always_on = false
timeout_secs = 60

[triggers]
keywords = ["search", "look up", "find online"]
```

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `skill.name` | yes | — | Unique skill identifier |
| `skill.description` | yes | — | Human-readable description |
| `skill.version` | no | `""` | Semantic version |
| `skill.always_on` | no | `false` | Active every turn regardless of keywords. For built-in skills, user overrides stored in `skill_overrides` DB table (v7); for custom/marketplace, stored in `skill.toml`. |
| `skill.timeout_secs` | no | `30` | Per-tool execution timeout (seconds) |
| `triggers.keywords` | no | `[]` | Case-insensitive substring-matched keywords |

### Handler Types

Defined in `tools.json` via `#[serde(tag = "type")]`:

**Builtin** — Dispatches to compiled Rust functions in `ToolRegistry` (e.g.,
`get_documentation`, `web_search`). Full `ToolContext` access.

**Exec** — Spawns a subprocess, passes tool input as JSON via stdin, captures stdout.
MIKA_* environment variables are scrubbed from child processes. Supports the
`__mika_v1` image protocol (see below) and long-running mode.

**HTTP** — POST/GET/PUT to a URL with tool input as JSON body (or query params for GET).
Optional static headers.

### Exec Handler Image Protocol (`__mika_v1`)

Exec handler scripts can return images by outputting a JSON envelope:

```json
{"__mika_v1": {"text": "Screenshot captured.", "images": ["/tmp/screenshot.png"]}}
```

The executor detects the sentinel key via prefix check, reads and validates image files
(canonicalize, regular file check, 5MB limit, magic-byte validation for JPEG/PNG/GIF/WebP),
base64-encodes them, and returns them as `ImageData` on `ToolOutput`. Max 5 images per
result. If detection fails, stdout is treated as plain text (backward compatible).

### Long-Running Exec Handlers

When `long_running: true` is set on an exec handler (conversation mode only, not
silent/team), the executor creates a callback task and returns immediately instead
of waiting for the subprocess to complete.

**Mechanism:**
1. Executor creates a callback task (`trigger_type=callback`, `action_type=resume_agent`)
   with label `long_running:{tool_name}`.
2. `__mika_task_id` (UUID) and `__mika_agent` (agent name) are injected into the
   tool input JSON passed to the subprocess via stdin.
3. Subprocess spawned with `kill_on_drop(false)` and `stdout(Stdio::null())`.
4. PID recorded to database via `set_task_process_id()`.
5. Tool returns immediately with "task created" message.

**Timeout:** `(estimated_duration_secs.unwrap_or(3600) * 3).clamp(600, 7_776_000)` —
minimum 10 minutes, maximum 90 days.

**Completion:** The subprocess calls `mika ask --task-id <uuid>` (CLI) or the external
system POSTs to `/tasks/{id}/complete` (server) to deliver results back. On success,
the task engine fires `dispatch_resume_agent` with `SilentTrigger::Callback`.

**Failure:** A background monitor (`spawn_long_running_exec`) awaits the child process.
On non-zero exit, stderr is captured (capped) and the task is marked failed. Expired
tasks get SIGTERM via `kill_orphan_processes()` in the tick loop.

### Skill Matching

`match_skills()` in `matcher.rs`:

1. **Always-on skills** included unconditionally (if enabled).
2. **Keyword-matched skills** included if any keyword is a case-insensitive substring
   of the user message. Keywords are pre-lowercased at scan time.
3. **Disabled skills** (`.disabled` marker file) excluded entirely.

Silent mode uses `safe_always_on_skills()` which filters out exec/http-handler skills
for security — only builtin-handler skills are available in autonomous background runs.

### Three-Tier Origin

| Origin | Description |
|--------|-------------|
| `[built-in]` | Bundled with Mika binary, re-seeded on startup |
| `[marketplace]` | Installed from Git repos via `mika skills install`, tracked in `marketplace.lock` |
| `[custom]` | Created locally via `create_skill` tool or manually |

### Marketplace

Git-based skill distribution. See [ADR-006](adr/006-git-based-skills-marketplace.md).

- **Install:** `mika skills install user/repo` (GitHub shorthand) or full Git URL.
  Shallow clones to temp dir, scans for `skill.toml` (depth <= 2), copies skill dir
  (excluding `.git/`, symlink escape checks). Multi-skill repos show interactive picker.
- **Update:** `mika skills update [name]` — re-clones, compares HEAD commit to pinned
  commit, replaces if changed.
- **Uninstall:** `mika skills uninstall <name>` — removes skill dir and lock entry.

Lock file (`marketplace.lock` at agent home root) tracks URL, path, commit hash, and
timestamps per installed skill.


## 8. MCP Client (Model Context Protocol)

Source: `crates/mika-agent/src/mcp/`

Mika connects to external MCP servers at startup via `McpManager`, using the `rmcp`
crate (v0.17) for both stdio and Streamable HTTP transports.

### Configuration

MCP servers are configured in `{agent_home}/mcp.json` (Claude Desktop convention,
written with `0600` permissions to protect secrets):

```json
{
  "mcpServers": {
    "filesystem": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
      "env": {},
      "enabled": true
    },
    "remote-api": {
      "transport": "http",
      "url": "https://mcp.example.com/v1",
      "headers": {
        "Authorization": "Bearer sk-test-123"
      },
      "enabled": true
    }
  }
}
```

Server names must be lowercase alphanumeric with single hyphens/underscores (no `__`,
which is reserved for tool namespacing).

### Transports

**Stdio:** Spawns a child process via `tokio::process::Command`. Environment isolation:
`env_clear()` + allowlist (`PATH`, `HOME`, `USER`, `LANG`, `TERM`, `TMPDIR`,
`XDG_RUNTIME_DIR`) + server-specific env from config. `MIKA_*` overrides are rejected
with a warning. `kill_on_drop(true)` ensures cleanup.

**Streamable HTTP:** Connects to a URL via `StreamableHttpClientTransport` from rmcp.
`Authorization` header routed through rmcp's `auth_header()` (case-insensitive match);
other headers via `custom_headers()`.

Both transports use a 30-second handshake timeout. Connections happen in parallel via
`JoinSet` for fast startup. Failures are logged as warnings and never block startup.

### Tool Namespacing

MCP tools are namespaced as `mcp__{server}__{tool}` to prevent collisions with
builtin tools and skills. Example: `mcp__filesystem__read_file`.

Dispatch chain: builtins → skills → MCP → unknown error.

### Security

- **Environment isolation:** Child processes cannot access `MIKA_ANTHROPIC_API_KEY`,
  `MIKA_INTERNAL_TOKEN`, or any other Mika secrets.
- **Header redaction:** Custom `Debug` impl on `McpServerConfig` redacts both header
  and env variable values in logs.
- **Result limits:** 5 images max per result, 5MB per image, 10,000 chars text.

### Availability

| Context | MCP Available |
|---------|--------------|
| CLI ask mode (`mika ask`) | Yes (per-invocation connections, graceful shutdown) |
| CLI chat mode (`mika`) | Yes (session-persistent connections) |
| Server mode (`mika-server`) | Yes (per-agent manager, startup connections) |
| Silent mode (heartbeat, reflection, callbacks) | No |
| Team agent runs | No (Phase 4 future) |

CLI commands: `mika mcp add/remove/list/enable/disable`, `--header KEY=VALUE` for
HTTP headers.


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
| `types.rs` | Constants: `task_status::*`, `action_type::*`, `trigger_type::*` (no magic strings) |
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
   - **CLI:** `mika ask --agent <name> --task-id <uuid> "<result>"` — marks task complete and exits (no silent agent run). TUI handles delivery.
   - **HTTP:** `POST /tasks/{id}/complete` with Bearer auth and `{"result": "..."}` body
4. `update_task_completed` validates status (`AND status IN ('pending','in_progress')`) before writing — returns `false` if already completed (TOCTOU guard).
5. **Delivery** depends on the path:
   - **Server path:** `TaskDispatcher::dispatch_resume_agent` fires via `SilentTrigger::Callback { label, result }` → marks task `delivered` on success.
   - **CLI/TUI path:** TUI polls `get_undelivered_callback_tasks()` every ~5s when idle → atomically claims via `mark_task_delivered()` → injects result into conversation as `role='tool_result'` → runs agent with `is_callback_turn: true` (blocks long-running task creation). On agent failure, unclaims task for retry.

The result is wrapped in `<callback_result trust="untrusted">` delimiters via `format_callback_framing()` before LLM injection to mitigate prompt injection. Callback turns cannot spawn new long-running tasks (defense in depth: code guard via `LongRunningContext=None` + prompt guard via `callback_context` in `PromptContext`). Task lifecycle: `pending → completed → delivered`.

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

### Key Indexes

```sql
-- Scheduling efficiency
CREATE INDEX idx_tasks_schedulable ON tasks(agent_id, next_fire_at ASC)
WHERE status IN ('pending', 'recurring_active');

-- TUI callback polling efficiency
CREATE INDEX idx_tasks_callback_delivery ON tasks(agent_id, completed_at)
WHERE trigger_type = 'callback' AND action_type = 'resume_agent' AND status = 'completed';

-- Trace correlation (partial indexes — only non-NULL trace_ids)
CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;
CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
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


## 11. Conversation Compaction

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


## 12. AsyncDatabase

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


## 13. HTTP Server (mika-server)

The per-customer agent container runs an Axum HTTP server:

### Auth Split

The server has two auth layers:

- **Mutation endpoints** (`/message`, `/tasks/{id}/complete`) require `MIKA_INTERNAL_TOKEN` only (gateway-to-agent traffic).
- **Dashboard API** (`/api/v1/*`) accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser). If `MIKA_DASHBOARD_TOKEN` is not set, dashboard routes fall back to `MIKA_INTERNAL_TOKEN` (backwards compatible).

This separation lets you give dashboard users a read-only token that cannot mutate agent state.

### Mutation Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/health` | GET | None | Liveness/readiness probe |
| `/message` | POST | Internal token | Receives messages (202 async processing, 10MB body limit) |
| `/tasks/{id}/complete` | POST | Internal token | Completes a callback task (200 sync; 409 if already completed; 100KB result cap; echoes `task_id` in error bodies) |
| `/api/v1/rewind/resolve` | POST | Internal token | Resolve recent exchanges for a session (returns anchor point and trace IDs) |
| `/api/v1/rewind/preview` | POST | Internal token | Preview a rewind operation (messages to delete, reversals, task cancellations, warnings) |
| `/api/v1/rewind/execute` | POST | Internal token | Execute a rewind (delete messages, reverse mutations, cancel tasks, inject context marker) |

### Dashboard API (read-only)

All dashboard routes are nested under `/api/v1/` with CORS scoped to
`MIKA_CORS_ORIGIN` (default `http://localhost:5173`), restricted to
GET + POST + OPTIONS methods and `Authorization` + `Content-Type` headers only.
Auth: accepts `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN`.

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/v1/timeline` | GET | Paginated unified timeline with filters (agent_id, event_type, trace_id, session_id, from/to timestamps) |
| `/api/v1/timeline/trace/{trace_id}` | GET | All events for a specific trace |
| `/api/v1/agents` | GET | List all agents with message counts |
| `/api/v1/agents/{id}` | GET | Agent detail with core memory, soul.md, and stats |
| `/api/v1/agents/{id}/sessions` | GET | Paginated sessions for an agent |
| `/api/v1/agents/{id}/audit` | GET | Paginated audit events for an agent |
| `/api/v1/sessions` | GET | Paginated sessions with optional agent_id/channel_type filters |
| `/api/v1/sessions/{id}` | GET | Session detail |
| `/api/v1/sessions/{id}/messages` | GET | Paginated messages for a session (base64 images stripped) |
| `/api/v1/investigate` | POST | SSE streaming investigation endpoint — lightweight read-only agent loop (max 5 steps, 120s timeout, 64KB body limit) for analyzing agent behavior from the dashboard |

Pagination: `?page=1&per_page=50` (max 200 per page, page clamped to 1–100,000).
Response format: `{ data: [...], total: N, page: P, per_page: PP }`.

`AppState` is Clone via Arc-wrapped dependencies. Agent lock
(`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock`
(429 if busy).

See [ADR-001](adr/001-axum-http-server-architecture.md) for design decisions.


## 13a. Observability Dashboard

Source: `dashboard/`

A React SPA for monitoring agent activity. Stack: React 19, TypeScript, Vite,
Tailwind CSS v4, TanStack React Query, React Router, Lucide icons.

### Pages

| Route | Page | Description |
|-------|------|-------------|
| `/` | Event Timeline | Unified timeline with live polling (5s), subsystem badges, trace links |
| `/agents` | Agents | Agent list with status and message counts |
| `/agents/:id` | Agent Detail | Core memory (raw/view), soul.md, recent audit events, sessions |
| `/sessions` | Sessions | Filterable session list with channel type icons |
| `/sessions/:id` | Session Detail | Chat-style message thread with role-based styling, tool call summaries, and investigation side panel (SSE-powered agent analysis) |
| `/traces` | Traces | Trace ID search |
| `/traces/:id` | Trace Detail | All events for a trace |

### Development

```bash
# Terminal 1: Start mika-server
MIKA_ANTHROPIC_API_KEY=<key> MIKA_INTERNAL_TOKEN=<64-hex> \
  MIKA_ROUTING_URL=<gateway-url> cargo run --bin mika-server

# Terminal 2: Start dashboard dev server
VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev --prefix dashboard
```

The Vite dev server proxies `/api` requests to `http://localhost:8080`
(configurable in `dashboard/vite.config.ts`). Auth: the dashboard reads
`VITE_MIKA_DASHBOARD_TOKEN` from env and sends it as `Authorization: Bearer <token>`.

### Design System

Dark theme: `#0d0f12` background, `#151820` cards, `#7c6af7` accent.
Fonts: Plus Jakarta Sans (UI), JetBrains Mono (code/IDs).
Subsystem colors: blue (messages), amber (audit), emerald (tasks).


## 14. Failed Sends (Durable Outbox Pattern)

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


## 15. Multi-Agent Support

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
agents via the `delegate_task` tool. **Work item guard:** `delegate_task` requires a
`work_item_id` parameter — the agent must create a work item first using
`create_work_item`, then pass its ID. Calls without a valid, active work item are
rejected at code level. Long-running skill executions enforce the same guard via
schema injection (`inject_work_item_id_field`). The delegate runs with its own
personality, memory, and skills but receives only `default_tools()` (no management
tools, no MCP) to prevent infinite delegation chains. The system prompt includes an
"Agents & Teams" section listing available agents with their identities (emoji + name).

### Team Workflows

Teams are defined in `~/.mika/teams/{name}/team.toml` and orchestrated by the
`run_team` tool. Team runs and message graphs are persisted to the shared container
database (`~/.mika/data/mika.db`) with graph-structured messages linked via
`parent_id`. Queryable via `get_team_status` and `get_team_history` tools.

#### Conversation Continuity

At team run start, the engine queries the most recent completed/failed/suspended run
for the same team via `get_last_completed_team_run_summary()` and injects a structured
summary into the orchestrator's system prompt. The summary includes the previous goal,
agent result previews (200 chars each, top 5 agents), deliverable (500 chars), critic
feedback, task statuses, and any pending tasks. Total budget: 2500 chars. First runs
skip injection. The summary is available via `GET /api/v1/team-runs/:run_id/summary`.

See [ADR-004](adr/004-multi-agent-teams-orchestration.md) for team orchestration.


## 16. Observability & Telemetry

Mika follows an "always instrument, optionally export" pattern with two orthogonal
correlation axes:

- **`trace_id`** (per-request/per-turn): 32-char lowercase hex string generated via
  `trace::generate_trace_id()` (`crates/mika-agent/src/trace.rs`). Extracts the OTel
  trace ID from the current span when the `telemetry` feature is active; falls back to
  UUID v4 hex otherwise. Threaded through `ToolContext`, `LongRunningContext`,
  `TeamEngine`, and all DB write paths (messages, audit_events, tasks, team_workspace).
- **`session_id` / `agent_id`** (system-level): Identifies the conversation session and
  owning agent.

The `unified_timeline` VIEW (`UNION ALL` across messages, audit_events, tasks) enables
cross-subsystem queries by trace_id — e.g., "show all messages, audit events, and tasks
from a single agent turn."

Tracing spans are compiled unconditionally into the binary — no feature flags needed.
Spans cover the agent loop (`agent_turn`), Claude API calls, per-tool execution, team
engine (`team_run`, `team_agent_task`), and server HTTP handlers (`tower_http::TraceLayer`).

### Optional OTLP Export

Export is feature-gated behind `--features telemetry`. When enabled,
`mika_common::telemetry::build_otel_layer()` builds an OpenTelemetry tracing layer
that exports spans via OTLP/HTTP. The layer composes into the tracing subscriber
alongside the normal log layer.

Three environment variables control export:

| Variable | Purpose |
|----------|---------|
| `MIKA_TELEMETRY_ENABLED` | Enable trace export (`true`/`false`, default: false) |
| `MIKA_OTLP_ENDPOINT` | OTLP HTTP endpoint URL (must include `/v1/traces` path) |
| `MIKA_OTLP_AUTH_HEADER` | Authorization header value (e.g., Base64 credentials) |

`build_otel_layer()` returns a `TelemetryGuard` that flushes pending spans on drop,
ensuring no traces are lost at shutdown. Both `mika-server` and `mika` CLI hold
the guard alive until process exit.

### Langfuse Compatibility

The OTLP export is compatible with Langfuse's OpenTelemetry ingestion endpoint.
Set `MIKA_OTLP_ENDPOINT` to `https://cloud.langfuse.com/api/public/otel/v1/traces`
and `MIKA_OTLP_AUTH_HEADER` to `publicKey:secretKey` (auto-encoded to Base64) for
authentication. For Jaeger, use `http://localhost:4318/v1/traces` (no auth needed).

### Graceful Degradation

When the `telemetry` feature is not compiled in, `build_otel_layer()` is a no-op
that returns `None`. When compiled but `MIKA_TELEMETRY_ENABLED` is false or unset,
no exporter is created. Spans still flow to the normal log subscriber either way.


## Appendix: Database Schema

**Schema version:** 9 (v1→v3: clean-slate session+messages redesign; v4: adds `commitments` dedup indexes; v5: renames `memory_events` → `audit_events`, adds `trace_id` columns to messages/audit_events/team_workspace/tasks, creates `unified_timeline` VIEW for cross-subsystem correlation; v6: adds `mention_count` column to `people` table, incremented on each `update_person` call; v7: adds `skill_overrides` table to persist built-in skill `always_on` user preferences across `seed_bundled_skills()` re-sync cycles; v8: full table rebuild of `tasks` — adds `manual` trigger_type, `none` action_type, `blocked` status to CHECK constraints, adds `source TEXT` and `reference_url TEXT` columns, creates `idx_tasks_manual_active` partial index; v9: full table rebuild of `audit_events` — makes `after_value` nullable, adds `rewound_by_trace_id TEXT` column for rewind tracking, creates `idx_audit_rewound` partial index)

### Tables

| Table | Purpose |
|-------|---------|
| `schema_version` | Migration tracking |
| `agents` | Agent registry (name, display_name, model, active flag) |
| `teams` | Team registry (name, display_name, orchestrator) |
| `sessions` | Conversation sessions (`agent_id` FK, `channel_type`, timestamps) |
| `messages` | Message history (`session_id` FK, `role`, `content`, `trace_id`, `metadata`) |
| `core_memory` | Layer 1 persistent memory blocks (`agent_id` FK) |
| `people` | Layer 2 people/contacts (`agent_id` FK) |
| `commitments` | Layer 2 tasks/promises with status tracking (`agent_id` FK) |
| `preferences` | Layer 2 user preferences (`agent_id` FK) |
| `events` | Layer 2 notable events (`agent_id` FK) |
| `audit_events` | Audit log for all memory mutations (`agent_id` FK, `trace_id`, `rewound_by_trace_id`) |
| `customer_config` | Key-value store (timezone, chat_id) |
| `failed_sends` | Durable outbox for failed outbound messages |
| `audit_event_summaries` | Tiered retention summaries (monthly) |
| `skills` | Skill metadata (name, description, builtin flag, enabled) |
| `skill_tools` | Tool definitions per skill |
| `search_content` | Unified search content for Layer 3 hybrid search |
| `fts_search` | FTS5 virtual table for full-text search |
| `vec_search` | sqlite-vec virtual table (vec0) for vector similarity |
| `tasks` | Unified task scheduler — all proactive behaviors (`agent_id`, `action_type`, `status`, `cron_expression`, `next_fire_at`, `fired_at`, `completed_at`, `created_trace_id`) |
| `team_runs` | Team run metadata (goal, status, iterations, deliverable, checkpoint) |
| `team_workspace` | Graph-structured team workspace entries with `parent_id` links; `trace_id` column |
| `skill_overrides` | Persistent user overrides for built-in skill properties (`agent_id` + `skill_name` PK, `always_on` nullable integer) |
| `unified_timeline` | VIEW — UNION ALL across messages, audit_events, tasks for cross-subsystem correlation by trace_id |

### SQLite Pragmas

The database is initialized with WAL journal mode, NORMAL synchronous level, foreign
keys enabled, a 5-second busy timeout, and incremental auto-vacuum.
