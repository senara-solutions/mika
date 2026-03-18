---
title: "feat: Embed dashboard SPA in mika-server binary"
type: feat
status: active
date: 2026-03-18
issue: https://github.com/senara-solutions/mika/issues/198
---

# Embed Dashboard SPA in mika-server Binary

## Overview

Serve the pre-built React dashboard SPA directly from the mika-server binary using `rust-embed`, eliminating the need for a separate web server or reverse proxy in production. The dashboard is embedded under `/dashboard/*` and controlled by `MIKA_DASHBOARD_ENABLED` env var (default `false`). When disabled, `/dashboard` returns a minimal branded HTML page explaining how to enable it.

Additionally, provide CLI and TUI controls for starting, stopping, and checking the dashboard dev server — making the dashboard a first-class citizen of the Mika developer experience.

## Problem Statement / Motivation

Currently the React observability dashboard runs as a separate Vite dev server (`:5173`) proxied to mika-server. This requires:
- A separate web server or reverse proxy in production
- Manual process management for the dev server
- No visibility into dashboard status from the TUI

This contradicts Mika's single-binary deployment model. Embedding the dashboard aligns API and UI behind one binary, simplifying deployment in Docker containers and Kubernetes.

## Proposed Solution

Three components:

1. **Embedded dashboard (production):** `rust-embed` compiles `dashboard/dist/` into the binary. Served under `/dashboard/*` with SPA fallback. Token injected via `window.__MIKA_CONFIG__` script tag.

2. **CLI `mika dashboard` subcommand:** `start`/`stop`/`status`/`open` for managing the Vite dev server as a background process with PID file tracking.

3. **TUI integration:** Status indicator dot in header bar + `/dashboard` slash commands.

## Technical Approach

### Architecture Decisions

**Token injection:** Server rewrites `index.html` at serve time, injecting a `<script>window.__MIKA_CONFIG__ = { token: "...", basePath: "/dashboard" };</script>` tag before the closing `</head>`. The SPA's `client.ts` reads `window.__MIKA_CONFIG__?.token` first, falling back to `import.meta.env.VITE_MIKA_DASHBOARD_TOKEN` (backwards compatible for dev mode).

**Which token is injected:** `MIKA_DASHBOARD_TOKEN` if set, otherwise `MIKA_INTERNAL_TOKEN` with a startup warning logged. This preserves the two-tier auth model — dashboard token gives read-only access. If only `MIKA_INTERNAL_TOKEN` exists, the embedded dashboard gets superuser access (acceptable for single-user deployments).

**Vite base path:** `vite.config.ts` reads `VITE_BASE_PATH` env var (default `/`). For embedded builds: `VITE_BASE_PATH=/dashboard/ npm run build --prefix dashboard`. React Router's `basename` reads from the same `window.__MIKA_CONFIG__.basePath` at runtime (or falls back to `/`).

**CORS:** When served from same origin (embedded mode), CORS is irrelevant. The existing CORS configuration for dev mode (`localhost:5173`) remains unchanged. No conditional logic needed.

**SPA fallback strategy:** Axum handler checks if the requested path matches an embedded static file. If yes, serve it with correct Content-Type. If no, serve `index.html` (with token injection). This ensures deep-links like `/dashboard/sessions/abc` work.

**Build sequencing:** Manual. `npm run build --prefix dashboard` before `cargo build`. NO build.rs coupling (per user feedback in MEMORY.md). Dockerfile handles this via a Node.js builder stage. If `dashboard/dist/` doesn't exist at compile time, `rust-embed` embeds zero files — `/dashboard/` returns a "not built" message.

### Implementation Phases

#### Phase 1: Embedded Dashboard Server (`crates/mika-agent`)

**1.1 Add `rust-embed` dependency**

`Cargo.toml` (workspace root):
```toml
[workspace.dependencies]
rust-embed = "8"
```

`crates/mika-agent/Cargo.toml`:
```toml
rust-embed.workspace = true
```

**1.2 Add `MIKA_DASHBOARD_ENABLED` config**

`crates/mika-common/src/config.rs`:
- Add to `CONFIG_KEYS` array:
  ```rust
  ConfigKeyInfo {
      key: "dashboard_enabled",
      backend: ConfigBackend::File,
      env_var: Some("MIKA_DASHBOARD_ENABLED"),
      secret: false,
      description: "Enable embedded dashboard SPA at /dashboard/ (default: false)",
  }
  ```
- Add field to `Settings` struct: `pub dashboard_enabled: bool` with `#[serde(default)]`
- Add arm to `get_effective_value` match
- Add to `.env.example` near existing dashboard vars

**1.3 Create embedded dashboard module**

New file: `crates/mika-agent/src/server/embedded_dashboard.rs`

```rust
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../dashboard/dist/"]
struct DashboardAssets;
```

Key functions:
- `dashboard_routes(settings: &Settings) -> Router<AppState>` — returns the `/dashboard` route group
- `serve_dashboard_asset(path, state)` — serves embedded file or SPA fallback
- `inject_config_into_html(html, token, base_path)` — rewrites `index.html` with `window.__MIKA_CONFIG__`
- `disabled_page()` — returns branded HTML page explaining how to enable
- `not_built_page()` — returns HTML page when dist/ was empty at compile time

The disabled page: dark theme (#0a0a0a bg, #e5e5e5 text), system font stack, Mika branding, shows `MIKA_DASHBOARD_ENABLED=true` snippet.

**1.4 Wire routes in server/mod.rs**

In `build_router()`:
- Import `embedded_dashboard::dashboard_routes`
- Add `.nest("/dashboard", dashboard_routes(&settings))` to the router
- Place AFTER mutation/dashboard API routes, BEFORE `.route("/health", ...)`
- Dashboard routes do NOT go through auth middleware (static asset serving is public; the SPA authenticates its own API calls)

**1.5 Content-Type handling**

Serve files with correct MIME types based on file extension:
- `.html` → `text/html; charset=utf-8`
- `.js` → `application/javascript`
- `.css` → `text/css`
- `.json` → `application/json`
- `.svg` → `image/svg+xml`
- `.png`/`.jpg`/`.gif`/`.webp` → `image/*`
- `.woff2`/`.woff` → `font/*`
- Default → `application/octet-stream`

#### Phase 2: Dashboard SPA Changes (`dashboard/`)

**2.1 Configurable Vite base path**

`dashboard/vite.config.ts`:
```ts
export default defineConfig({
  base: process.env.VITE_BASE_PATH || '/',
  // ... existing config
})
```

**2.2 Runtime token acquisition**

`dashboard/src/api/client.ts`:
```ts
function getToken(): string {
  // Runtime injection (embedded mode)
  if (window.__MIKA_CONFIG__?.token) {
    return window.__MIKA_CONFIG__.token;
  }
  // Build-time env var (dev mode)
  return import.meta.env.VITE_MIKA_DASHBOARD_TOKEN || '';
}
```

Add TypeScript type declaration for `window.__MIKA_CONFIG__`.

**2.3 Dynamic BrowserRouter basename**

`dashboard/src/main.tsx`:
```tsx
const basePath = window.__MIKA_CONFIG__?.basePath || '/';
<BrowserRouter basename={basePath}>
```

#### Phase 3: CLI Dashboard Management (`crates/mika-cli`)

**3.1 Add `Dashboard` subcommand to cli.rs**

```rust
/// Manage the dashboard dev server
Dashboard(DashboardArgs),
```

```rust
#[derive(Args)]
pub struct DashboardArgs {
    #[command(subcommand)]
    pub command: DashboardCommand,
}

#[derive(Subcommand)]
pub enum DashboardCommand {
    /// Start the dashboard dev server
    Start,
    /// Stop the dashboard dev server
    Stop,
    /// Show dashboard status
    Status,
    /// Open dashboard in browser
    Open,
}
```

Update `agent_override()` and `team_override()` exhaustive matches.

**3.2 Create commands/dashboard.rs**

```rust
pub async fn run(command: DashboardCommand) -> Result<()>
```

Sub-functions:
- `start()`: Check PID file → check liveness → resolve project root → check `npm` in PATH → spawn `npm run dev:dashboard` with `VITE_MIKA_DASHBOARD_TOKEN` from config → write PID → print URL
- `stop()`: Read PID → check liveness → send SIGTERM → wait (up to 5s) → remove PID file
- `status()`: Read PID → check liveness → print running/stopped with URL and PID
- `open()`: Determine URL → `open::that(url)` (use `std::process::Command` with platform-specific `xdg-open`/`open`/`start`)

PID file: `~/.mika/dashboard.pid` (global, not per-agent).
Stale PID handling: On `start`, if PID file exists but process is dead, remove stale file and proceed.
Process liveness: `kill(pid, 0)` via `libc::kill` or `/proc/{pid}` check.

**3.3 Update main.rs dispatch**

```rust
Some(Commands::Dashboard(args)) => commands::dashboard::run(args.command).await,
```

**3.4 Update commands/mod.rs**

```rust
pub mod dashboard;
```

#### Phase 4: TUI Dashboard Integration (`crates/mika-cli`)

**4.1 Add dashboard state to App**

`tui/app.rs`:
```rust
/// Whether the dashboard dev server is running
pub dashboard_running: bool,
```

Initialize to `false` in `App::new()`. Add `check_dashboard_status()` method that reads PID file and checks liveness.

**4.2 Poll dashboard status**

In the existing tick handler (same location as `POLL_INTERVAL_TICKS`), add:
```rust
if self.tick_count % POLL_INTERVAL_TICKS == 0 {
    self.dashboard_running = check_dashboard_running();
}
```

`check_dashboard_running()` is a sync function (reads PID file, checks `/proc/{pid}`). It's fast enough for inline tick handling.

**4.3 Add status dot to header bar**

`tui/ui.rs` in `draw_header()`:
```rust
// After session info, before time
Span::raw("   "),
Span::styled(
    if app.dashboard_running { "●" } else { "●" },
    Style::default().fg(if app.dashboard_running { Color::Green } else { Color::Red }),
),
Span::styled(" Dashboard", Style::default().fg(Color::DarkGray)),
```

Add to BOTH branches (team mode and normal mode).

**4.4 Add `/dashboard` slash command**

`tui/commands/mod.rs` — add to `COMMANDS` array:
```rust
SlashCommand {
    name: "dashboard",
    aliases: &[],
    description: "Manage dashboard dev server (start/stop/status)",
    args_hint: Some("[start|stop|status]"),
    completer: Some(dashboard_completer),
},
```

`tui/commands/handlers.rs` — add handler:
```rust
"dashboard" => Some(handle_dashboard(app, args).await),
```

`handle_dashboard` dispatches:
- No args or empty → toggle (start if stopped, stop if running)
- `"start"` → start dev server, update `app.dashboard_running`
- `"stop"` → stop dev server, update `app.dashboard_running`
- `"status"` → show status message in chat area

#### Phase 5: Docker and Build Pipeline

**5.1 Update Dockerfile.agent**

Add Node.js builder stage before Rust builder:
```dockerfile
# Stage 1: Build dashboard SPA
FROM node:22-slim AS dashboard-builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY packages/ packages/
COPY dashboard/ dashboard/
RUN npm ci --ignore-scripts && npm run build --prefix dashboard

# Stage 2: Build Rust binary (existing, with dashboard assets)
FROM rust:1.93-slim AS builder
# ... existing setup ...
COPY --from=dashboard-builder /app/dashboard/dist dashboard/dist
# ... existing cargo build ...
```

**5.2 Update .env.example**

Add near existing dashboard vars:
```bash
# MIKA_DASHBOARD_ENABLED=true  # Enable embedded dashboard at /dashboard/ (default: false)
```

## System-Wide Impact

- **Interaction graph:** `/dashboard/*` route → `embedded_dashboard::serve_dashboard_asset()` → `rust-embed` file lookup → Content-Type detection → optional `index.html` injection. No interaction with agent loop, tools, or database.
- **Error propagation:** Missing files → SPA fallback to `index.html`. Missing `dashboard/dist/` at compile time → "not built" page. Missing token → SPA loads but API calls return 401.
- **State lifecycle risks:** None. Dashboard serving is stateless. PID file is the only persistent state for CLI management, with stale detection.
- **API surface parity:** No changes to existing API routes. New `/dashboard/*` routes are additive.
- **Binary size impact:** Dashboard dist is typically 1-3MB. `rust-embed` embeds files as-is. Acceptable for the deployment simplification gained.

## Acceptance Criteria

### Functional Requirements

- [ ] `MIKA_DASHBOARD_ENABLED=true` with built dashboard → `/dashboard/` serves React SPA
- [ ] `MIKA_DASHBOARD_ENABLED=false` (default) → `/dashboard/` returns branded disabled page
- [ ] Dashboard not built (empty dist/) → `/dashboard/` returns "not built" instructions page
- [ ] SPA deep-links work (e.g., `/dashboard/sessions/abc` → index.html with correct routing)
- [ ] Token injected via `window.__MIKA_CONFIG__` script tag in embedded mode
- [ ] Dashboard SPA authenticates API calls using injected or build-time token
- [ ] `mika dashboard start` spawns Vite dev server, writes PID
- [ ] `mika dashboard stop` sends SIGTERM, cleans PID file
- [ ] `mika dashboard status` shows running/stopped with URL
- [ ] `mika dashboard open` opens browser
- [ ] Stale PID files detected and cleaned on `start`
- [ ] TUI header shows green dot when dashboard running, red when not
- [ ] `/dashboard`, `/dashboard start`, `/dashboard stop`, `/dashboard status` work in TUI
- [ ] `VITE_BASE_PATH=/dashboard/` produces correctly pathed assets
- [ ] Dev mode (`npm run dev:dashboard`) continues to work with `/` base path

### Non-Functional Requirements

- [ ] `cargo build --release --features telemetry` succeeds
- [ ] `cargo clippy` passes
- [ ] All existing tests pass (`cargo test`)
- [ ] No new external crates beyond `rust-embed`
- [ ] No build.rs coupling to Node.js

## Files to Modify

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add `rust-embed = "8"` to workspace deps |
| `crates/mika-agent/Cargo.toml` | Add `rust-embed.workspace = true` |
| `crates/mika-common/src/config.rs` | Add `MIKA_DASHBOARD_ENABLED` config key + Settings field |
| `crates/mika-agent/src/server/embedded_dashboard.rs` | **New:** rust-embed struct, SPA handler, token injection, disabled/not-built pages |
| `crates/mika-agent/src/server/mod.rs` | Wire `/dashboard` route group in `build_router()` |
| `dashboard/vite.config.ts` | Add `VITE_BASE_PATH` env var for configurable `base` |
| `dashboard/src/api/client.ts` | Add `window.__MIKA_CONFIG__` token fallback |
| `dashboard/src/main.tsx` | Dynamic `BrowserRouter basename` from `window.__MIKA_CONFIG__` |
| `dashboard/src/vite-env.d.ts` | Add `Window.__MIKA_CONFIG__` type declaration |
| `crates/mika-cli/src/cli.rs` | Add `Dashboard(DashboardArgs)` subcommand + exhaustive match updates |
| `crates/mika-cli/src/commands/mod.rs` | Add `pub mod dashboard;` |
| `crates/mika-cli/src/commands/dashboard.rs` | **New:** PID management, process spawn/kill, status, browser open |
| `crates/mika-cli/src/main.rs` | Add `Dashboard` dispatch |
| `crates/mika-cli/src/tui/app.rs` | Add `dashboard_running: bool` + polling logic |
| `crates/mika-cli/src/tui/ui.rs` | Add dashboard status dot to header bar |
| `crates/mika-cli/src/tui/commands/mod.rs` | Add `/dashboard` slash command definition |
| `crates/mika-cli/src/tui/commands/handlers.rs` | Add `/dashboard` handler dispatch |
| `Dockerfile.agent` | Add Node.js builder stage for dashboard |
| `.env.example` | Add `MIKA_DASHBOARD_ENABLED` documentation |

## Dependencies & Risks

- **Risk:** Users build from source without running `npm build` first → empty dashboard. Mitigation: startup warning log when `MIKA_DASHBOARD_ENABLED=true` but no files embedded.
- **Risk:** Google Fonts CDN dependency in embedded mode. Mitigation: system font fallback already in CSS; purely cosmetic degradation.
- **Risk:** Binary size increase (~1-3MB). Mitigation: acceptable for the deployment benefit.
- **Dependency:** Node.js 22 in Docker builder stage. Already used for dashboard development.

## Sources & References

- Related issue: #198
- Config pattern: `crates/mika-common/src/config.rs` (ConfigKeyInfo registry)
- Server router: `crates/mika-agent/src/server/mod.rs` (build_router)
- CLI pattern: `crates/mika-cli/src/cli.rs` (Commands enum)
- TUI header: `crates/mika-cli/src/tui/ui.rs` (draw_header)
- TUI commands: `crates/mika-cli/src/tui/commands/mod.rs` (COMMANDS array)
- Build tooling feedback: `~/.claude/projects/.../memory/feedback_build_tooling.md`
- Shared UI package: `docs/solutions/architecture-patterns/extract-shared-ui-package.md`
