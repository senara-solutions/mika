# Unified Task Engine — Brainstorm

**Date:** 2026-03-04
**Status:** Design complete, ready for planning

---

## What We're Building

A unified durable task engine that replaces three disconnected systems — the Tokio cron scheduler for reminders, the Python calendar sidecar, and the synchronous blocking agent loop — with a single reactive architecture backed by SQLite.

The core insight: every proactive behavior in Mika is fundamentally the same thing — *something fires when a trigger condition is met, with context, producing an outcome*. Reminders, calendar alerts, heartbeats, team delegation, user input requests, and long-running tools are all instances of one pattern. They should be one system.

---

## Why This Approach

**Unification over specialization.** The current codebase has three separate scheduling mechanisms, none of which can see the others. The task engine makes them composable. A reminder is just `trigger: time, action: send_message`. A heartbeat is `trigger: recurring, action: run_skill`. A team agent delegation is `trigger: callback, action: invoke_orchestrator`.

**Durability as a first principle.** The current Tokio scheduler is entirely in-memory. A container restart loses all scheduled timers — only reminders stored in SQLite survive because the scheduler queries them on startup. The task engine makes *all* scheduled work durable: every task exists in SQLite first, the in-memory priority queue is derived from it, and restart recovery is trivial.

**Single DB per container.** Currently the codebase maintains a separate SQLite file per agent and per team, making cross-agent queries impossible. The task engine requires a unified view. On a clean slate, the right answer is one `mika.db` per container, with agents and teams as rows in tables. Config files (TOML, soul.md) remain as source of truth for *configuration*; the DB tracks runtime state, relationships, and task ownership.

**No new dependencies.** The task engine is a plain Rust async loop using the existing `tokio`, `rusqlite`, and `chrono` stack. No workflow framework, no external queue, no additional services.

---

## Key Decisions

### Database Topology: One DB Per Container

**Decision:** Consolidate from the current per-agent + per-team DB sharding to a single `~/.mika/data/mika.db` per container.

**Rationale:** The "killer feature" — "show me all my reminders, pending agent work, and upcoming calendar alerts" — is a single SQL query. With per-agent sharding it becomes `ATTACH` gymnastics. Single DB also enables proper foreign keys between agents, teams, tasks, conversations, and memory.

**Impact on existing structure:**
- `~/.mika/agents/main/data/mika.db` → gone (data migrated to main DB, conversations get `agent_id` FK)
- `~/.mika/teams/engineering/data/mika.db` → gone (team_runs and team_messages move to main DB)
- `~/.mika/data/mika.db` → the single source of truth

### Agents and Teams: First-Class DB Citizens

**Decision:** `agents` and `teams` tables in the DB. Config files remain for configuration; DB tracks runtime state.

```sql
CREATE TABLE agents (
    id TEXT PRIMARY KEY,       -- slug: 'main', 'cto'
    name TEXT NOT NULL,
    home_dir TEXT NOT NULL,    -- filesystem path for soul.md, identity.toml, skills, mcp.json
    active BOOLEAN NOT NULL DEFAULT 1,
    last_seen INTEGER,         -- unix timestamp
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE teams (
    id TEXT PRIMARY KEY,       -- slug: 'engineering'
    name TEXT NOT NULL,
    config_path TEXT NOT NULL, -- path to team TOML
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

### Task Chain Depth: Cap at 3

**Decision:** Enforce a maximum chain depth of 3 (root → child → grandchild) via a `depth` column checked at insert time.

**Rationale:** The deepest real use case is team delegation: orchestrator (depth 0) → phase task (depth 1) → agent delegation task (depth 2). Nothing in the design requires depth 4+. If an orchestrator needs depth 5, it should decompose better, not chain deeper. Bumping the constant is a one-line change if reality ever demands it.

### User Reply Routing: Per-Task Timeout Window

**Decision:** `user_reply` tasks carry a `timeout_at` timestamp. Any user reply that arrives while a `user_reply` task is `pending` and before `timeout_at` is routed to complete that task. After expiry, the task transitions to `expired`, and the next user message routes to normal conversation.

**No LLM disambiguation** — spending an LLM call to route every single message while a task is pending adds latency and cost to every interaction. The time window is simpler, predictable, and sufficient.

Default `timeout_at`: 30 minutes from task creation. Per-task override: the creating agent (or tool) sets `timeout_at` based on the nature of the question:
- Quick confirmation → 5 minutes
- Decision requiring thought → 2 hours

### Task Priority: FIFO

**Decision:** No priority system. Tasks execute in `next_fire_at` order.

**Rationale:** This is a single-user system processing ~20 tasks per day. The probability of two tasks firing on the same second is effectively zero. Sequential FIFO processing with millisecond separation is imperceptible. Priority systems are for multi-tenant platforms. If the Langfuse traces ever reveal a collision problem, priority can be added then.

### Process Termination on Cancel

**Decision:** Track the OS PID of background processes in `tasks.process_id`. On task cancellation: `SIGTERM` the process if the PID is still alive.

**Scope:** Applies to `trigger_type = 'callback'` tasks where the handler is a spawned shell process (exec skills). Does not apply to tasks where background work is an async Rust future (nothing to SIGTERM there).

---

## Full Schema (Clean Slate)

All timestamps use INTEGER unix timestamps (`unixepoch()`). All text enum columns use CHECK constraints. COLLATE NOCASE on human-entered unique identifiers. WAL mode, NORMAL synchronous, foreign_keys ON, busy_timeout 5000ms.

### Identity Tables

```sql
CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    home_dir TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT 1,
    last_seen INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    config_path TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

### Task Engine (Core Primitive)

```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,                       -- UUID v4

    -- Ownership
    agent_id TEXT NOT NULL REFERENCES agents(id),
    team_run_id TEXT REFERENCES team_runs(id), -- set when task is part of a team run

    -- Chain structure (max depth 3)
    parent_task_id TEXT REFERENCES tasks(id),
    depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),

    -- Human-readable label (shown in /tasks TUI)
    label TEXT NOT NULL,

    -- Trigger (what causes this task to fire)
    trigger_type TEXT NOT NULL CHECK (trigger_type IN (
        'time', 'recurring', 'callback', 'user_reply', 'event', 'condition'
    )),
    cron_expr TEXT,               -- 'recurring' only: cron expression
    event_source TEXT,            -- 'event' only: external event identifier (calendar event id)
    event_offset_secs INTEGER,    -- 'event' only: fire N seconds before the event start
    condition_expr TEXT,          -- 'condition' only: evaluated during heartbeat (JSON or DSL TBD)

    -- Scheduling
    next_fire_at INTEGER,         -- unix timestamp; NULL for callback/user_reply until externally triggered
    timeout_at INTEGER,           -- unix timestamp; NULL = no expiry (use with care)

    -- Action (what happens when this task fires)
    action_type TEXT NOT NULL CHECK (action_type IN (
        'send_message', 'resume_agent', 'inject_context', 'run_skill', 'invoke_orchestrator'
    )),
    action_config TEXT NOT NULL DEFAULT '{}', -- JSON: action-specific parameters

    -- Status
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending', 'in_progress', 'completed', 'failed', 'cancelled', 'expired', 'recurring_active'
    )),

    -- Background process (for SIGTERM on cancel)
    process_id INTEGER,           -- OS PID of spawned handler process

    -- Agent suspension context (for resume_agent and invoke_orchestrator)
    -- Stores: {conversation_message_ids: [int], last_message_id: int}
    -- NOT the system prompt (rebuilt fresh on resume)
    input_context TEXT,           -- JSON
    result TEXT,                  -- JSON: task output, injected into agent on resume

    -- Provenance
    created_by_session TEXT,      -- session_id of the agent turn that created this task

    -- Audit
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    fired_at INTEGER,             -- last fire timestamp
    completed_at INTEGER          -- set when reaching a terminal state
);

-- Indexes optimized for the engine tick query
CREATE INDEX idx_tasks_fire_queue ON tasks(next_fire_at, status)
    WHERE status IN ('pending', 'recurring_active');
CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
CREATE INDEX idx_tasks_user_reply ON tasks(agent_id, trigger_type, status)
    WHERE trigger_type = 'user_reply';
CREATE INDEX idx_tasks_team_run ON tasks(team_run_id) WHERE team_run_id IS NOT NULL;
CREATE INDEX idx_tasks_parent ON tasks(parent_task_id) WHERE parent_task_id IS NOT NULL;
```

**`action_config` examples by action type:**

```json
// send_message
{"text": "Reminder: Call Alice about the Q1 budget"}

// resume_agent
{"conversation_id": "main", "resume_prompt": "The analysis is complete."}

// inject_context
{"context_type": "calendar_alert", "event_title": "Board meeting", "event_at": 1741090800}

// run_skill
{"skill_name": "heartbeat", "args": {}}

// invoke_orchestrator
{"team_run_id": "abc-123", "phase": "review", "sibling_task_ids": ["task-1", "task-2", "task-3"]}
```

### Conversations

```sql
CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    metadata TEXT,                 -- JSON: tool_call summary [{name, input_preview, output_preview, success}]
    compacted_through_id INTEGER,  -- last message ID covered by the conversation summary
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_conversations_agent_id ON conversations(agent_id, id);
CREATE INDEX idx_conversations_channel ON conversations(agent_id, channel_type, id);
CREATE INDEX idx_conversations_created ON conversations(agent_id, created_at);
```

**Compaction + suspension interaction:** Before compacting, the compaction routine checks `SELECT COUNT(*) FROM tasks WHERE agent_id=? AND status IN ('pending', 'in_progress') AND action_type='resume_agent'`. If >0, compaction is skipped for that agent — the suspended conversation must be preserved intact for resumption.

### Memory (Per-Agent, All Tables Get agent_id FK)

```sql
CREATE TABLE core_memory (
    agent_id TEXT NOT NULL REFERENCES agents(id),
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    token_count INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (agent_id, key)
);

CREATE TABLE people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    canonical_name TEXT NOT NULL COLLATE NOCASE,
    relationship TEXT,
    notes TEXT,
    first_mentioned INTEGER NOT NULL DEFAULT (unixepoch()),
    last_mentioned INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (agent_id, canonical_name)
);

CREATE TABLE commitments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    description TEXT NOT NULL COLLATE NOCASE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed', 'cancelled')),
    due_date TEXT,
    person_id INTEGER REFERENCES people(id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER,
    UNIQUE (agent_id, description)
);
CREATE INDEX idx_commitments_status ON commitments(agent_id, status);
CREATE INDEX idx_commitments_due ON commitments(due_date);

CREATE TABLE preferences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    category TEXT NOT NULL COLLATE NOCASE,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (agent_id, category)
);

CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    description TEXT NOT NULL,
    event_date TEXT,
    context TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_events_date ON events(agent_id, event_date);

CREATE TABLE memory_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    session_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    target_key TEXT NOT NULL,
    before_value TEXT,
    after_value TEXT,
    reasoning TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_memory_events_agent ON memory_events(agent_id, session_id);
CREATE INDEX idx_memory_events_created ON memory_events(agent_id, created_at);

CREATE TABLE memory_event_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,
    tool_counts TEXT NOT NULL,       -- JSON
    category_counts TEXT NOT NULL,   -- JSON
    total_mutations INTEGER NOT NULL,
    top_targets TEXT NOT NULL,       -- JSON
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (agent_id, year, month)
);
```

### Layer 3 Search

```sql
CREATE TABLE search_content (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    source_type TEXT NOT NULL,   -- 'people', 'commitments', 'preferences', 'events'
    source_id INTEGER,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_search_content_agent ON search_content(agent_id, source_type, source_id);

-- FTS5 and vec0 virtual tables cannot have FK columns, use content_id
CREATE VIRTUAL TABLE fts_search USING fts5(
    content,
    content_id UNINDEXED,
    agent_id UNINDEXED,
    source_type UNINDEXED,
    tokenize='porter unicode61'
);

CREATE VIRTUAL TABLE vec_search USING vec0(
    content_id INTEGER PRIMARY KEY,
    embedding float[512]
);
```

### Team Runs (Moved from Per-Team DB)

```sql
CREATE TABLE team_runs (
    id TEXT PRIMARY KEY,           -- UUID
    team_id TEXT NOT NULL REFERENCES teams(id),
    goal TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed')),
    failure_reason TEXT,
    iteration INTEGER NOT NULL DEFAULT 0,
    max_iterations INTEGER NOT NULL DEFAULT 3,
    deliverable TEXT,
    started_at INTEGER NOT NULL DEFAULT (unixepoch()),
    ended_at INTEGER
);
CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at);

CREATE TABLE team_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES team_runs(id),
    parent_id INTEGER REFERENCES team_messages(id),
    agent_id TEXT REFERENCES agents(id),
    message_type TEXT NOT NULL CHECK (message_type IN (
        'goal', 'orchestrator', 'assignment', 'agent_response', 'error', 'deliverable'
    )),
    content TEXT NOT NULL,
    iteration INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_team_messages_run ON team_messages(run_id, created_at);
CREATE INDEX idx_team_messages_parent ON team_messages(parent_id);
```

### Operational Tables

```sql
CREATE TABLE heartbeat_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    sent_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_heartbeat_sends ON heartbeat_sends(agent_id, sent_at);

CREATE TABLE reflection_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    ran_at INTEGER NOT NULL DEFAULT (unixepoch()),
    status TEXT NOT NULL DEFAULT 'completed' CHECK (status IN ('completed', 'failed')),
    changes_made INTEGER DEFAULT 0,
    summary TEXT
);
CREATE INDEX idx_reflection_runs ON reflection_runs(agent_id, ran_at);

CREATE TABLE customer_config (
    agent_id TEXT NOT NULL REFERENCES agents(id),
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (agent_id, key)
);

CREATE TABLE failed_sends (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    text TEXT NOT NULL,
    request_id TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    retry_count INTEGER NOT NULL DEFAULT 0
);
```

---

## Task Engine Architecture

### The Engine Loop (Replaces `ReminderScheduler`)

```
struct TaskEngine {
    db: AsyncDatabase,
    // In-memory priority queue: BinaryHeap sorted by next_fire_at ASC
    queue: BinaryHeap<QueuedTask>,
    dispatcher: Arc<TaskDispatcher>,
}
```

**Startup:**
1. Load all tasks with `status IN ('pending', 'recurring_active')` from SQLite.
2. Recover `in_progress` tasks: check if their `process_id` is still alive; restart or mark failed.
3. Expire tasks where `timeout_at IS NOT NULL AND timeout_at <= unixepoch()` → mark `expired`.
4. Compute `next_fire_at` for `recurring` tasks using `cron_expr` + `chrono`.
5. Build in-memory `BinaryHeap<QueuedTask>` sorted by `next_fire_at ASC`.

**Engine tick (every 1 second via `tokio::time::interval`):**
1. Peek at queue head. If `next_fire_at > now`, sleep until next tick.
2. Pop all tasks with `next_fire_at <= now`.
3. For each: transition to `in_progress` in DB, dispatch action (see dispatcher below).
4. For `recurring` tasks that completed: compute next `next_fire_at`, update DB, re-enqueue.
5. For completed one-shot tasks: update `status = 'completed'`.

**External triggers (callback endpoint, user reply):**
- `POST /tasks/{id}/complete` — gateway or background process posts the result.
- User reply matched to a pending `user_reply` task → same flow.
Both paths: update task in SQLite, then dispatch the action immediately (no wait for next tick).

**Condition-based tasks** (`trigger_type = 'condition'`): evaluated during heartbeat task execution, not on their own schedule. The heartbeat skill queries `SELECT * FROM tasks WHERE trigger_type='condition' AND status='pending'` and evaluates each `condition_expr` against current context.

### Task Dispatcher

Dispatches based on `action_type`:

- `send_message` — call `message_sender.send(text)`. Simple.
- `resume_agent` — rebuild system prompt fresh, load core memory fresh, load saved conversation messages by ID, inject task result, call `run_agent_inner()`.
- `inject_context` — write to a `pending_context` staging area in DB (new table or existing `core_memory` with a namespaced key). The next `run_agent()` call picks it up and injects it into the first user turn.
- `run_skill` — call `run_silent_agent()` with the specified skill as the directive.
- `invoke_orchestrator` — check if all sibling tasks in the team run phase are complete; if so, assemble results and call `run_team_agent()` on the orchestrator with all subtask results injected.

### Long-Running Tool Integration

**tools.json skill declaration:**
```json
{
    "name": "analyze_codebase",
    "handler": {
        "type": "exec",
        "command": "handlers/run.sh",
        "long_running": true,
        "estimated_duration_secs": 120
    }
}
```

**Agent loop handling:**
1. Agent calls a `long_running` tool.
2. `execute_tool()` detects `long_running: true`.
3. Creates a task: `trigger='callback', action='resume_agent'`, saves current message IDs to `input_context`.
4. Spawns the exec handler as a background process, stores PID in `tasks.process_id`.
5. Returns to the agent loop: `"Task submitted (ID: {id}). I'll resume when it completes."`.
6. Agent loop ends gracefully — this is a clean exit, not a timeout.
7. Handler completes → `POST /tasks/{id}/complete` with result JSON.
8. Task engine fires → `resume_agent` action → agent loop restarts with fresh context + result injected.

---

## How Existing Features Map to Tasks

| Current mechanism | After task engine | `trigger_type` | `action_type` |
|---|---|---|---|
| Tokio cron reminder | Task row in DB | `time` | `send_message` |
| Recurring reminder | Task row, recurring | `recurring` | `send_message` |
| Calendar sidecar poll | Calendar skill creates tasks | `event` | `inject_context` |
| Heartbeat endpoint | Recurring task | `recurring` | `run_skill` |
| Reflection run | Recurring task | `recurring` | `run_skill` |
| Team agent delegation | Async tasks w/ callback | `callback` | `invoke_orchestrator` |
| User input request | New capability | `user_reply` | `resume_agent` |
| Long-running tool | New capability | `callback` | `resume_agent` |

The `ReminderScheduler` struct is **deleted**. The `reminders` table is **deleted** (reminders are now `tasks` with `trigger='time'`). The `heartbeat_sends` table is kept for rate limiting. The per-team DB is **eliminated** (team_runs and team_messages move to the main DB).

---

## TUI/Dashboard Integration

**`/tasks` slash command:** Lists all tasks for the current agent, grouped by status. Columns: ID, label, trigger type, action type, next_fire_at, status.

**TUI footer:** Shows pending task count badge (e.g., `[3 tasks pending]`).

**Unified natural language query:** `"Show me everything I'm waiting on"` → single SQL:
```sql
SELECT label, trigger_type, action_type, status, next_fire_at
FROM tasks
WHERE agent_id = 'main'
  AND status IN ('pending', 'in_progress', 'recurring_active')
ORDER BY COALESCE(next_fire_at, 9999999999) ASC;
```

**Team dashboard:** Reads task status per agent during team runs. `TeamEvent::AgentStarted` and `TeamEvent::PhaseChanged` events now map to task status transitions.

---

## Migration Strategy

Since there's no backward compatibility constraint (single user), the migration is a clean cut:

1. Schema version 12: create new tables (`agents`, `teams`, updated `tasks`, all tables with `agent_id` FK).
2. Startup migration script: auto-register all existing agent directories into `agents` table, auto-register team configs into `teams` table.
3. Migrate existing `reminders` rows → `tasks` rows (`trigger_type='time'`, `action_type='send_message'`).
4. Migrate `team_runs`/`team_messages` from per-team DBs into main DB.
5. Drop `reminders` table.
6. All existing per-agent SQLite files are merged into the single container DB.

---

## Open Questions

### Resolved

- **Chain depth:** Cap at 3, enforced via `depth` column at insert time.
- **User reply routing:** Per-task `timeout_at`, default 30 min, expires to `expired` status.
- **Task priority:** FIFO (no priority system); single-user system, collision probability ~zero.
- **DB topology:** Single DB per container.
- **Agents/teams in DB:** Yes, `agents` and `teams` tables.
- **Process kill on cancel:** Yes, SIGTERM via `process_id` column.
- **`inject_context` staging:** Scan tasks at prompt build time. The agent loop scans for `inject_context` tasks with `status='in_progress'` at the start of each turn, reads their `result` field, injects into the system prompt context, marks them `completed`. No new table needed.
- **`condition_expr` format:** Plain text, LLM-evaluated during heartbeat. The heartbeat agent reads all pending `condition` tasks and evaluates each `condition_expr` in natural language. Structured predicates are YAGNI.

### Still Open

1. **`event` trigger source format:** Calendar events come from an external skill. How does the skill communicate the event's `fire_at` to the task engine? The task needs `event_at` (when the event starts) and `event_offset_secs` (how early to fire). The calendar sync skill creates tasks; the engine fires them. The `event_source` column carries the calendar event ID for deduplication on re-sync.

2. **Server mode: where does the `POST /tasks/{id}/complete` endpoint live?** On the `mika-server` binary. Background processes POST directly to `mika-server`'s local endpoint (within the container). The gateway is not involved — it's container-internal only.

3. **Max concurrent pending tasks per agent:** Suggested cap: 100 pending tasks (check at create time). Seems large enough to never matter in practice. This is an implementation detail for the planning phase.

4. **Task expiry GC:** Suggested: delete tasks older than 90 days in terminal states (`completed`, `failed`, `cancelled`, `expired`). Run during the reflection job. `recurring_active` tasks are never GC'd. Implementation detail for planning phase.

---

## Reference Patterns Considered

- **Temporal.io / Durable Functions:** The inspiration for durable workflow suspension. We're implementing a subset: single-level suspension (no nested workflows), SQLite as the durable store, no separate worker fleet. Key insight borrowed: don't store system prompt in the checkpoint — it gets stale. Rebuild fresh on resume.
- **LangGraph checkpointing:** Storing conversation state at a checkpoint ID. We store message IDs (not message content) — the messages are already in `conversations`, we just need to know where to resume from.
- **Celery:** Task queue with result backend. Our `tasks` table is both the queue and the result backend. The `AsyncDatabase` thread is our single-worker executor.
- **The key constraint:** This is a lightweight durable workflow engine embedded in a single-user assistant. It fits in SQLite. Don't reach for Temporal's complexity — the right tool is the right size.
