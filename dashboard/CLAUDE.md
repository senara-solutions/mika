# Dashboard — Observability UI

React 19 + TypeScript + Vite + Tailwind CSS v4 + TanStack React Query observability dashboard SPA.

## Architecture

- **Production:** Embedded into the mika-server binary via `rust-embed` and served at `/dashboard/*` (controlled by `MIKA_DASHBOARD_ENABLED`, default `false`)
- **Development:** Runs as standalone Vite dev server on `:5173` proxied to mika-server API
- **Token injection:** `window.__MIKA_CONFIG__` in embedded mode; `VITE_MIKA_DASHBOARD_TOKEN` env var in dev mode
- **Shared UI components:** `@senara-solutions/ui` extracted into `packages/ui/` (npm workspace)

## Pages

Event Timeline, Agents, Sessions, Traces, Tasks (+ detail), Team Runs, LLM Calls (+ detail), Tool Calls (+ detail), Dev Runs (+ detail). Auth via `VITE_MIKA_DASHBOARD_TOKEN` env var. Bearer token. Imports shared components from `@senara-solutions/ui`.

## `packages/ui/`

`@senara-solutions/ui` shared React component library (Vite library mode, published to GitHub Packages). Components: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge. Utils: formatTime, badges, agentColors. Theme CSS with design tokens. Peer deps: React 19, Tailwind CSS v4, lucide-react.

## Commands

- `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev:dashboard` — Run dashboard dev server (builds `@senara-solutions/ui` first, requires mika-server on :8080)
- `npm run build --prefix dashboard` — Build dashboard for production (sets `VITE_BASE_PATH=/dashboard/` automatically)
- `mika dashboard start` — Enable the embedded dashboard on the running mika-server (via `POST /api/v1/dashboard/enable`)
- `mika dashboard stop` — Disable the embedded dashboard on the running mika-server (via `POST /api/v1/dashboard/disable`)
- `mika dashboard status` — Query embedded dashboard status from mika-server (via `GET /api/v1/dashboard/status`)
- `mika dashboard open` — Open dashboard URL in browser

## Environment Variables

- `MIKA_DASHBOARD_ENABLED` — Initial state for embedded dashboard SPA at `/dashboard/` (default: `false`). Can be toggled at runtime via `POST /api/v1/dashboard/enable` and `POST /api/v1/dashboard/disable`. Requires `MIKA_DASHBOARD_TOKEN` to be set for token injection.
- `MIKA_CORS_ORIGIN` — Allowed origin for dashboard CORS (default: `http://localhost:5173`). Only applies to `/api/v1/*` dashboard routes.
- `MIKA_DASHBOARD_TOKEN` — Separate bearer token for read-only dashboard API routes (`/api/v1/*`). If unset, dashboard routes accept `MIKA_INTERNAL_TOKEN` (backwards compatible). `MIKA_INTERNAL_TOKEN` is always accepted on all routes (superuser). Dashboard frontend uses `VITE_MIKA_DASHBOARD_TOKEN` env var. Required for embedded dashboard token injection.

## Embedded Dashboard (Server-side)

Runtime-togglable via `AppState.dashboard_enabled: Arc<AtomicBool>` (initialized from `MIKA_DASHBOARD_ENABLED`, default `false`). Toggle endpoints: `POST /api/v1/dashboard/enable`, `POST /api/v1/dashboard/disable`, `GET /api/v1/dashboard/status` (returns `{enabled, has_assets, has_token}`) — all accept dashboard or internal token auth. When enabled, the pre-built React SPA is served at `/dashboard/*` via `rust-embed` (compiled into binary). SPA fallback to `index.html` for client-side routing. Token injected via `window.__MIKA_CONFIG__` using `serde_json` with HTML-safe escaping. Only `MIKA_DASHBOARD_TOKEN` is injected (never the internal superuser token). Security headers: `Cache-Control: no-store` on index.html, `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`. When disabled, `/dashboard/` returns a branded HTML page. Build sequencing: `npm run build --prefix dashboard` before `cargo build` (`VITE_BASE_PATH=/dashboard/` is set automatically in the npm build script; no build.rs coupling). Dockerfile.agent has a Node.js builder stage for this. CLI `mika dashboard start/stop/status` communicates with these toggle endpoints (requires `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN`; server URL from `MIKA_SERVER_URL`, default `http://localhost:8080`).
