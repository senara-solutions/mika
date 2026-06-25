# Plan — feat(agent-core): per-agent heartbeat opt-out (mika#1404)

## Goal

Add a per-agent `[heartbeat] enabled` config in `identity.toml` that gates heartbeat task registration, mirroring the existing reflection opt-out pattern. Agents with `enabled = false` register no heartbeat task, and any stale heartbeat task is cancelled on startup. Default is `true` for backward compatibility.

## Files

**Modify**

- `crates/mika-agent/src/prompt.rs` — Add `HeartbeatConfig` struct and `heartbeat` field on `Identity`
- `crates/mika-agent/src/task_engine/mod.rs` — Add `heartbeat_enabled_for_agent()` accessor function
- `crates/mika-cli/src/commands/chat.rs` — Gate heartbeat registration on the new accessor; cancel stale task when disabled
- `crates/mika-agent/src/server/mod.rs` — Same gating in the server startup loop

**No new files required.**

## Design

### 1. Identity config (`prompt.rs`)

Add a `HeartbeatConfig` struct mirroring `ReflectionConfig`'s enabled-flag shape, but simpler (heartbeat has no time/timezone/notify fields):

```rust
/// Configuration for periodic heartbeat task.
#[derive(Debug, Deserialize, Clone)]
pub struct HeartbeatConfig {
    #[serde(default = "default_heartbeat_enabled")]
    pub enabled: bool,
}

fn default_heartbeat_enabled() -> bool {
    true // Back-compat: heartbeat is enabled by default
}
```

No `Default` impl is defined on `HeartbeatConfig`. The `#[serde(default = "default_heartbeat_enabled")]` on the `enabled` field handles the only scenario where a serde default is needed: when the `[heartbeat]` section is present but `enabled` is omitted. When the entire `[heartbeat]` section is absent, the `Option<HeartbeatConfig>` field is `None` and the accessor's `unwrap_or(true)` provides the enabled-by-default behavior. This avoids a dual-default-path where two mechanisms produce the same result through different code paths (review-guide.md § KISS).

Add `heartbeat: Option<HeartbeatConfig>` field to the `Identity` struct with `#[serde(default)]`.

**Key difference from reflection:** Reflection defaults to `enabled: false` (opt-in). Heartbeat defaults to `enabled: true` (opt-out). This preserves current behavior — agents without a `[heartbeat]` block keep getting heartbeat tasks.

**Semantics of `Option<HeartbeatConfig>` — single default path per case:**
- `None` (no `[heartbeat]` section) → accessor's `unwrap_or(true)` → enabled (default behavior, backward-compatible)
- `Some(HeartbeatConfig { enabled: true })` (section present, key present) → enabled (explicit opt-in)
- `Some(HeartbeatConfig { enabled: true })` (section present, key omitted) → `default_heartbeat_enabled()` fires → enabled
- `Some(HeartbeatConfig { enabled: false })` → disabled (opt-out)

### 2. Accessor function (`task_engine/mod.rs`)

Add a public function mirroring `reflection_cron_for_agent()` but returning a simple `bool`:

```rust
/// Check if heartbeat is enabled for the agent from identity.toml config.
/// Returns `true` (default) unless `[heartbeat] enabled = false`.
pub async fn heartbeat_enabled_for_agent(home_dir: &Path) -> bool {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    identity.heartbeat.as_ref().map(|c| c.enabled).unwrap_or(true)
}
```

Loads `identity.toml` from disk on each call, matching the existing `reflection_cron_for_agent()` I/O pattern (review-guide.md § DRY — conscious mirror, not an oversight). This is simpler than the reflection accessor because heartbeat doesn't need timezone/time conversion — it always uses the fixed `0 0 * * * *` cron. The function returns `bool` rather than `Option<String>` since the cron expression is constant.

### 3. Chat path gating (`chat.rs`)

Replace the unconditional heartbeat registration (lines ~181-187) with the same conditional pattern used for reflection (lines ~188-202):

```rust
if task_engine::heartbeat_enabled_for_agent(&ctx.home_dir).await {
    task_engine::ensure_recurring_task(
        &ctx.async_db,
        "heartbeat",
        "0 0 * * * *",
        r#"{"trigger":"heartbeat"}"#,
    )
    .await;
} else if let Err(e) = ctx
    .async_db
    .cancel_recurring_task_by_label("heartbeat")
    .await
{
    tracing::warn!(error = %e, "failed to cancel stale heartbeat task");
}
```

### 4. Server path gating (`server/mod.rs`)

Same transformation at lines ~1330-1336. Replace the unconditional `ensure_recurring_task` call with the conditional pattern:

```rust
if task_engine::heartbeat_enabled_for_agent(&agent_state.home_dir).await {
    task_engine::ensure_recurring_task(
        &db,
        "heartbeat",
        HEARTBEAT_CRON,
        r#"{"trigger":"heartbeat"}"#,
    )
    .await;
} else if let Err(e) = db.cancel_recurring_task_by_label("heartbeat").await {
    warn!(agent = %agent_name, error = %e, "failed to cancel stale heartbeat task");
}
```

## Testing

### Unit tests

1. **`prompt.rs` — deserialization tests:**
   - `heartbeat_config_defaults_to_enabled` — `Identity` with no `[heartbeat]` block → `heartbeat` is `None`, and `unwrap_or(true)` yields `true`
   - `heartbeat_config_explicit_false` — `[heartbeat]\nenabled = false` → `heartbeat` is `Some(HeartbeatConfig { enabled: false })`
   - `heartbeat_config_explicit_true` — `[heartbeat]\nenabled = true` → `Some(HeartbeatConfig { enabled: true })`

2. **`task_engine/mod.rs` — accessor logic:** The accessor is a thin wrapper over identity loading; deserialization tests above cover the logic. No separate test needed beyond confirming the `unwrap_or(true)` default.

### Integration tests

No eval harness scenario needed — this is a config-gating feature with no LLM interaction. The existing heartbeat eval scenarios continue to exercise the enabled-by-default path.

## Scope boundaries

- **No schema migration.** This is purely a config/identity change — no SQLite DDL.
- **No well-known agent identity changes in this PR.** Mika Prime's `identity.toml` will set `[heartbeat] enabled = false` separately (either manually or via a follow-up that provisions her identity). The ticket's AC says "Mika Prime sets `enabled = false`" — this is an operator action on her `identity.toml`, not a code change in `well_known_agents.rs` (Mika Prime is not a well-known agent provisioned by `dev_mode`).
- **No `HEARTBEAT_CRON` changes.** The cron expression stays `0 0 * * * *`.
- **No dashboard/API changes.** The heartbeat config is not surfaced in any API endpoint.

## Revision history

- rev 2 (2026-06-26): addressed F1 by dropping the `Default` impl on `HeartbeatConfig` and documenting the single-default-path-per-case semantics (§ KISS — avoid dual mechanisms that produce the same result through different paths); addressed F2 by adding explicit note in Section 2 that the accessor's disk-read pattern is a conscious mirror of `reflection_cron_for_agent()` (§ DRY).
