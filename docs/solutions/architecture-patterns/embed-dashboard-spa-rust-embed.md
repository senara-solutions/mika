---
title: Embed React dashboard SPA in Rust binary with rust-embed
category: architecture-patterns
date: 2026-03-18
tags: [rust-embed, axum, dashboard, SPA, vite, docker, cli, tui]
module: mika-agent/server, mika-cli, dashboard
issue: "#198"
---

# Embed React Dashboard SPA in Rust Binary

## Problem

The React observability dashboard ran as a separate Vite dev server on `:5173`, requiring a reverse proxy or separate process in production. This contradicts the single-binary deployment model where one container serves both API and UI.

## Solution

Use `rust-embed` to compile `dashboard/dist/` into the mika-server binary at build time. Serve the SPA at `/dashboard/*` with an Axum fallback handler that supports client-side routing.

### Key Components

**1. rust-embed struct (crates/mika-agent/src/server/embedded_dashboard.rs)**

```rust
#[derive(Embed)]
#[folder = "../../dashboard/dist/"]
struct DashboardAssets;
```

- If `dashboard/dist/` is empty at compile time, zero files are embedded (graceful degradation)
- The folder must exist — add `dashboard/dist/.gitkeep` tracked via `.gitignore` exception
- `DashboardAssets::iter()` and `DashboardAssets::get()` are trait methods from `rust_embed::Embed` — the trait must be in scope

**2. Token injection via serde_json (not string interpolation)**

```rust
let config = serde_json::json!({ "token": token, "basePath": "/dashboard" });
let json = serde_json::to_string(&config).unwrap_or_default();
let safe_json = json
    .replace('&', "\\u0026")
    .replace('<', "\\u003c")
    .replace('>',', "\\u003e");
let script = format!("<script>window.__MIKA_CONFIG__={safe_json};</script>");
```

Never use manual string escaping (`replace('"', "\\\"")`) for embedding tokens in HTML `<script>` tags — a token containing `</script>` would break out of the script context. Always use `serde_json` for JSON serialization, then escape `<`, `>`, `&` as Unicode escapes for HTML safety.

**3. Never fall back to the superuser token**

If `MIKA_DASHBOARD_TOKEN` is not set, show a "token not configured" page instead of injecting `MIKA_INTERNAL_TOKEN`. The internal token grants mutation access (message injection, task completion) — leaking it in publicly-served HTML is a security risk.

**4. Vite base path coordination**

Both Vite's `base` config and React Router's `basename` must be set to `/dashboard/` for embedded mode:

- `vite.config.ts`: `base: process.env.VITE_BASE_PATH || '/'`
- `main.tsx`: `<BrowserRouter basename={window.__MIKA_CONFIG__?.basePath || '/'}>`
- Docker build: `VITE_BASE_PATH=/dashboard/ npm run build --prefix dashboard`

Missing the `VITE_BASE_PATH` causes all asset references to use `/assets/...` instead of `/dashboard/assets/...`, resulting in 404s.

**5. Route placement in Axum**

```rust
mutation_routes
    .nest("/api/v1", dashboard_api_routes)
    .nest("/dashboard", embedded_dashboard::dashboard_routes())  // No auth layer
    .route("/health", get(handle_health))
```

Dashboard routes go outside auth middleware — static assets are publicly served. The SPA authenticates its own API calls using the injected token. Add `Cache-Control: no-store` to `index.html` responses and `Cache-Control: public, max-age=31536000, immutable` to hashed static assets.

**6. CLI/TUI dashboard management**

Extract shared functions (`start_dashboard_process()`, `stop_dashboard()`, `is_dashboard_running()`) in a single module. Both CLI subcommands and TUI slash commands call the same functions — never duplicate process management logic.

Process liveness: use `kill -0 <pid>` (via `Command::new("kill").arg("-0")`) for POSIX portability. Do NOT use `/proc/{pid}` which is Linux-only.

### Docker Multi-Stage Build

```dockerfile
FROM node:22-slim AS dashboard-builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY packages/ packages/
COPY dashboard/ dashboard/
RUN npm ci --ignore-scripts && VITE_BASE_PATH=/dashboard/ npm run build --prefix dashboard

FROM rust:1.93-slim AS builder
# ... existing setup ...
COPY --from=dashboard-builder /app/dashboard/dist dashboard/dist
```

## Prevention

- **Build sequencing**: Never use `build.rs` to invoke npm/vite. Keep Rust and Node.js build systems decoupled. Dockerfile handles sequencing naturally.
- **Token security**: Always use `serde_json` for embedding data in `<script>` tags. Never use string interpolation for HTML contexts.
- **Cross-platform**: Avoid Linux-specific APIs (`/proc`, `libc::kill`) in CLI code that ships on macOS. Use portable alternatives.
- **API deduplication**: When CLI and TUI need the same functionality, extract shared functions immediately — don't create `_pub` wrapper variants of private functions.

## Related

- [Build tooling separation feedback](../../../.claude/projects/-data-workspace-senara-solutions-mika-platform-mika/memory/feedback_build_tooling.md)
- [Extract shared UI package](extract-shared-ui-package.md)
- [Docker BuildKit cache mounts](docker-buildkit-cache-mounts-compose.md)
