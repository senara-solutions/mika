# Plan: feat(agent-core): single-session-by-nature agents (mika#1401)

## Problem

Today, `mika ask` and the HTTP `/send` handler mint a fresh `Uuid::new_v4()` session on every invocation unless the caller explicitly threads `--session-id`. For agents like Mika Prime — who are conceptually single-session — this is held by discipline only. Forget the flag and the invariant breaks, scattering the agent's history across per-ask session confetti.

## Goal

An opt-in per-agent `identity.toml` property that makes an agent **single-session by construction**: it has exactly one canonical session, by nature, not by discipline. The default for non-opted-in agents remains unchanged (random UUID per ask).

## Non-Goals

- No "session-policy" machinery or multi-tier session strategies (YAGNI)
- No changes to compaction behavior (compaction already keys on `agent_id`, not `session_id`)
- No changes to the silent/callback/heartbeat session paths (those use their own derived namespaces: `callback-*`, `heartbeat-*`, etc.)

## Design Decisions

### D1: Identity property shape

Add a `[session]` section to `identity.toml` with two fields:

```toml
[session]
singleton = true
canonical_id = "00000000-0000-0000-0000-000000000000"  # optional
```

- `singleton` (bool, default `false`): when `true`, the agent is single-session-by-nature.
- `canonical_id` (string, optional): the literal session ID to use. When absent, the engine derives one as `canonical-{agent_id}` (using the prefix-typed family per the ticket's mechanism section, sibling to `system-{agent_id}`, `delegate-{uuid}`, etc.).

This matches the existing identity configuration pattern (`[kg]`, `[context.summary]`, `[skills]`, `[tools]`). The `canonical_id` field exists specifically for mika-prime's operator-chosen zero-UUID — other future opt-in agents can omit it and get the deterministic derivation.

### D2: Session resolution change points

Three code paths create sessions for conversational use:

| Path | File | Current behavior | New behavior |
|------|------|-----------------|--------------|
| CLI `mika ask` | `crates/mika-cli/src/commands/ask.rs:107-109` | `Uuid::new_v4()` when no `--session-id` | Resolve canonical session from identity when `singleton = true` |
| CLI `mika chat` (TUI) | `crates/mika-cli/src/commands/chat.rs` + `tui/app.rs` | `Uuid::new_v4()` on start and `/clear` | Resolve canonical session; `/clear` clears messages but reuses the same session ID |
| HTTP `/send` handler | `crates/mika-agent/src/server/handlers.rs:742` | `Uuid::new_v4()` | Resolve canonical session from identity (loaded at `init_agent` time, cached on `AgentState`) |

**Explicit `--session-id` always wins.** When the caller provides `--session-id`, it is used verbatim regardless of the `singleton` flag. This preserves backward compatibility and allows diagnostic/relay sessions.

### D3: `end_session` becomes a no-op for singleton agents

`db.rs:6569` `end_session(id)` sets `ended_at`. For singleton agents, this must be a no-op so the canonical session is never ended. This is critical for pruning safety — `prune_old_sessions` deletes rows with `ended_at IS NOT NULL` (though the current pruning query also requires specific prefixes like `heartbeat-`, `callback-`, etc., so a singleton session is doubly safe).

Implementation: add a `end_session_if_not_singleton(id, is_singleton)` helper, or check the session prefix against the canonical namespace before ending.

### D4: Canonical session creation uses `INSERT OR IGNORE`

Like `get_or_create_system_session`, the canonical session creation is idempotent. First invocation creates the row; subsequent invocations reuse it. This means every `mika ask` to a singleton agent hits the same session row.

### D5: Derived session ID namespace

The canonical session ID follows the deployed prefix-typed family:
- `system-{agent_id}` — compaction summaries (existing)
- `canonical-{agent_id}` — singleton agent conversational session (new)
- `00000000-0000-0000-0000-000000000000` — mika-prime override via `canonical_id`

The `canonical-` prefix is **not** in `prune_old_sessions`'s LIKE clause list, so it's structurally exempt from pruning. The zero-UUID is also exempt (no matching prefix).

### D6: Concurrency documentation

For a single-surface, zero-skill oracle (mika-prime's current profile), concurrency is low. The singleton merges all channels into one thread — for a manual-ask oracle this is the intent. The code comments and docs will state this is designed behavior, per the ticket's Footnote Check 1.

### D7: Migration script for mika-prime

The PR ships a documented operator-runnable SQL script that merges existing non-canonical sessions into the canonical zero-UUID. The script:

1. Wraps in `PRAGMA foreign_keys = OFF; BEGIN; ... COMMIT; PRAGMA foreign_keys = ON;`
2. Updates all FK-referencing tables for mika-prime's agent_id where session_id != canonical
3. Deletes the now-orphaned old session rows
4. Is idempotent (WHERE clauses self-narrow after first run)

Tables to update (per ticket's operator-authored amendment):
- `sessions.id` (the row identity itself — handle via delete-old-after-merge)
- `sessions.parent_session_id`
- `messages.session_id`
- `audit_events.session_id`
- `a2a_task_map.session_id`
- `llm_calls.session_id`
- `tool_calls.session_id`
- `tasks.created_by_session`

## Implementation Steps

### Step 1: Add `SessionIdentityConfig` to `Identity` struct

**File:** `crates/mika-agent/src/prompt.rs`

Add a new config struct and field:

```rust
#[derive(Debug, Deserialize, Clone, Default)]
pub struct SessionIdentityConfig {
    /// When true, this agent uses a single canonical session for all
    /// conversational interactions. The session persists across invocations
    /// and is never ended by `end_session`. Default: false.
    #[serde(default)]
    pub singleton: bool,
    /// Explicit canonical session ID. When absent and singleton=true,
    /// the engine derives one as `canonical-{agent_id}`.
    #[serde(default)]
    pub canonical_id: Option<String>,
}
```

Add to `Identity`:
```rust
#[serde(default)]
pub session: SessionIdentityConfig,
```

Update `Identity::default()` to include `session: SessionIdentityConfig::default()`.

### Step 2: Add `get_or_create_canonical_session` to `Database`

**File:** `crates/mika-agent/src/db.rs`

Add a function mirroring `get_or_create_system_session`:

```rust
pub fn get_or_create_canonical_session(&self, session_id: &str, agent_id: &str) -> Result<String> {
    self.conn.execute(
        "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, 'cli')",
        params![session_id, agent_id],
    )?;
    Ok(session_id.to_string())
}
```

Add the async wrapper in `async_db.rs`.

### Step 3: Add `resolve_canonical_session_id` helper

**File:** `crates/mika-agent/src/prompt.rs` (or a new `session.rs` module if cleaner)

```rust
/// Resolve the canonical session ID for a singleton agent.
/// Returns `None` if the agent is not singleton.
pub fn resolve_canonical_session_id(identity: &Identity, agent_id: &str) -> Option<String> {
    if !identity.session.singleton {
        return None;
    }
    Some(
        identity.session.canonical_id
            .clone()
            .unwrap_or_else(|| format!("canonical-{agent_id}"))
    )
}
```

### Step 4: Update CLI `mika ask` session resolution

**File:** `crates/mika-cli/src/commands/ask.rs`

At line 106-109, change the session resolution:

```rust
let reusing_session = session_id.is_some();
let identity = mika_agent::prompt::load_identity(&ctx.home_dir);
let canonical = mika_agent::prompt::resolve_canonical_session_id(&identity, agent_name);

let session_id = if let Some(s) = session_id {
    s.to_string()
} else if let Some(ref canonical_id) = canonical {
    canonical_id.clone()
} else {
    Uuid::new_v4().to_string()
};
```

When using a canonical session, use `get_or_create_canonical_session` instead of `create_session_with_metadata` (which would fail on duplicate PK). The metadata/task_id can be stored on the messages instead.

Note: identity is already loaded at line 286 for skill allowlist — move the load earlier to reuse it, or accept the double-load (identity loading is cheap filesystem I/O).

### Step 5: Update CLI `mika chat` (TUI) session resolution

**File:** `crates/mika-cli/src/commands/chat.rs`

The chat command already loads identity at line 67. Use `resolve_canonical_session_id` to determine the initial session ID instead of `Uuid::new_v4()`.

For `/clear` in `tui/commands/handlers.rs`: when the agent is singleton, `/clear` should NOT create a new session. Instead, it clears the display and optionally runs compaction on the existing session, but the session ID stays the same. Add a `is_singleton_session` flag to `App` state to gate this behavior.

### Step 6: Update HTTP `/send` handler session resolution

**File:** `crates/mika-agent/src/server/handlers.rs`

At line 742, change session creation. The identity is already loaded at `init_agent` time — cache the resolved canonical session ID on `AgentState`:

Add to `AgentState`:
```rust
/// Canonical session ID for singleton agents. None for normal agents.
pub canonical_session_id: Option<String>,
```

Populate in `init_agent` (server/mod.rs:386):
```rust
let identity = crate::prompt::load_identity(agent_home);
let canonical_session_id = crate::prompt::resolve_canonical_session_id(&identity, agent_name);
```

In `run_agent_for_message` (handlers.rs:742):
```rust
let session_id = if let Some(ref canonical) = a.canonical_session_id {
    // Singleton agent: reuse the canonical session (create if absent)
    if let Err(e) = a.db.get_or_create_canonical_session(canonical, a.db.agent_id()).await {
        warn!(error = %e, "failed to create canonical session");
    }
    canonical.clone()
} else {
    let id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = a.db.create_session(&id, a.db.agent_id(), &req.channel).await {
        warn!(error = %e, "failed to create session");
    }
    id
};
```

### Step 7: Make `end_session` a no-op for singleton sessions

**File:** `crates/mika-agent/src/db.rs`

Two approaches, prefer the simpler one:

**Option A (preferred):** Add `end_session_unless_canonical(id, canonical_id)` that skips when `id == canonical_id`. The callers that end sessions (CLI ask exit, TUI `/clear`, silent dispatcher, conversation mode exit) pass the canonical_id when available.

**Option B:** Check the session prefix in `end_session` — skip for `canonical-*` prefix. But this doesn't cover the zero-UUID case without hardcoding.

Go with Option A. The call sites in the CLI have the identity available; the server call sites have `AgentState.canonical_session_id`.

### Step 8: Write migration script for mika-prime

**File:** `scripts/migrate-singleton-sessions.sql` (or `scripts/migrate-mika-prime-sessions.sh`)

A documented operator-runnable script that:
1. Takes `AGENT_ID` and `CANONICAL_SESSION_ID` as parameters
2. For each FK-referencing table, updates `session_id` from old values to canonical
3. Merges messages from old sessions into the canonical session (preserving ordering by `rowid`/timestamp)
4. Deletes old session rows
5. Wraps everything in `PRAGMA foreign_keys = OFF` + transaction
6. Is idempotent

Include a verification query:
```sql
SELECT session_id, COUNT(*) FROM messages
WHERE agent_id = 'mika-prime'
GROUP BY session_id;
-- Expected: exactly one row with canonical session_id
```

### Step 9: Update documentation

**Files:**
- `docs/runtime-structure.md` — document the `[session]` identity.toml section
- `crates/mika-agent/CLAUDE.md` — add session singleton to the Identity struct docs
- Code comments at the session resolution sites explaining compaction interaction (per ticket's "honest purity claim")

### Step 10: Add tests

1. **Unit test for `resolve_canonical_session_id`** — singleton=true with and without canonical_id, singleton=false
2. **Unit test for `get_or_create_canonical_session`** — idempotency (call twice, same row)
3. **Unit test for `end_session` no-op** — verify canonical session's `ended_at` stays NULL
4. **Integration test** — `mika ask` with singleton agent identity, verify session reuse across invocations via DB query

## Acceptance Criteria

1. An agent with `[session] singleton = true` in identity.toml uses the same session for every `mika ask` invocation (without requiring `--session-id`)
2. The canonical session is never ended (pruning-safe)
3. Explicit `--session-id` overrides the singleton behavior
4. mika-prime's canonical_id is the zero-UUID (`00000000-0000-0000-0000-000000000000`)
5. The migration script successfully merges existing mika-prime sessions
6. No changes to compaction, silent mode, callback, or heartbeat session paths
7. `/clear` in TUI for singleton agents clears messages but preserves the session
8. HTTP `/send` handler uses canonical session for singleton agents

## Risk Assessment

**Low risk.** The change is opt-in (default behavior unchanged), affects only conversational session creation (not silent/callback/system paths), and the singleton session is structurally safe against pruning. Compaction already works per-agent regardless of session count. The only data migration is the one-time mika-prime script, which is operator-runnable with verification queries.

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/prompt.rs` | Add `SessionIdentityConfig`, `resolve_canonical_session_id` |
| `crates/mika-agent/src/db.rs` | Add `get_or_create_canonical_session` |
| `crates/mika-agent/src/async_db.rs` | Add async wrapper for canonical session |
| `crates/mika-agent/src/server/state.rs` | Add `canonical_session_id` to `AgentState` |
| `crates/mika-agent/src/server/mod.rs` | Populate canonical_session_id in `init_agent` |
| `crates/mika-agent/src/server/handlers.rs` | Use canonical session in `/send` handler |
| `crates/mika-cli/src/commands/ask.rs` | Use canonical session in `mika ask` |
| `crates/mika-cli/src/commands/chat.rs` | Use canonical session in TUI |
| `crates/mika-cli/src/tui/commands/handlers.rs` | Skip new session on `/clear` for singleton agents |
| `scripts/migrate-singleton-sessions.sql` | One-time migration script |
| `docs/runtime-structure.md` | Document `[session]` section |
| `crates/mika-agent/CLAUDE.md` | Update Identity struct docs |

## Open Questions (resolved in plan)

1. **Should the canonical session use a special `channel_type`?** No — use `'cli'` for CLI-created and `req.channel` for HTTP-created. The singleton concept is orthogonal to channel type. Multiple channels merging into one session is the designed intent for single-surface oracles.

2. **Should `create_session_with_metadata` work for canonical sessions?** The first call creates the session; subsequent calls should not fail. Use `INSERT OR IGNORE` for the session row. Task metadata correlation (from `--task-id`) goes on the message, not the session — the session is shared across invocations.

3. **Should this be a schema migration?** No. No DDL changes are needed — the sessions table already supports arbitrary text IDs. The identity.toml change is configuration, not schema.
