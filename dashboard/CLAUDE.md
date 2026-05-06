# Dashboard — Observability UI

React 19 + TypeScript + Vite + Tailwind CSS v4 + TanStack React Query observability dashboard SPA.

## Design system

The Dashboard is one of three surfaces in the Mika ecosystem (alongside Cloud Console and Landing Page) that share a single design system. Before designing or implementing any visual change here, read:

- [`mika/docs/design/north-star.md`](../docs/design/north-star.md) — the WHY (intuitive, new-gen, uniform across the ecosystem, the system is the law).
- [`mika/docs/design/luminescent-core.md`](../docs/design/luminescent-core.md) — the rulebook (colors, typography, surfaces, components, do/don'ts).

The rulebook is owned by Vincent and updated via direct commits to main. Implementation PRs apply the rulebook but do not relitigate it. PRs that "feel different" must propose a rulebook extension first. See `north-star.md` § "What gets reviewed in PRs (and what doesn't)".

This Dashboard is currently undergoing the first of three ecosystem-wide design reconciliations (mika#669 / milestone #13 Dashboard improvements).

## Architecture

- **Production:** Embedded into the mika-server binary via `rust-embed` and served at `/dashboard/*` (controlled by `MIKA_DASHBOARD_ENABLED`, default `false`)
- **Development:** Runs as standalone Vite dev server on `:5173` proxied to mika-server API
- **Token injection:** `window.__MIKA_CONFIG__` in embedded mode; `VITE_MIKA_DASHBOARD_TOKEN` env var in dev mode
- **Shared UI components:** `@senara-solutions/ui` extracted into `packages/ui/` (npm workspace)

## Pages

**Overview / Home (mika#666):** Landing page at `/` (index route). Shows "state of the world" — stacked widget sections for Agents, Work Items, Dev Runs, Team Runs, Cost (24h), and Recent Activity. Composes existing API hooks on the frontend (no dedicated backend endpoint). Auto-refreshes all widgets at 15s shared interval with LIVE badge. Fresh-install gate: shows cohesive empty state when no agents are provisioned. Event Timeline moved to `/timeline`.

Event Timeline (`/timeline`), Agents, Sessions, Traces, Tasks (+ detail), Team Runs, LLM Calls (+ detail), Tool Calls (+ detail), Dev Runs (+ detail). Auth via `VITE_MIKA_DASHBOARD_TOKEN` env var. Bearer token. Imports shared components from `@senara-solutions/ui`. All list pages expose `<TimeRangeFilter />` with URL-reflected `?from=...&to=...` state (mika#659).

**LLM Calls page (mika#660):** `<CostTrendChart>` time-series visualization above the table, showing estimated cost over time aggregated from `llm_calls` token counts. Two variants: total (single line) and stacked-by-agent. Uses recharts. Backed by `GET /api/v1/llm-calls/cost-trend` with auto-bucketing (hourly <3d, daily ≥3d). Server defaults to last 24h when no `from` param. Pricing is estimated server-side via `crates/mika-agent/src/pricing.rs`.

**Agent Detail page (mika#656):** Tabbed Core Memory panel with three views — Sections (expandable blocks with `<TokenBudgetBar />` thresholds and per-section timestamps), Facts (paginated structured facts from people/commitments/preferences/events tables), and History (core memory edit audit trail filtered to `update_core_memory` events). Facts served via `GET /api/v1/agents/:id/facts`. Structured content (WORKFLOWS JSON) auto-detected and rendered as definition lists; non-JSON rendered via `<MarkdownContent />`.

## `packages/ui/`

`@senara-solutions/ui` shared React component library (Vite library mode, published to GitHub Packages). Components: StatusBadge (six-variant: success/warning/error/info/neutral/blocked), TaskStatusBadge (thin adapter delegating to StatusBadge), Pagination, EmptyState (with optional action affordance), LoadingState (list/detail skeleton variants with ARIA), ErrorState (retry + details affordances with ARIA), CopyButton, MarkdownContent, ListRow (three-variant: static/navigable/expandable — canonical row primitive for list/table surfaces with keyboard a11y and ARIA), SelectFilter (categorical one-of-N filter dropdown), AgentFilter (thin adapter delegating to SelectFilter with consumer-injected agents prop), TimeRangeFilter (presets + custom picker, ISO 8601 emission, server-side enforcement), TokenBudgetBar (three-tier color threshold progress bar with ARIA meter semantics). Utils: formatTime, badges, agentColors, formatApiError (human-shaped error message conversion). Theme CSS with design tokens (colors, spacing scale). Peer deps: React 19, Tailwind CSS v4, lucide-react. See `packages/ui/CLAUDE.md` for the enforcement rules and canonical primitives table.

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
