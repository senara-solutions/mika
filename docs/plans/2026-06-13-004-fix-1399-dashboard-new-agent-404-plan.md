---
ticket: mika#1399
branch: fix/1399/dashboard-new-agent-404
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/1399
execution: code
---

# Plan: dashboard 404 on agents created while server is running (mika#1399)

## Problem frame

`mika-spirit` builds `AppState.agents: Arc<HashMap<String, Arc<AgentState>>>` once at startup (`server/mod.rs:666` populates from `agent::list_agents()`). Agents born after boot — created on disk via `~/.mika/agents/<id>/` and exercised by `mika ask --agent <id>` — are invisible to the dashboard's agent-detail page (`dashboard.rs:152` `handle_agent_detail` → `state.resolve_agent` → `None` → 404).

The data is fully present: DB row, sessions, llm_calls, identity.toml, soul.md. Only the running server's in-memory view is stale. Workaround: restart. Bites Mika Prime and any claude-pilot-spawned agent.

## Approach: lazy-insert on resolve-miss (Option 1)

Per first-pass groom guidance: Option 1 (lazy-insert) is the architecturally correct fix — it handles all entry paths (any code path that calls `resolve_agent` benefits, including out-of-band dir creation). Option 2 (event-driven refresh on create) is incomplete (misses non-tool-mediated creates). Option 3 (DB-fallback) degrades the page.

Lazy-insert requires turning the currently-immutable map into a concurrent structure. **Use `DashMap` instead of `RwLock<HashMap>`** — the codebase already uses `DashMap` for similar shared-mutable patterns (`pr_reviews_posted: Arc<DashMap<String, HashSet<String>>>` on `AppState`). Lower friction than `RwLock<HashMap>` (no read-guard / write-guard plumbing through callers), explicitly designed for read-heavy concurrent access.

## Scope boundaries

- Change `AppState.agents` type from `Arc<HashMap<String, Arc<AgentState>>>` to `Arc<DashMap<String, Arc<AgentState>>>`.
- Update `AppState::resolve_agent` to:
  1. Look up in the map (fast path).
  2. On miss: check if the agent exists on disk (`~/.mika/agents/<id>/identity.toml`) AND has a DB row. If yes, lazy-construct `AgentState` via the existing `init_agent` factory and insert.
  3. Return `Option<Arc<AgentState>>` (unchanged contract).
- Update all readers (callers of `state.agents.iter()`, `.get()`, etc.) — DashMap's API is mostly drop-in but iteration semantics differ.
- **Out of scope:** event-driven refresh on `mika ask` create (the lazy-insert path subsumes this); removing agents from the map when their dir is deleted (separate concern — orphan task is mika#1436); rebuilding domain KG / restart-only side effects when a new agent appears.

## Implementation Units

### U1 — DashMap migration

**Goal:** `AppState.agents` is `Arc<DashMap<String, Arc<AgentState>>>`.

**Files:**
- Modify: `crates/mika-agent/src/server/state.rs` (the field type at ~line 64)
- Modify: `crates/mika-agent/src/server/mod.rs` (population site at ~line 666; final assignment to `AppState`)
- Modify: All readers — `grep -rn "state.agents\|app.agents" crates/mika-agent/src/server/` enumerates them

**Approach:**

1. Change the field type:
   ```rust
   pub agents: Arc<DashMap<String, Arc<AgentState>>>,
   ```
2. Population site builds the DashMap:
   ```rust
   let agents: DashMap<String, Arc<AgentState>> = DashMap::new();
   for (name, state) in discovered { agents.insert(name, Arc::new(state)); }
   let agents = Arc::new(agents);
   ```
3. Reader updates: DashMap's `.get(k)` returns a `Ref<K, V>` (read guard) instead of `Option<&V>`. Most call sites use the `Arc<AgentState>` clone, so the pattern becomes:
   ```rust
   state.agents.get(name).map(|r| r.value().clone())
   ```
   `.iter()` returns ref-guard iterators; if any callers hold across `.await`, that needs explicit conversion to owned data (collect first, then iterate). Per `Send`-safety convention.

**Test scenarios:**
- **Existing tests pass:** all server tests that exercise agent resolution continue to pass with the new field type.
- **Iter call sites don't hold across await:** verify via grep + `cargo clippy` (DashMap ref-guards are not `Send` — clippy will catch held-across-await cases).

**Verification:** `cargo build -p mika-agent` clean; `cargo test -p mika-agent` passes.

### U2 — Lazy-insert in `resolve_agent`

**Goal:** `resolve_agent(name)` returns `Some(Arc<AgentState>)` if the agent exists on disk + DB but isn't yet in the map.

**Files:**
- Modify: `crates/mika-agent/src/server/state.rs` (around `resolve_agent` at ~line 103)

**Approach:**

```rust
pub async fn resolve_agent(&self, name: &str) -> Option<Arc<AgentState>> {
    let effective = if name.is_empty() { &self.default_agent } else { name };

    // Fast path: in-map
    if let Some(r) = self.agents.get(effective) {
        return Some(r.value().clone());
    }

    // Slow path: lazy-construct from disk + DB
    let agent_home = self.global_home_dir.join("agents").join(effective);
    if !agent_home.join("identity.toml").exists() {
        return None;
    }
    if self.dashboard_db.get_agent(effective).await.ok().flatten().is_none() {
        return None;
    }

    // Construct via the existing factory; same parameters as startup-time init.
    let agent_state = match init_agent_state_for_lazy_path(self, effective, &agent_home).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::warn!(agent = effective, error = %e, "lazy agent construction failed");
            return None;
        }
    };

    self.agents.insert(effective.to_string(), agent_state.clone());
    Some(agent_state)
}
```

**Constraint:** `resolve_agent` becomes `async`. This is a contract change for all callers. Audit via `grep -rn "resolve_agent" crates/mika-agent/src/server/`. The existing callers are HTTP handlers (already `async`), so this is mechanical.

`init_agent_state_for_lazy_path` is a thin wrapper around the existing `init_agent` factory at `server/mod.rs:360`, threading the parameters the factory needs (tool_registry, gateway_url, github_token, settings, etc.) from `AppState`. Implementer extracts the construction logic into a callable from both startup and lazy paths.

**Test scenarios:**
- **Resolve existing in-map agent:** fast path returns the cached `Arc<AgentState>` (no DB or filesystem hits).
- **Resolve new disk-and-DB agent:** slow path constructs, inserts, returns. Second resolve hits the fast path.
- **Resolve non-existent agent:** returns `None`. No spurious insert.
- **Resolve agent with disk but no DB row:** returns `None`. (Agent exists in `~/.mika/agents/` but was never exercised; the dashboard 404 is correct in this case — same UX, no data to show.)
- **Race on lazy-construct:** two concurrent `resolve_agent` calls for the same new agent. DashMap's atomic insert ensures one wins; the other reads the inserted value. Verify via concurrent-call test.

**Verification:** unit tests + integration test against a live mika-spirit (spawn server, create agent via `mika ask`, hit dashboard agent-detail, expect 200).

### U3 — Update `handle_agent_detail` + sibling handlers

**Goal:** Handlers `.await` the now-async `resolve_agent`.

**Files:**
- Modify: `crates/mika-agent/src/server/dashboard.rs` (line 157, 333 per grep)
- Modify: any other handler calling `resolve_agent`

**Approach:** Add `.await`:

```rust
let agent_state = match state.resolve_agent(&agent_id).await {
    Some(a) => a,
    None => { ... 404 ... }
};
```

**Test scenarios:** existing tests pass; handlers still return correct 200/404 for valid/invalid agents.

**Verification:** `cargo test -p mika-agent server::dashboard::tests` passes.

### U4 — Factory extraction for lazy construction

**Goal:** `init_agent`'s body is callable from both startup and the lazy-resolve path.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` (around `init_agent` at line 360)

**Approach:** Extract the AgentState-building logic from `init_agent` into a helper `build_agent_state(agent_name, agent_home, ctx: &AgentInitCtx) -> Result<AgentState>` where `AgentInitCtx` bundles the shared parameters (tool_registry, gateway_url, github_token, settings, etc.).

The startup path calls it inside its for-loop; the lazy path calls it from `AppState::resolve_agent`. `AppState` can construct `AgentInitCtx` from its own fields.

**Test scenarios:** startup behavior unchanged (build the same way it did before extraction); lazy path uses the same factory.

**Verification:** smoke test the startup path on a multi-agent host.

### U5 — Docs + audit-log

**Goal:** Document the lazy-construction behavior; add an audit log line on successful lazy insert (so operators can see when this fires).

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § HTTP Server (mika-spirit) — add a note on the lazy-resolution behavior
- Modify: `crates/mika-agent/src/server/state.rs` — `tracing::info!` on successful lazy insert (`agent_resolved_lazily` event with `agent_id` field)

**Approach:** Short additions; helps debugging when the dashboard "magically" finds a new agent.

**Verification:** manual read + log grep post-deploy.

## Dependencies / sequencing

- U1 → U2 (U2 uses DashMap from U1)
- U1 → U3 (U3 updates async callers; needs U1's type and U2's signature)
- U4 can happen in parallel with U1/U2 — extracting the factory is independent
- U5 ships in same PR; last

## Patterns to follow (cross-cutting)

- `pr_reviews_posted: Arc<DashMap<String, HashSet<String>>>` (existing AppState field) — DashMap usage pattern
- `server::init_agent` at `mod.rs:360` — existing factory shape
- `state.resolve_agent` at `state.rs:103` — existing signature (becomes async)

## Verification (top-level)

- `cargo test -p mika-agent` passes
- `cargo clippy --workspace` clean (DashMap ref-guard held-across-await checks)
- `cargo fmt --all -- --check` clean
- Smoke test:
  1. Start mika-spirit
  2. Create new agent on disk + exercise via `mika ask --agent new-agent ...`
  3. Open `http://localhost:8081/dashboard/agents/new-agent` — page loads (was 404 pre-fix)
  4. `grep agent_resolved_lazily` in server log — sees the lazy-insert event
  5. Refresh page — fast path hit (no second `agent_resolved_lazily` event)

## Risk / known unknowns

- **DashMap ref-guard held across await.** DashMap's `Ref` is `!Send`. If any current caller holds the ref across a `.await`, compilation fails (clippy/rustc catches it). Resolution: clone the `Arc<AgentState>` out of the ref immediately and drop the guard before awaiting. The `.get(name).map(|r| r.value().clone())` pattern is the idiom.
- **Lazy-construct cost on first resolve.** `init_agent` reads identity.toml + soul.md, opens a scoped DB handle, resolves KG config. ~tens of milliseconds. First dashboard load of a new agent pays this cost once; subsequent loads hit the fast path. Acceptable for a p3 UX fix.
- **`init_agent` failure modes during lazy construction.** If identity.toml is malformed or the agent dir is incomplete, construction fails. The slow path logs a `warn!` and returns `None` — same UX as the original 404, but operator now has a log line for diagnosis.
- **Stale entries.** Agents whose dir is deleted while server is running remain in the map (orphans). Out of scope for this ticket — orphan removal is mika#1436 territory.

## Out-of-scope (explicit)

- Removing stale agents from the map when their dir is deleted (mika#1436).
- Event-driven refresh on `mika ask`-mediated create (lazy-insert subsumes the need).
- Rebuilding domain KG on agent creation (separate startup-only side effect; lazy-resolved agents don't get KG until next restart, which is fine because mika-arch is the sole KG consumer per CLAUDE.md § KG topology #800).
- Dashboard UI changes (server-side fix only).
