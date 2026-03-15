# Runtime Structure Reference

This document describes the `~/.mika` runtime directory layout, SQLite database schema, and log file locations. For config file formats, environment variables, and config cascade, see [configuration.md](configuration.md). For skill.toml format, see [skills.md](skills.md).

## Directory Layout

Home directory: `$MIKA_HOME` (default `~/.mika/`).

```
~/.mika/                              # Global home (0700)
├── .env                              # Secrets (0600), loaded by dotenvy
├── config.toml                       # Global config (shared across agents)
├── active_agent                      # Plain text: active agent name
├── data/
│   ├── mika.db                       # Shared SQLite database (all agents)
│   └── mika.db.vN-backup             # Auto-backup before migrations
├── agents/
│   └── mika/                         # Default agent (same structure per agent)
│       ├── config.toml               # Per-agent config overrides (0600)
│       ├── identity.toml             # Agent name + emoji (0600)
│       ├── soul.md                   # Personality definition (0600)
│       ├── heartbeat.md              # Heartbeat checklist (0600)
│       ├── user.md                   # User info seed (0600)
│       ├── cli-reference.md          # Auto-generated CLI help
│       ├── mcp.json                  # MCP server config (0600)
│       ├── marketplace.lock          # Marketplace skill tracking (TOML)
│       ├── .input_history            # TUI input history (JSON, 0600)
│       ├── logs/                     # Daily-rotating logs (0700)
│       │   ├── mika.log
│       │   └── mika.log.YYYY-MM-DD
│       ├── skills/                   # Skill directories (0700)
│       │   ├── self-knowledge/
│       │   ├── shell-exec/
│       │   ├── web-search/
│       │   ├── file-reader/
│       │   ├── tmux/
│       │   ├── github/
│       │   ├── mcp/
│       │   ├── agents-teams/
│       │   └── <marketplace-skill>/
│       └── exports/                  # Conversation exports
└── teams/
    └── <team-name>/
        ├── team.toml                 # Team definition
        ├── workspace/
        │   └── <run-uuid>/           # Per-run workspace (isolated)
        │       ├── .meta/            # Engine-written metadata
        │       │   ├── goal.md
        │       │   ├── assignments.md
        │       │   ├── critic_feedback.md
        │       │   └── deliverable.md
        │       └── <agent-output>    # Agent-written files
        └── logs/                     # Team-mode logs
```

**Key invariant:** The database is shared across all agents (`~/.mika/data/mika.db`). Agent/team home directories are used only for config, skills, and file I/O — never for database paths. Always use `home::container_db_path()`.

## SQLite Database

**Location:** `~/.mika/data/mika.db` (single file, WAL mode)

**PRAGMAs:** `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000`, `auto_vacuum=INCREMENTAL`

**Current schema version:** 10

### Core Tables

**schema_version** — `version INTEGER`, `applied_at INTEGER DEFAULT unixepoch()`

**agents** — `id TEXT PK`, `name TEXT NOCASE`, `home_dir TEXT`, `active BOOLEAN DEFAULT 1`, `last_seen INTEGER`, `created_at INTEGER`

**sessions** — `id TEXT PK`, `agent_id TEXT FK→agents`, `channel_type TEXT DEFAULT 'cli'`, `started_at INTEGER`, `ended_at INTEGER`, `metadata TEXT`

**messages** — `id INTEGER PK AUTO`, `session_id TEXT FK→sessions`, `agent_id TEXT FK→agents`, `role TEXT CHECK(user|assistant|system|summary|tool_result)`, `content TEXT`, `metadata TEXT`, `trace_id TEXT`, `compacted_through_id INTEGER`, `created_at INTEGER`

### Memory Tables

**core_memory** — `(agent_id, key) PK`, `value TEXT`, `token_count INTEGER`, `updated_at INTEGER`. Sections: `user_summary`, `self_model`, `current_priorities`, `key_people`, `workflows`.

**people** — `id INTEGER PK AUTO`, `agent_id TEXT FK→agents`, `canonical_name TEXT NOCASE`, `relationship TEXT`, `notes TEXT`, `first_mentioned INTEGER`, `last_mentioned INTEGER`, `mention_count INTEGER DEFAULT 1`. Unique: `(agent_id, canonical_name)`.

**commitments** — `id INTEGER PK AUTO`, `agent_id FK`, `description TEXT NOCASE`, `status TEXT CHECK(pending|completed|cancelled)`, `due_date TEXT`, `person_id FK→people`, `created_at`, `completed_at`

**preferences** — `(agent_id, category) PK`, `value TEXT`, `updated_at INTEGER`

**events** — `id INTEGER PK AUTO`, `agent_id FK`, `description TEXT`, `event_date TEXT`, `context TEXT`, `created_at`

**search_content** — `id INTEGER PK AUTO`, `agent_id FK`, `source_type TEXT`, `source_id INTEGER`, `content TEXT`, `embedding_json TEXT`, `created_at`, `updated_at`

**Virtual tables:** `fts_search` (FTS5 on search_content), `vec_search` (sqlite-vec, float[512])

### Task Engine

**tasks** — `id TEXT PK`, `agent_id FK`, `team_run_id FK→team_runs`, `parent_task_id FK→tasks(self)`, `depth INTEGER CHECK(0..3)`, `label TEXT`, `trigger_type TEXT CHECK(time|recurring|callback|user_reply|event|condition|manual)`, `cron_expr TEXT`, `event_source TEXT`, `event_offset_secs INTEGER`, `condition_expr TEXT`, `next_fire_at INTEGER`, `timeout_at INTEGER`, `action_type TEXT CHECK(send_message|resume_agent|inject_context|run_skill|invoke_orchestrator|none)`, `action_config TEXT DEFAULT '{}'`, `status TEXT CHECK(pending|in_progress|completed|failed|cancelled|expired|recurring_active|delivered|blocked)`, `process_id INTEGER`, `input_context TEXT`, `result TEXT`, `created_by_session TEXT`, `created_trace_id TEXT`, `reference_url TEXT`, `source TEXT`, `created_at`, `updated_at`, `fired_at`, `completed_at`

### Team Tables

**teams** — `id TEXT PK`, `name TEXT NOCASE`, `config_path TEXT`, `created_at INTEGER`

**team_runs** — `id TEXT PK`, `team_id FK→teams`, `goal TEXT`, `status TEXT CHECK(running|completed|failed|cancelled|suspended)`, `failure_reason TEXT`, `iteration INTEGER DEFAULT 1`, `max_iterations INTEGER DEFAULT 3`, `deliverable TEXT`, `checkpoint TEXT`, `trace_id TEXT`, `started_at INTEGER`, `ended_at INTEGER`

**team_workspace** — `id INTEGER PK AUTO`, `run_id FK→team_runs`, `parent_id FK→self`, `agent_name TEXT`, `entry_type TEXT`, `content TEXT`, `trace_id TEXT`, `iteration INTEGER DEFAULT 1`, `created_at`

### Audit Tables

**audit_events** — `id INTEGER PK AUTO`, `agent_id FK`, `session_id TEXT`, `tool_name TEXT`, `target_key TEXT`, `before_value TEXT`, `after_value TEXT` (nullable), `reasoning TEXT`, `trace_id TEXT`, `rewound_by_trace_id TEXT`, `created_at`

**audit_event_summaries** — `id INTEGER PK AUTO`, `agent_id FK`, `year INTEGER`, `month INTEGER`, `summary TEXT`, `event_count INTEGER`, `created_at`. Unique: `(agent_id, year, month)`.

### System Tables

**heartbeat_sends** — `id INTEGER PK AUTO`, `agent_id FK`, `sent_at INTEGER`

**reflection_runs** — `id INTEGER PK AUTO`, `agent_id FK`, `status TEXT`, `changes_made INTEGER DEFAULT 0`, `summary TEXT`, `created_at`

**customer_config** — `(agent_id, key) PK`, `value TEXT`, `updated_at INTEGER`

**failed_sends** — `id INTEGER PK AUTO`, `agent_id FK`, `text TEXT`, `request_id TEXT`, `retry_count INTEGER DEFAULT 0`, `created_at`

**skill_overrides** — `(agent_id NOCASE, skill_name NOCASE) PK`, `always_on INTEGER`

### View

**unified_timeline** — `UNION ALL` across `messages`, `audit_events`, `tasks`, `team_workspace`. Columns: `trace_id`, `session_id`, `agent_id`, `event_type`, `event_subtype`, `summary` (truncated to 200 chars), `created_at`. Team workspace entries use `event_type='team_workspace'`, synthetic `session_id='team-{run_id}'`, and `agent_id=NULL`.

### Notable Indexes

Unique partial indexes (duplicate prevention):
- `idx_tasks_unique_recurring` — one active recurring task per (agent, label)
- `idx_tasks_unique_reminder` — one active reminder per (agent, label)
- `idx_events_unique_description` — one event per (agent, description, date)
- `idx_commitments_unique_pending` — one pending commitment per (agent, description, due_date)

Performance indexes: `idx_tasks_schedulable` (pending/recurring by next_fire_at), `idx_msg_session` (messages by session+time), `idx_msg_agent_created`, `idx_sessions_agent`.

Partial trace indexes: `idx_msg_trace`, `idx_audit_trace`, `idx_tasks_trace`, `idx_team_ws_trace` (WHERE NOT NULL).

Callback delivery: `idx_tasks_callback_delivery` (partial, for TUI polling).

## Log File Locations

| Binary | Location | Format | Rotation |
|--------|----------|--------|----------|
| `mika` (CLI) | `{agent_home}/logs/mika.log` | JSON | Daily (tracing_appender) |
| `mika` (team mode) | `{team_dir}/logs/mika.log` | JSON | Daily |
| `mika-server` | stdout | JSON | None |
| `mika-server` | `$MIKA_SERVER_LOG_FILE` (optional) | JSON | None |
| `mika-gateway` | stdout | JSON | None |
| `mika-gateway` | `$MIKA_GATEWAY_LOG_FILE` (optional) | JSON | None |

CLI TUI mode logs to file only (no stderr, protects ratatui alternate screen). Non-TUI subcommands log to both stderr (pretty) and file (JSON).
