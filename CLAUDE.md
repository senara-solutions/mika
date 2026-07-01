# Mika - AI Executive Assistant

> **Hierarchical context:** This file is the root index (~20k chars). Each crate and subdirectory has its own `CLAUDE.md` with detailed architecture. Claude Code loads both this file and the CLAUDE.md in your current working directory. When working in `crates/mika-agent/`, you get both root context and agent-specific detail automatically.

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation. Each customer gets their own agent container with SQLite storage. A shared gateway (`mika-gateway`) routes Telegram and GitHub webhook messages to the correct container.

**Current phase:** Phase 4 — Deployment infrastructure (Dockerfiles done, CI/CD done).

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context -> build prompt -> LLM API -> match stop_reason -> execute tools or respond
- **LLM:** Multi-provider via `LlmProvider` trait (13 providers). See `crates/mika-common/CLAUDE.md` for provider details.
- **Database:** SQLite via rusqlite (single DB per container at `~/.mika/data/mika.db`)
- **HTTP server:** Axum 0.8 (mika-spirit binary). See `crates/mika-agent/CLAUDE.md` for endpoint details.
- **HTTP client:** reqwest 0.12 with rustls-tls
- **Async runtime:** tokio
- **MCP client:** rmcp 2.0 (official Rust MCP SDK) — stdio and Streamable HTTP transports
- **Config:** config-rs with `MIKA_` env prefix + dotenvy for `~/.mika/.env` secrets
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev) + optional OpenTelemetry export via `telemetry` feature flag
- **Telemetry:** opentelemetry 0.31 + tracing-opentelemetry 0.32, feature-gated OTLP HTTP export (Langfuse-compatible)
- **Dashboard:** React 19 + TypeScript + Vite + Tailwind CSS v4 + TanStack React Query. See `dashboard/CLAUDE.md`.

## Directory Structure

- `crates/mika-common/` — Shared library: config, LLM providers, Claude API client, OAuth, GitHub App auth, telemetry. See `crates/mika-common/CLAUDE.md`.
- `crates/mika-a2a/` — A2A (Agent-to-Agent) protocol v0.3: JSON-RPC types, task state machine, SSE streaming. See `crates/mika-a2a/CLAUDE.md`.
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, skills, task engine, HTTP server (mika-spirit). See `crates/mika-agent/CLAUDE.md`.
- `crates/mika-gateway/` — Telegram and GitHub webhook router: Postgres customer registry, message routing, A2A proxy. See `crates/mika-gateway/CLAUDE.md`.
- `crates/mika-cli/` — TUI CLI binary (`mika`): ratatui chat interface, clap subcommands. See `crates/mika-cli/CLAUDE.md`.
- `packages/ui/` — `@senara-solutions/ui` shared React component library (Vite library mode, published to npmjs.org as a public package — token-free anonymous install, mika#1386). Components: StatusBadge (six-variant: success/warning/error/info/neutral/blocked), Pagination, EmptyState (with optional action affordance), LoadingState (list/detail skeleton variants with ARIA), ErrorState (retry + details affordances with ARIA), CopyButton, MarkdownContent, TaskStatusBadge (thin adapter delegating to StatusBadge), ListRow (three-variant: static/navigable/expandable — canonical row primitive for all list/table surfaces), SelectFilter (categorical one-of-N filter dropdown), AgentFilter (thin adapter delegating to SelectFilter with consumer-injected agents prop), TimeRangeFilter (presets + custom picker, ISO 8601 emission, server-side enforcement), TokenBudgetBar (three-tier color threshold progress bar with ARIA meter semantics), CostMeter (unbounded threshold-based cost display with ARIA status semantics — two variants: full/chip), LiveRefreshToggle (auto-refresh toggle switch + LIVE badge — canonical affordance for all dashboard live-refresh surfaces). **Hand-rolled implementations of these primitives are review fails — see `packages/ui/CLAUDE.md` for enforcement rules and escape-hatch criteria.** See `packages/ui/CLAUDE.md`.
- `dashboard/` — React observability dashboard. See `dashboard/CLAUDE.md`.
- `docs/` — Public documentation (architecture, configuration, deployment, runtime-structure, skills, slash-commands, getting-started) — **single source of truth** for all docs. See [docs/runtime-structure.md](docs/runtime-structure.md) for full `~/.mika` directory layout, DB schema, and log paths.
- `docs/adr/` — Architecture Decision Records (numbered)
- `docs/architecture/` — Architecture references including `review-guide.md` (SOLID/DRY/YAGNI/KISS/Orthogonality with citations to mika code; primary consumer is `mika-arch`'s plan-review skills, but applies to any code authored or reviewed in this repo).
- `docs/design/` — Design system: `north-star.md` (the WHY behind every visual decision across the Mika ecosystem) + `luminescent-core.md` (the rulebook) + `dashboard-stitch-map.md` (Dashboard ↔ Stitch reconciliation, milestone #13 sequence, workflow agreement). Single design system across Observability Dashboard, Cloud Console, and Landing Page; consumed via `packages/ui/` (`@senara-solutions/ui`). The rulebook is owned by Vincent and updated via direct commits, not PRs; implementation PRs apply it but do not relitigate it.
- `docs/openapi/` — OpenAPI specs (mika-spirit.yaml, gateway.yaml)
- `docs/solutions/` — Documented solutions to past problems (bugs, best practices, workflow patterns), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`). Relevant when debugging or implementing in documented areas.
- `skills/bundled/` — Source tree for engine-coupled bundled skills discovered at build time via `crates/mika-agent/build.rs`. See `crates/mika-agent/CLAUDE.md` Skills System for details.
- `scripts/` — Utility scripts (sync-agent-docs.sh for crates.io publish prep)
- `Makefile` — Development workflow targets: `make build`, `make deploy` (dashboard+build+install+restart), `make test`, `make lint`, `make fmt`, `make check`
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands (`/mika` — full dev workflow, `/mika-doc-audit` — standalone documentation audit, `/mika-issue` — create a single GitHub issue, `/mika-issues` — batch-create GitHub issues)

## Versioning

- **Pre-1.0 breaking changes:** Until v1.0, breaking changes do not require backward compatibility. They are shipped as minor or patch releases (no major version bump). PRs that introduce breaking changes must document the required manual migration steps in the PR description.

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run. Integration tests for the agent loop live in `crates/mika-agent/tests/eval/` — these use `MockLlmProvider` (sequence-based, no network) via the `EvalHarness` builder to exercise the full `run_agent()` path deterministically. `EvalHarness` supports optional dependency injection via builder methods: `.embedding_client()`, `.brave_api_key()`, `.github_token()`, `.mcp_manager()` (all default `None`). `MockLlmProvider` is in `mika-common::llm::mock`, gated behind `#[cfg(any(test, feature = "test-utils"))]`. `Settings::test_defaults()` in `mika-common` provides a canonical test `Settings` constructor (also `test-utils` gated). Real-provider eval matrix tests are gated behind `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` env var — run with `cargo test -p mika-agent --test eval -- --ignored` after setting `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq` (comma-separated, or `all`). Calibration mode (`MIKA_EVAL_CALIBRATE=1`) writes ephemeral artifacts to `target/eval-calibration/`. KG provider comparison eval (#762) lives at `tests/eval/kg_provider_eval/` and is gated separately behind `#[ignore]` + `MIKA_EVAL_KG_PROVIDERS` (comma-separated `provider/model` strings, or `default` for the four-provider minimum set) — run with `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval`. Fixtures live in `docs/solutions/kg/eval-fixtures-2026-04-24/`; decision matrix in `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md`.
- **Model calibration (#1190):** Never swap an agent's base model or skill-override model without a passing `make calibrate-<role>` run. The calibration framework at `crates/mika-agent/src/calibration/` provides role-scoped scenario suites (mika-dev: 5 scenarios anchored on #1168/#1166/#1173; mika-arch: 5 scenarios for disposition/citation/finding-list contracts; mika-qa: 5 scenarios for verdict format/AC enumeration/absence grounding/wip-rescue/replay consistency). Run: `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6`, `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6`, or `make calibrate-mika-qa MODEL=anthropic/claude-sonnet-4-6`. The `calibrate` binary produces JSON artifacts + markdown reports. Baselines live at `docs/eval/calibration/baselines/`. Every model-swap PR must include the calibration report and update the baseline.
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Doc sync:** `docs/` is the single source of truth. `crates/mika-agent/build.rs` copies docs into `OUT_DIR` at build time via `include_str!(concat!(env!("OUT_DIR"), ...))`. Crate-local copies in `crates/mika-agent/docs/` are fallbacks for crates.io; sync them with `scripts/sync-agent-docs.sh` before publishing. CI enforces sync via the `docs-sync` job in `ci.yml` — PRs that modify `docs/` without running the sync script will fail.
- **Proactive state checking:** The system prompt instructs the agent to check existing state before any write operation to prevent duplicates after compaction. New write tools should have a corresponding query tool.
- **Grounding rule:** The system prompt prohibits the agent from claiming downstream system state unless a tool result confirms it. Reinforced in `format_callback_framing` and `SilentTrigger::Callback`.
- **Confirmation before action:** The system prompt instructs the agent to answer informational questions directly without starting multi-step workflows.
- **Context priority:** current user message > core memory > active skill context > conversation summary > conversation history > search results. See `docs/memory-classification.md`.
- **Database:** Case-insensitive COLLATE NOCASE on unique text columns. Schema v39. See `crates/mika-agent/CLAUDE.md` for schema details.
  - v27->v28: `agent_kg_corpora` table (#798) — maps `agent_id` to `docs_root_hash` for multi-corpus query fan-out. Populated by startup lexical ingest.
  - v28->v29: Backfill migration (#908) — scrubs secret-shaped values from existing `tool_calls.input` and `tool_calls.output` rows using `secret_scrubber::scrub_secrets()`. Data-only, no DDL. New `save_tool_call()` calls also scrub before INSERT.
  - v29->v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'matched_llm_db_fallback'` (#874). Enables DB-fallback acceptance path for LLM matches outside the in-prompt candidate window.
  - v30->v31: `llm_calls.response_text TEXT` and `llm_calls.reasoning TEXT` columns (#653). Stores serialized LLM response content (stripped of internal tags, 50K char cap) and extended thinking text. Additive ALTER TABLE, no table rebuild.
  - v31->v32: `kg_invalidated_no_match` sidecar table (#961). Tracks entities retried after domain-graph rebuild invalidates prior `no_match` outcomes.
  - v32->v33: Delete NULL-hash `kg_extractions` rows (#1052). Fixes the NULL-hash deadlock where pre-v26 rows cycled between "pending" and "skip" on every startup. Companion to the `INSERT OR IGNORE` → `ON CONFLICT DO UPDATE` upsert change in `subject_extractor.rs`.
  - v33->v34: `tasks.dispatch_class TEXT` column (#1001). Per-class dispatch slot split — `'implement'` or `'groom'`, nullable (pre-v34 NULL = `'implement'` via COALESCE). Enables concurrent grooming + implementation dispatches per agent.
  - v34->v35: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'no_candidate_of_type'` (#1154). Table rebuild. Enables the resolver to distinguish phantom subjects (no domain counterpart) from genuine disambiguation failures.
  - v38->v39: `operational_items` table (#1262). Canonical operational-item ledger for the What's Next engine. Seven `kind` variants (goal/task/commitment/decision/blocker/evidence/next_action), six `status` variants (now/waiting/delegated/scheduled/at_risk/done), dedup via `UNIQUE(agent_id, source_table, source_id)` partial index. Writes always-on; reads gated behind `MIKA_OPERATIONAL_PARTNER=1`. Evidence refs stored as JSON TEXT column with Rust-type-as-only-writer contract.
- **Secrets:** All API keys and tokens in `Settings` use `secrecy::SecretString` (`Option<SecretString>`) for compile-time exposure safety and zeroize-on-drop. Secrets are exposed at the `Settings` accessor boundary (e.g., `provider_fields()`, `agent_github_token()`) via `.expose_secret()` — downstream types use plain `String`/`&str`. `Settings` has manual `Debug` impl that redacts all secret fields. `get_effective_value()` returns `"[SET]"` for secret-flagged fields (never raw values). Exec handler executor scrubs all MIKA_* env vars from child processes. MCP child processes use `env_clear()` + allowlist. Git subprocesses scrub MIKA_* vars and set `GIT_TERMINAL_PROMPT=0`.
- **Labels:** `.github/labels.yml` is the canonical label taxonomy (type, priority, component, state, sprint hooks, automation). All issue-creation paths reference it. The `ready` label is the canonical positive-consent signal for mika-dev autonomous dispatch (mika#841). The `release` label is applied by the `release-pr` CI workflow to release PRs.
- **Operational ledger:** Writes to `operational_items.evidence_refs` MUST go through `crates/mika-agent/src/operational/types.rs::EvidenceRef`. Direct SQL writes that bypass the Rust type are unsupported and will break the closed-enum guarantee from foundation Decision F.
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `tokio::sync::mpsc::channel(512)` bounded channel (closure-based dispatch). Clone-able, Send+Sync. `with_db` releases the mutex before calling `send().await` — async backpressure yields to the Tokio executor instead of pinning worker threads (#1258). Worker thread uses `blocking_recv()` (sync bridge for the async channel; `rusqlite::Connection` is `!Send` so the worker must be a dedicated OS thread). Closures take `&mut Database` (required by `rusqlite::Transaction` RAII — see #636).

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~3463 tests)
- `cargo test -p mika-agent --test eval` — Run agent loop integration tests (eval harness)
- `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai cargo test -p mika-agent --test eval -- --ignored` — Run real-provider eval matrix (requires API keys)
- `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval` — Run KG provider comparison eval (requires API keys for all selected providers)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, `mika kg status`, etc.)
- `cargo run --bin mika-spirit` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev:dashboard` — Run dashboard dev server (builds `@senara-solutions/ui` first, requires mika-spirit on :8080)
- `npm run build --prefix dashboard` — Build dashboard for production (sets `VITE_BASE_PATH=/dashboard/` automatically)
- `make deploy` — Full deploy: build dashboard + release binaries with telemetry, install to `~/.local/bin/`, restart services. Prints the built SHA and warns when local HEAD is behind `origin/main`.
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image
- `docker compose up` — Run agent + gateway (add `--profile db` for local Postgres)
- `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6` — Run mika-dev calibration suite
- `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6` — Run mika-arch calibration suite
- `make calibrate-mika-qa MODEL=anthropic/claude-sonnet-4-6` — Run mika-qa calibration suite

## Architecture Summary

For detailed architecture of each subsystem, see the crate-level CLAUDE.md files. Key architectural decisions:

- **One container per customer** — per-customer isolation with SQLite
- **Three-layer memory model** — core memory (system prompt) + structured facts + hybrid search (FTS5 + vector). See `crates/mika-agent/CLAUDE.md`.
- **Agent loop** — max 20 tool steps, 5-min timeout, 11 post-condition guards on EndTurn (includes intent-precondition registry, required-suffix-line guard, required-finding-list guard, milestone-close-claim guard, and assert-grounded guard). See `crates/mika-agent/CLAUDE.md`.
- **Skills marketplace** — git-based distribution, per-provider/model prompt variants, dependency resolution. See `crates/mika-agent/CLAUDE.md`.
- **Unified task engine** — SQLite-backed scheduler, callback/resume lifecycle, team suspend/resume. See `crates/mika-agent/CLAUDE.md`.
- **HTTP server (mika-spirit)** — Axum, two auth layers, embedded dashboard. See `crates/mika-agent/CLAUDE.md`.
- **Gateway** — Telegram + GitHub webhook routing, A2A proxy, Postgres. See `crates/mika-gateway/CLAUDE.md`.
- **A2A protocol** — v0.3, JSON-RPC, task state machine. See `crates/mika-a2a/CLAUDE.md`.
- **Knowledge Graph** — Three-layer KG (domain/lexical/subject) in SQLite. Domain graph builder (deterministic, startup) projects skills/tools/agents/problem_types/concepts into `kg_entities`/`kg_relationships`. Concept entities (#928) use hierarchical naming (`concept:cross-repo:*`, `concept:infra:*`) to cover cross-repo workflow and Helm/K8s infrastructure concepts for mika-platform and mika-cloud corpora. Lexical ingestor (#689) chunks `docs/solutions/**/*.md` per-agent into `kg_chunks` + FTS5/vec search. Subject extractor (#690) runs LLM-based NER to extract entities and fact triples from chunks into `kg_subject_entities`/`kg_subject_relationships` with provenance tracking. Extraction runs async at startup (background per-agent) and sync on compound hook. Entity resolver (#691) bridges subject graph to domain graph via two-stage pipeline (exact-match then LLM disambiguation) into `kg_subject_resolutions`/`kg_resolutions_log`. Resolution runs async at startup and as background spawn after compound extraction. Per-agent KG scoping via `identity.toml` `[kg]` section (#778) — `enabled` (default true) and `docs_root` (optional) control per-agent corpus isolation; agents with matching `docs_root` share extraction via `docs_root_hash` (v27). **KG topology (#800):** mika-arch is the sole KG consumer among well-known agents; mika-dev and mika-qa are provisioned with `[kg].enabled = false` (zero `query_knowledge_graph` usage — retrieval goes through `search_memory`). Re-enable per-agent with one identity.toml edit + restart if a dev/qa flow needs KG. See `crates/mika-agent/CLAUDE.md`.
- **Docker images:** Multi-stage builds with BuildKit cache. `Dockerfile.agent` (95MB) for per-customer containers. `Dockerfile.gateway` for the stateless gateway. Both use rustls, non-root user `mika` (UID 1000). Release profile: LTO + strip. `docker-compose.yml` defines agent, gateway, and postgres services. **Host dependency:** `jq` is required by all skill handler scripts.
- **CI/CD:** Five GitHub Actions workflows: `ci.yml` (PR checks), `pr-body-validation.yml` (PR body validation), `release-pr.yml` (versioning/changelog via release-please), `release.yml` (cross-platform binaries), `publish-ui.yml` (`@senara-solutions/ui` to npmjs.org as a public package). All actions pinned to commit SHAs. CI includes a `byte-slice-lint` job that runs `scripts/check-byte-slices.sh` to prevent unsafe `&str` byte-slicing patterns that panic on multi-byte UTF-8 (#764), a `loop-select-lint` job that runs `scripts/check-loop-select.sh` to reject `tokio::select!` inside `run_loop`'s body — the deadline-check guarantee depends on iteration-top semantics not being shadowed (#848), and a `docker-build` job that builds all Dockerfiles (agent, gateway, mika-os, mika-runtime-server, mika-runtime-gateway, mika-runtime-cli, mika-runtime-all) on every PR to catch structural bugs before merge. **PR Body Validation (#527):** `pr-body-validation.yml` runs `scripts/check-pr-body-consistency.sh` on every `pull_request` event (opened, edited, synchronize). Two checks: (a) closure-consistency — when the PR body declares `Closes #N`, the script walks #N's formal sub-issues via GitHub GraphQL `trackedIssues`; if any are OPEN and not acknowledged, the gate hard-fails (`exit 1`); (b) follow-up tracker — when the body contains a deferral trigger phrase (e.g., "will be fixed in a follow-up", "deferred to a separate PR"), a `Tracked in: <ref>` line naming the tracker issue/PR is required. To resolve failures: add `Tracked in: senara-solutions/<repo>#<number>` lines to the PR body for each deferred item, or close the sub-issues in the same PR.

## Orchestrator Role Transfer (mika#1641)

The platform-orchestrator seat is transferring from Claude Code to **Mika** (the
executive-assistant agent); Claude Code's role shrinks to **monitor-only**. This is a
staged, bounded-reversible transfer with seven acceptance criteria (AC1–AC7). Code and
docs (AC1 tool surface, AC2 calibration, AC3 handbook, AC7 rollback) ship ahead of the
operational cut (AC5 pair-mode window, AC6 hard cut). Key documents:

- **Operator handbook:** `docs/operator/mika-orchestrator-handbook.md` — daily-rhythm
  checklist, wedge taxonomy, routing matrix, hard rules, escalation chain, tool
  quickref. Seeds Mika's core memory (AC3).
- **Rollback procedure:** `docs/operator/mika-orchestrator-rollback.md` — one-line
  reverts to the pre-transfer topology (AC7).
- **Bearing-circle decision:** `docs/operating/bearing-circle.md` — whether Mika enters
  Mika Prime's conversation circle. **Vincent-only (AC4), decision pending**; AC5 gates
  on it.
- **Calibration:** `make calibrate-mika-orchestrator MODEL=<provider/model>` +
  `docs/eval/calibration/mika-orchestrator-1641/` (AC2). No orchestrator model swap
  without a passing run (mika#1190).

Mika's orchestrator tool surface is the `github` skill added to
`DEFAULT_AGENT_SKILL_ALLOWLIST` (`crates/mika-common/src/home.rs`), on top of the
`git-ops` / `shell-exec` / `tmux` / `file-reader` / `gh-read-only` she already carries.

## Environment Variables

See `.env.example` for the full list. Per-provider API keys (set the one for your active provider):
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key. For OAuth tokens (`sk-ant-oat*`): auto-detected, PKCE managed auth (auto-refresh via `OAuthTokenManager`, cached in `~/.mika/oauth.json`). OAuth setup: `mika setup --mode oauth`.
- `MIKA_OPENAI_API_KEY` — OpenAI API key (also used for Layer 3 vector search embeddings)
- `MIKA_OPENROUTER_API_KEY` — OpenRouter API key
- `MIKA_GROQ_API_KEY` — Groq API key
- `MIKA_OLLAMA_API_KEY` — Ollama API key (usually not needed)
- `MIKA_MISTRAL_API_KEY` — Mistral API key
- `MIKA_GOOGLE_API_KEY` — Google AI API key
- `MIKA_DEEPSEEK_API_KEY` — DeepSeek API key
- `MIKA_ZAI_API_KEY` — Z.AI API key (for native GLM-5.2 routing, bypasses OpenRouter margin). Optional `MIKA_ZAI_MODEL` (default `glm-5.2`). See https://z.ai/docs/api.

Optional (web search):
- `MIKA_BRAVE_API_KEY` — Brave Search API key for `web_search` builtin skill (get free key at https://brave.com/search/api/)

Optional (Knowledge Graph LLM):
- `MIKA_KG_INGESTION_MODEL` — Shared fallback model for KG extraction and resolution. Format: `provider/model`. **OpenRouter is recommended** — Anthropic direct is ~10× more expensive for bulk NER and triggers a `kg_anthropic_provider` WARN at startup. Example: `openrouter/deepseek/deepseek-v3`. If unset, KG features requiring LLM calls are disabled.
- `MIKA_KG_EXTRACTION_MODEL` — Model for NER + fact-triple extraction (#690). Falls back to `MIKA_KG_INGESTION_MODEL` if unset. Task is mechanical JSON extraction — cheap/fast tier recommended.
- `MIKA_KG_RESOLUTION_MODEL` — Model for entity resolution disambiguation (#691). Falls back to `MIKA_KG_INGESTION_MODEL` if unset. Mid-tier model recommended for better judgment on ambiguous matches.
- `MIKA_KG_BATCH_BUDGET` — Per-batch LLM call cap on KG startup extraction and resolution (#757). Default `500` per #757 burst-defense invariant ("no silent multi-thousand-call bursts"). Budget is distributed fairly across corpora for both extraction (#962) and resolution (#927) using two-pass allocation (`kg::budget::allocate_fair_budget`), so array order no longer starves secondary corpora. Worst-case per-startup cost is `2 × N_agents × budget` (extraction batch + resolution batch, one of each per agent). The 30-min periodic tick (#906, #1052) runs both extraction and resolution at the same budget, adding up to `2 × N_agents × budget` LLM calls per tick (48 ticks/day). Once extraction coverage reaches 100%, the tick's extraction phase is a no-op (zero pending = zero budget allocated). Overflow emits a `kg_budget_exhausted` WARN and leaves remaining work for the next tick. `0` disables the phase entirely. Extraction idempotency uses `ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE` upsert (#1052) — NULL-hash rows and content-changed docs are re-extracted; identical-content re-extractions are no-ops. See `docs/solutions/architecture-patterns/kg-extraction-trigger-semantics-2026-05-09.md` for the full trigger model.
- `MIKA_KG_DOCS_ROOT` — Absolute path to the docs root the `LexicalIngestor` reads (#738). Defaults to `<CWD>/docs/solutions` when unset — works in containers where the Dockerfile copies `docs/` into the workdir. Needed on hosts where the service starts with CWD ≠ repo root (e.g., OpenRC `supervise-daemon` launches with CWD=`/`). Also settable as `kg_docs_root` in config.toml. If set to an empty string, lexical ingestion skips with a distinct warn.
- `MIKA_KG_DOCS_ROOTS` — Optional colon-separated list of docs-root paths for multi-corpus agents (e.g., mika-arch reasoning across multiple repos). Global fallback; per-agent `[kg].docs_roots` in identity.toml takes precedence. Linux/macOS only. **Required for mika-arch in dev mode** — at provision time, `MIKA_ARCH_IDENTITY` is computed from this env so `[kg].docs_roots` always contains absolute paths (mika-spirit runs with CWD=`/` under OpenRC/systemd). When unset, mika-arch is skipped at provision with an explicit `error!` log; other well-known agents (mika-dev/qa/relay) come up normally.

### Post-restart safety check (#757)

After KG-related deploys, four signals tell you the fix is working. The second restart after deploy is the steady-state signal — the first restart backfills NULL `source_doc_hash` rows from v26 under the budget.

- **Signal A — extraction not re-running.** `grep subject_extraction_start server.log | jq 'select(.pending_docs == 0)'` should list every agent by the second post-deploy restart. Note: does NOT imply resolver is caught up — see Signal C.
- **Signal B — budget not exhausted.** `grep kg_budget_exhausted server.log` returns zero lines on a healthy restart.
- **Signal C — resolver backlog drains over time.** `SELECT agent_id, COUNT(*) FROM kg_subject_entities se WHERE NOT EXISTS (SELECT 1 FROM kg_resolutions_log rl WHERE rl.agent_id = se.agent_id AND rl.subject_entity_id = se.id) GROUP BY agent_id;` → count trends to 0 across restarts (bounded by per-restart budget). May take multiple restart cycles for agents starting > 3,000 pending.
- **Signal D — concrete cost prediction.** With OpenRouter configured, expect ~`N_agents × budget` resolution LLM calls on the first restart + ~0 extraction calls. At OpenRouter cheap-tier pricing (~\$0.0001/call) that's **\$0.05–\$0.50 per restart** until the backlog drains. Substantially more than \$1 per restart indicates budget-guard failure, stale idempotency, or provider routing regression.
- **Signal E — tick drain (#906).** `grep kg_resolver_tick.complete server.log | jq 'select(.pending_after == 0)'` — `pending_after` trending to 0 over hourly windows confirms the periodic resolver tick is draining the backlog without restart. Steady-state mika-arch primary corpus should reach `pending_after == 0` within ~17–18 hours of continuous operation (at 500/tick, 2 ticks/hour). Sustained `aborted_budget = true` across multiple ticks indicates the operator should raise `MIKA_KG_BATCH_BUDGET` temporarily for accelerated drain.
- **Signal F — per-corpus fairness (#927).** `grep kg_resolver_tick.complete server.log | jq '.per_corpus_attempted'` — the `per_corpus_attempted` JSON field shows attempt counts per corpus per tick. After #927, all corpora with pending entities should show non-zero attempts on every tick, not just the primary. If a secondary corpus shows 0 attempts while having pending entities, the fairness allocation is broken.
- **Signal G — extraction per-corpus fairness (#962).** `grep subject_extraction_ready server.log | jq '.per_corpus_extracted'` — the `per_corpus_extracted` JSON field shows doc-extraction counts per corpus per startup. After #962, all corpora with pending docs should show non-zero extractions, not just the primary. If a secondary corpus shows 0 extractions while having pending docs, the fairness allocation is broken. **Architectural note:** Extraction fairness is caller-side (`server/mod.rs` distributes budget to per-corpus `SubjectExtractor` instances); resolution fairness is internal (`SubjectEntityResolver` distributes budget across corpora within a single instance). This asymmetry is intentional — the extractor's provenance transaction is per-doc-per-corpus and would be invasively complex to multi-corpus-ify (see mika#962 plan).
- **Signal H — extraction tick drain (#1052).** `grep kg_extraction_tick.complete server.log | jq 'select(.total_pending == 0)'` — `total_pending` trending to 0 confirms the periodic tick is draining the extraction backlog without restart. Companion to Signal E (resolution tick drain). `grep kg_extraction_coverage server.log | jq '.per_corpus_coverage'` shows per-corpus extraction coverage percentages (total, extracted, null_hash, pct). All corpora should converge to 100% coverage within a few tick cycles after deploy.
- **Signal I — per-agent search index backfill (#1155).** `grep lexical_ingest_complete server.log | jq 'select(.docs_index_backfilled > 0)'` — `docs_index_backfilled` > 0 on first restart after deploy confirms multi-corpus agents (mika-arch) are self-healing their per-agent `search_content` gap. Subsequent restarts should show `docs_index_backfilled=0` (backfill is one-shot per agent per corpus). `chunks_indexed_backfill` gives the total chunk-row count written. If non-zero persists across multiple restarts, the skip-path optimization is racing or the chunker version drifted.
- **Signal J — no-op wrapper detection (#1172).** `grep deferred_dispatch_noop_completion server.log` — any hits indicate a deferred wrapper completed its silent turn without spawning a real `run_claude_pilot` dispatch. This is the failure mode mika#1124 fixed; a hit post-deploy means the fix regressed or a new no-op-cascade variant appeared. Investigate the parent task ID in the log event to determine whether the dispatch slot is stuck.
- **Signal K — guard fabrication telemetry (#953).** Two paired events: (a) `grep 'guard\.' server.log | jq 'select(.event | startswith("guard.") and . != "guard.correction_accepted")'` — detection events. Any hits indicate a fabrication-class guard fired. The `guard_correlation_id` field links to the correction event. (b) `grep guard.correction_accepted server.log | jq '{guard_correlation_id, corrected_content}'` — correction events. The `corrected_content` field shows the accepted response after re-prompt. Join on `guard_correlation_id` for the full detection→correction trace. (c) `grep arch_anchoring_self_report server.log` — architect correct-self-report events (success signal, not fabrication). Also queryable via SQL: `SELECT * FROM audit_events WHERE target_key = 'arch_anchoring_self_report'`. Sustained guard detections from the same agent indicate #952 prompt defenses need reinforcement.
- **Signal L — identical-diff circuit breaker (#1563).** `grep identical_diff_circuit_breaker server.log` — any hits indicate the circuit breaker fired. The `head_sha` and `identical_count` fields show the convergence failure details. Investigate the PR and plan for the root cause of the stuck fix loop.
- **Signal M — pilot push guard (#1318).** `grep pilot_push_guard server.log` — two sub-events emitted by dispatch-lib to stderr on every dev-groom dispatch: `pilot_push_guard.clean` (no remote-ref change during pilot session — expected on every dispatch) and `pilot_push_guard.violation` (pilot pushed to the remote — scope-of-authority violation, should never appear; investigate immediately if it does). The guard compares `PRE_RUN_REMOTE_HEAD` (captured via `git ls-remote` before pilot launch) with the post-run remote HEAD. Any change indicates the pilot pushed, which is a content-only scope violation for dev-groom. On violation, the dispatch is marked `PIPELINE_INCOMPLETE — push violation` and the iterate loop + push are skipped.
- **Signal N — wip-rescue staleness probe (#1631).** `grep stale-against-main` in GitHub PR labels — any open draft PR with this label has a type-incompatible rebase against current main. The `wip-staleness-check` workflow runs on every push to main and probes all `wip(` titled or `wip-rescue` labelled draft PRs. Operator action: rebase the branch, fix clippy errors, then promote from draft.

Optional (GitHub App — preferred over PAT for agent operations):
- `MIKA_GITHUB_APP_ID` — GitHub App ID (u64). Required for GitHub App authentication.
- `MIKA_GITHUB_APP_PRIVATE_KEY` — GitHub App private key (base64-encoded PEM). Encode with: `base64 -w0 < your-app.pem`
- `MIKA_GITHUB_APP_INSTALLATION_ID` — GitHub App installation ID for the org (u64). All 3 vars must be set; when configured, installation tokens replace PAT for `run_gh`, context injection, and work item enrichment. Falls back to PAT on exchange failure.
- `MIKA_GITHUB_APP_LOGIN` — GitHub App bot login (e.g., `mika-dev[bot]`). Used for assignee filtering in autonomous issue pickup. Optional — derived from the App slug or set explicitly. Per-agent: set in `~/.mika/agents/<name>/.env` for per-agent App identity.

Optional (GitHub — agent operations, fallback when App not configured):
- `MIKA_GITHUB_TOKEN` — GitHub Personal Access Token for agent operations (context injection, work item enrichment, PR merge). Needs Pull requests R/W, Issues R/W, Contents R scopes.

Optional (GitHub — investigation panel):
- `MIKA_INVESTIGATE_GITHUB_TOKEN` — GitHub Personal Access Token for investigation panel issue creation only (needs `repo` scope for private repos, `public_repo` for public). Not used for agent operations when `MIKA_GITHUB_TOKEN` is set.
- `MIKA_GITHUB_REPO` — Target repository in `owner/repo` format (e.g. `senara-solutions/mika`). Both `MIKA_INVESTIGATE_GITHUB_TOKEN` and this must be set to enable the `create_github_issue` investigation tool.

Optional (gh CLI in agent sessions):
- `GH_TOKEN` — GitHub PAT for `gh` CLI in Claude Code sessions spawned via claude-pilot. Without this, `gh` falls back to the host user's personal `~/.config/gh/hosts.yml`. Do NOT set in `~/.mika/.env` — if detected there, `check_env_warnings()` actively removes it from the process environment at startup (#380). `scrub_mika_env_vars()` also scrubs `GH_TOKEN` from exec handler child processes via `EXTRA_SCRUB_VARS` (defense-in-depth). The builtin `run_gh` and `pr_merge_with_gate` handlers, along with all exec handler skill subprocesses, re-inject `MIKA_GITHUB_TOKEN` as `GH_TOKEN` AFTER the scrub for platform identity separation (#515).

Server mode additionally requires:
- `MIKA_ROUTING_URL` — Gateway URL for outbound message delivery
- `MIKA_INTERNAL_TOKEN` — Shared secret for Bearer auth between gateway and agent

Optional (startup behavior):
- `MIKA_DEV_MODE` — Enable dev mode (default: false). When true, auto-provisions well-known development agents (`mika-dev`, `mika-qa`, `mika-relay`, `mika-arch`) on startup with role-specific identity, soul, and skill assignments. mika-dev gets self-dev family skills (KG disabled, #800); mika-qa gets qa-review family skills (KG disabled, #800); mika-relay gets only permission-policy (haiku model for cheap permission classification); mika-arch gets groom-ticket, groom-milestone, and second-review skills (read-only architect, Kimi base with Sonnet 4.6 skill overrides, sole KG consumer). Idempotent — existing agents are never overwritten.
- `MIKA_DISABLE_BUNDLED_SKILLS` — Skip bundled skill re-sync on startup (default: false). WARNING: do not enable in production — prevents security updates to handler scripts.
- `MIKA_DISABLE_AGENT_PROVISIONING` — Skip well-known agent auto-creation on startup (default: false). When true, prevents `dev_mode` from creating or updating agent identity files, allowing manual edits to persist across restarts/deploys. Same pattern as `MIKA_DISABLE_BUNDLED_SKILLS`.

Optional (callback watchdog):
- `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` — Grace period (seconds) after subprocess death detection before marking a callback task `failed` (default: 120). The watchdog runs every 60s in the engine tick loop and detects dead subprocesses via `/proc/<pid>/stat` process start time comparison. Prevents stale long-running callbacks from blocking the dispatch queue indefinitely (#959).

Optional (dispatch grooming gate):
- `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` — Emergency bypass for the grooming-marker dispatch gate (#919). When `1` or `true` (case-insensitive), `validate_dispatch_readiness()` skips the three-signal grooming check on `dev-pilot` dispatches. Logged at WARN on every hit. Default: unset (gate active).

Optional (runtime observability):
- `MIKA_STORE_LLM_CALLS` — Store LLM call metadata (model, tokens, latency) in SQLite (default: true)
- `MIKA_STORE_TOOL_CALLS` — Store full tool call input/output in SQLite (default: true, 50KB cap per field)
- `MIKA_LOG_LLM_BODIES` — Log full LLM request/response bodies at debug level to log file AND, when telemetry is enabled, attach as `gen_ai.prompt`/`gen_ai.completion` span attributes for Langfuse Generation input/output (default: false, dev-only)

Optional (telemetry — requires `--features telemetry` build):
- `MIKA_TELEMETRY_ENABLED` — Enable OpenTelemetry trace export (default: false)
- `MIKA_OTLP_ENDPOINT` — OTLP HTTP endpoint URL with `/v1/traces` path
- `MIKA_OTLP_AUTH_HEADER` — OTLP auth header value (Base64-encoded credentials for Langfuse)

Optional (CLI -> server communication):
- `MIKA_SPIRIT_URL` — mika-spirit base URL for CLI dashboard commands (default: `http://localhost:8080`)
- `MIKA_GATEWAY_URL` — mika-gateway base URL for CLI webhook DLQ commands (default: `http://localhost:3001`)

Optional (dashboard): See `dashboard/CLAUDE.md`.

Optional (log format and files):
- `MIKA_LOG_FORMAT` — Stdout log format for mika-spirit and mika-gateway: `json` (default) or `pretty` (human-readable, for local dev). CLI always uses pretty.
- `MIKA_SPIRIT_LOG_FILE` — File path for mika-spirit log output (always JSON regardless of `MIKA_LOG_FORMAT`)

Gateway mode: See `crates/mika-gateway/CLAUDE.md` for gateway-specific env vars.

## Pending Work

- **Deployment:** Production deployment guide, Docker image CI, Kubernetes/cloud manifests
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Workspace Context

This repo is part of the [mika-platform](../CLAUDE.md) workspace. For cross-repo navigation, development workflow, and the autonomous development loop, see `../CLAUDE.md`.

**Auto-groom on dispatch (mika#996):** The autonomous loop now auto-grooms ungroomed `ready`-labelled or milestone-child tickets before dispatching them to `dev-pilot`. When a ticket reaches dispatch (via `ready` label webhook or milestone-cascade M4) without a `Plan: docs/plans/` callout in its issue body, mika-dev dispatches `dev-groom` first (two-pass architect review via `mika-arch-groom-ticket`). On `Verdict: GROOMED`, the handler re-enters the dispatch flow and fires `dev-pilot`. On `Verdict: ESCALATE`, dispatch halts and surfaces to operator. Grooming and dispatch run serially as two phases of the same child task. Orchestrator-manual `/mika-groom-ticket` remains available for free-text dispatch and human-driven grooming.

Cross-repo documentation:
- `../docs/solutions/cross-repo-patterns/` — Security hardening playbook, reference architecture patterns

---

## Skills System

### Engine-Coupled vs Community Skills

Skills are bundled and discovered at build time via `crates/mika-agent/build.rs`. There are two categories:

**Engine-Coupled Skills** (in `skills/bundled/`):
- Live next to the Rust engine code they depend on
- Correctness depends on lockstep with engine schemas and contracts
- Ship atomically with the engine (discovered at build time)
- Examples: `self-dev`, `dev-pilot`, `qa-review`, `permission-policy`

**Community Skills** (in `mika-skills` repo):
- Standalone skills with no engine dependencies
- Distributed via git-based marketplace
- Examples: external API integrations, utility skills

### Directory Structure (`skills/bundled/`)

```
skills/bundled/
├── _shared/               # Shared dispatch library (dispatch-lib.sh) — NOT a skill, excluded from build-time discovery
├── self-dev/              # Main self-development orchestration (per-issue + milestone + project workflows)
├── self-dev-callback/     # Callback handler for claude-pilot and deploy_mika background tasks (#1106)
├── self-dev-iterate/      # PR iteration handler for self-dev
├── self-dev-webhook-qa/   # QA webhook handler for self-dev
├── self-dev-webhook-ci/   # CI webhook handler for self-dev
├── self-dev-webhook-ready-label/ # Ready-label dispatch handler for GitHub issue labeled ready (#1106)
├── dev-pilot/             # Claude Code implementation dispatch (thin wrapper → _shared/dispatch-lib.sh, entry: /mika)
├── dev-groom/             # Two-pass grooming dispatch — operator or autonomous (own tool: run_claude_pilot_groom, entry: /mika-groom-ticket; mika#1173)
├── qa-review/             # PR review skill
├── qa-review-build-callback/ # Build callback handler for QA review
├── permission-policy/     # Permission handler for claude-pilot sessions
├── resolve-pr-conflicts/  # Rebase + conflict resolution for CONFLICTING PRs
├── address-pr-comments/   # Address PR review comments
├── self-check/            # Self-check/health verification
├── build-mika/            # Build verification
├── deploy-mika/           # Deployment
├── agents-teams/          # Agent/team management
├── skill-review/          # Skill review handler
├── dev-handsoff/            # End-of-run handsoff artifact writer (v0.1, artifact-only — keyword-triggered, prompt-only)
├── mika-arch-groom-ticket/  # First-pass plan review (Sonnet 4.6) — produces READY/ITERATE/ESCALATE
├── mika-arch-groom-milestone/ # Milestone-level plan review (Sonnet 4.6) — per-sub-issue + sequencing + cross-cutting
└── mika-arch-second-review/ # Second-pass plan review (Sonnet 4.6) — produces GROOMED/ESCALATE
```

### Build-Time Discovery

The `build.rs` walks `skills/bundled/` and generates `BUNDLED_SKILL_MANIFESTS` containing:
- Skill name and manifest (from `skill.toml`)
- System prompt content (from `system_prompt.md`)

Skills are loaded at runtime from this generated constant — no filesystem access required. Directories starting with `.` (dotfiles) or `_` (convention-reserved for shared support libraries like `_shared/`) are excluded from **skill** discovery. **Support directories** (underscore-prefixed) are discovered separately at build time and seeded unconditionally by `seed_bundled_skills_if_needed()` (even when `MIKA_DISABLE_BUNDLED_SKILLS=true`), so sibling skills can source them at runtime via relative path (mika#923). The `_shared/` directory contains `dispatch-lib.sh` — shared plumbing for claude-pilot dispatch skills (dev-pilot, dev-groom).

- **Post-flight dirty-worktree recovery (#1282):** When dev-pilot exits with dirty worktree but zero commits, dispatch-lib auto-commits with `wip()` prefix, pushes, and opens a draft PR. Content is rescued; outcome remains PIPELINE_INCOMPLETE. Operator must review and promote the draft PR.
- **Worktree slash-command seeding (#1415):** A dispatch worktree's `.claude/commands/` is sourced from the **worktree's branch HEAD** — it is *not* a deploy/skill-install target. At worktree-setup, `dispatch-lib`'s `_seed_worktree_slash_commands()` adds the meta-repo orchestration commands (`/mika-groom-ticket`, `/mika-revise-plan`, …) so the inner Claude Code session can resolve them (#1173), under two hard invariants: (1) it **never overwrites** a command the worktree's branch already tracks — the sub-repo's own polymorphic `/mika` (#1255) and sub-repo-scoped `/mika-issue` always win; (2) the seeded meta-only copies are **shielded via the common-dir `info/exclude`** so they never dirty `git status` (a blanket `cp -r` previously re-clobbered `/mika` and dropped ~18 untracked siblings on every dispatch, breaking the resume rebase — the dirty-worktree cause behind #1414). Canonical *skill* install paths remain `~/.mika/agents/<agent>/skills/` (per-agent) and the main checkout's `.claude/commands/`; worktree command dirs are neither.

### Adding a New Bundled Skill

1. Create `skills/bundled/<skill-name>/` directory
2. Add `skill.toml` with required fields. Keywords live under `[triggers]` (required unless `always_on = true`):
   ```toml
   [skill]
   name = "<skill-name>"
   version = "0.1.0"

   [triggers]
   keywords = ["trigger", "words"]
   ```
3. Add `system_prompt.md` with the skill's system prompt
4. **Add to well-known agent allowlists** — all four well-known agents use identity-driven `[skills].allowlist` in their identity templates (`well_known_agents.rs`). New bundled skills are **denied by default** unless explicitly added to an agent's allowlist. Add the skill name to each agent's identity const that should have access:
   - `MIKA_DEV_IDENTITY` (26 skills) — development workflow skills
   - `MIKA_QA_IDENTITY` (17 skills) — review and quality skills
   - `MIKA_RELAY_IDENTITY` (1 skill) — only `permission-policy`; rarely needs new skills
   - `MIKA_ARCH` uses a computed identity via `build_mika_arch_identity()` (3 skills) — read-only review skills only
5. Build — the skill is automatically discovered and available
6. **Verify structure** — run `make verify-bundled-skills` (mika#1575). This pre-merge gate (the structural counterpart to mika#1326 AC2) asserts the new bundle is complete: required files present, manifest parses, handlers resolve, `required_tools` tokens consistent, and identity allowlists coherent. CI runs it on every PR. See `docs/architecture/bundled-skill-verification.md`.
