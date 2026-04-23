# Mika - AI Executive Assistant

> **Hierarchical context:** This file is the root index (~20k chars). Each crate and subdirectory has its own `CLAUDE.md` with detailed architecture. Claude Code loads both this file and the CLAUDE.md in your current working directory. When working in `crates/mika-agent/`, you get both root context and agent-specific detail automatically.

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation. Each customer gets their own agent container with SQLite storage. A shared gateway (`mika-gateway`) routes Telegram and GitHub webhook messages to the correct container.

**Current phase:** Phase 4 — Deployment infrastructure (Dockerfiles done, CI/CD done).

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context -> build prompt -> LLM API -> match stop_reason -> execute tools or respond
- **LLM:** Multi-provider via `LlmProvider` trait (11 providers). See `crates/mika-common/CLAUDE.md` for provider details.
- **Database:** SQLite via rusqlite (single DB per container at `~/.mika/data/mika.db`)
- **HTTP server:** Axum 0.8 (mika-server binary). See `crates/mika-agent/CLAUDE.md` for endpoint details.
- **HTTP client:** reqwest 0.12 with rustls-tls
- **Async runtime:** tokio
- **MCP client:** rmcp 0.17 (official Rust MCP SDK) — stdio and Streamable HTTP transports
- **Config:** config-rs with `MIKA_` env prefix + dotenvy for `~/.mika/.env` secrets
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev) + optional OpenTelemetry export via `telemetry` feature flag
- **Telemetry:** opentelemetry 0.31 + tracing-opentelemetry 0.32, feature-gated OTLP HTTP export (Langfuse-compatible)
- **Dashboard:** React 19 + TypeScript + Vite + Tailwind CSS v4 + TanStack React Query. See `dashboard/CLAUDE.md`.

## Directory Structure

- `crates/mika-common/` — Shared library: config, LLM providers, Claude API client, OAuth, GitHub App auth, telemetry. See `crates/mika-common/CLAUDE.md`.
- `crates/mika-a2a/` — A2A (Agent-to-Agent) protocol v0.3: JSON-RPC types, task state machine, SSE streaming. See `crates/mika-a2a/CLAUDE.md`.
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, skills, task engine, HTTP server (mika-server). See `crates/mika-agent/CLAUDE.md`.
- `crates/mika-gateway/` — Telegram and GitHub webhook router: Postgres customer registry, message routing, A2A proxy. See `crates/mika-gateway/CLAUDE.md`.
- `crates/mika-cli/` — TUI CLI binary (`mika`): ratatui chat interface, clap subcommands. See `crates/mika-cli/CLAUDE.md`.
- `packages/ui/` — `@senara-solutions/ui` shared React component library (Vite library mode, published to GitHub Packages). Components: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge.
- `dashboard/` — React observability dashboard. See `dashboard/CLAUDE.md`.
- `docs/` — Public documentation (architecture, configuration, deployment, runtime-structure, skills, slash-commands, getting-started) — **single source of truth** for all docs. See [docs/runtime-structure.md](docs/runtime-structure.md) for full `~/.mika` directory layout, DB schema, and log paths.
- `docs/adr/` — Architecture Decision Records (numbered)
- `docs/openapi/` — OpenAPI specs (mika-server.yaml, gateway.yaml)
- `docs/solutions/` — Documented solutions to past problems (bugs, best practices, workflow patterns), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`). Relevant when debugging or implementing in documented areas.
- `skills/bundled/` — Source tree for engine-coupled bundled skills discovered at build time via `crates/mika-agent/build.rs`. See `crates/mika-agent/CLAUDE.md` Skills System for details.
- `scripts/` — Utility scripts (sync-agent-docs.sh for crates.io publish prep)
- `Makefile` — Development workflow targets: `make build`, `make deploy` (dashboard+build+stop+install), `make test`, `make lint`, `make fmt`, `make check`
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands (`/mika` — full dev workflow, `/mika-doc-audit` — standalone documentation audit, `/mika-issue` — create a single GitHub issue, `/mika-issues` — batch-create GitHub issues)

## Versioning

- **Pre-1.0 breaking changes:** Until v1.0, breaking changes do not require backward compatibility. They are shipped as minor or patch releases (no major version bump). PRs that introduce breaking changes must document the required manual migration steps in the PR description.

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run. Integration tests for the agent loop live in `crates/mika-agent/tests/eval/` — these use `MockLlmProvider` (sequence-based, no network) via the `EvalHarness` builder to exercise the full `run_agent()` path deterministically. `EvalHarness` supports optional dependency injection via builder methods: `.embedding_client()`, `.brave_api_key()`, `.github_token()`, `.mcp_manager()` (all default `None`). `MockLlmProvider` is in `mika-common::llm::mock`, gated behind `#[cfg(any(test, feature = "test-utils"))]`. `Settings::test_defaults()` in `mika-common` provides a canonical test `Settings` constructor (also `test-utils` gated). Real-provider eval matrix tests are gated behind `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` env var — run with `cargo test -p mika-agent --test eval -- --ignored` after setting `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq` (comma-separated, or `all`). Calibration mode (`MIKA_EVAL_CALIBRATE=1`) writes ephemeral artifacts to `target/eval-calibration/`.
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Doc sync:** `docs/` is the single source of truth. `crates/mika-agent/build.rs` copies docs into `OUT_DIR` at build time via `include_str!(concat!(env!("OUT_DIR"), ...))`. Crate-local copies in `crates/mika-agent/docs/` are fallbacks for crates.io; sync them with `scripts/sync-agent-docs.sh` before publishing. CI enforces sync via the `docs-sync` job in `ci.yml` — PRs that modify `docs/` without running the sync script will fail.
- **Proactive state checking:** The system prompt instructs the agent to check existing state before any write operation to prevent duplicates after compaction. New write tools should have a corresponding query tool.
- **Grounding rule:** The system prompt prohibits the agent from claiming downstream system state unless a tool result confirms it. Reinforced in `format_callback_framing` and `SilentTrigger::Callback`.
- **Confirmation before action:** The system prompt instructs the agent to answer informational questions directly without starting multi-step workflows.
- **Context priority:** current user message > core memory > active skill context > conversation summary > conversation history > search results. See `docs/memory-classification.md`.
- **Database:** Case-insensitive COLLATE NOCASE on unique text columns. Schema v26. See `crates/mika-agent/CLAUDE.md` for schema details.
- **Secrets:** All API keys and tokens in `Settings` use `secrecy::SecretString` (`Option<SecretString>`) for compile-time exposure safety and zeroize-on-drop. Secrets are exposed at the `Settings` accessor boundary (e.g., `provider_fields()`, `agent_github_token()`) via `.expose_secret()` — downstream types use plain `String`/`&str`. `Settings` has manual `Debug` impl that redacts all secret fields. `get_effective_value()` returns `"[SET]"` for secret-flagged fields (never raw values). Exec handler executor scrubs all MIKA_* env vars from child processes. MCP child processes use `env_clear()` + allowlist. Git subprocesses scrub MIKA_* vars and set `GIT_TERMINAL_PROMPT=0`.
- **Labels:** `.github/labels.yml` is the canonical label taxonomy (type, priority, component). All issue-creation paths reference it.
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `sync_channel(512)` mpsc channel (closure-based dispatch). Clone-able, Send+Sync. `with_db` releases the mutex before calling `send()` to avoid deadlocks.

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~2815 tests)
- `cargo test -p mika-agent --test eval` — Run agent loop integration tests (eval harness)
- `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai cargo test -p mika-agent --test eval -- --ignored` — Run real-provider eval matrix (requires API keys)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, etc.)
- `cargo run --bin mika-server` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev:dashboard` — Run dashboard dev server (builds `@senara-solutions/ui` first, requires mika-server on :8080)
- `npm run build --prefix dashboard` — Build dashboard for production (sets `VITE_BASE_PATH=/dashboard/` automatically)
- `make deploy` — Build dashboard + release binaries with telemetry, stop running mika-server/mika-gateway, install to `~/.local/bin/`
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image
- `docker compose up` — Run agent + gateway (add `--profile db` for local Postgres)

## Architecture Summary

For detailed architecture of each subsystem, see the crate-level CLAUDE.md files. Key architectural decisions:

- **One container per customer** — per-customer isolation with SQLite
- **Three-layer memory model** — core memory (system prompt) + structured facts + hybrid search (FTS5 + vector). See `crates/mika-agent/CLAUDE.md`.
- **Agent loop** — max 20 tool steps, 5-min timeout, 7 post-condition guards on EndTurn (includes intent-precondition registry). See `crates/mika-agent/CLAUDE.md`.
- **Skills marketplace** — git-based distribution, per-provider/model prompt variants, dependency resolution. See `crates/mika-agent/CLAUDE.md`.
- **Unified task engine** — SQLite-backed scheduler, callback/resume lifecycle, team suspend/resume. See `crates/mika-agent/CLAUDE.md`.
- **HTTP server (mika-server)** — Axum, two auth layers, embedded dashboard. See `crates/mika-agent/CLAUDE.md`.
- **Gateway** — Telegram + GitHub webhook routing, A2A proxy, Postgres. See `crates/mika-gateway/CLAUDE.md`.
- **A2A protocol** — v0.3, JSON-RPC, task state machine. See `crates/mika-a2a/CLAUDE.md`.
- **Knowledge Graph** — Three-layer KG (domain/lexical/subject) in SQLite. Domain graph builder (deterministic, startup) projects skills/tools/agents/problem_types into `kg_entities`/`kg_relationships`. Lexical ingestor (#689) chunks `docs/solutions/**/*.md` per-agent into `kg_chunks` + FTS5/vec search. Subject extractor (#690) runs LLM-based NER to extract entities and fact triples from chunks into `kg_subject_entities`/`kg_subject_relationships` with provenance tracking. Extraction runs async at startup (background per-agent) and sync on compound hook. Entity resolver (#691) bridges subject graph to domain graph via two-stage pipeline (exact-match then LLM disambiguation) into `kg_subject_resolutions`/`kg_resolutions_log`. Resolution runs async at startup and as background spawn after compound extraction. See `crates/mika-agent/CLAUDE.md`.
- **Docker images:** Multi-stage builds with BuildKit cache. `Dockerfile.agent` (95MB) for per-customer containers. `Dockerfile.gateway` for the stateless gateway. Both use rustls, non-root user `mika` (UID 1000). Release profile: LTO + strip. `docker-compose.yml` defines agent, gateway, and postgres services. **Host dependency:** `jq` is required by all skill handler scripts.
- **CI/CD:** Four GitHub Actions workflows: `ci.yml` (PR checks), `release-plz.yml` (versioning/changelog), `release.yml` (cross-platform binaries), `publish-ui.yml` (`@senara-solutions/ui` to GitHub Packages). All actions pinned to commit SHAs. CI includes a `byte-slice-lint` job that runs `scripts/check-byte-slices.sh` to prevent unsafe `&str` byte-slicing patterns that panic on multi-byte UTF-8 (#764).

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

Optional (web search):
- `MIKA_BRAVE_API_KEY` — Brave Search API key for `web_search` builtin skill (get free key at https://brave.com/search/api/)

Optional (Knowledge Graph LLM):
- `MIKA_KG_INGESTION_MODEL` — Shared fallback model for KG extraction and resolution. Format: `provider/model`. **OpenRouter is recommended** — Anthropic direct is ~10× more expensive for bulk NER and triggers a `kg_anthropic_provider` WARN at startup. Example: `openrouter/deepseek/deepseek-v3`. If unset, KG features requiring LLM calls are disabled.
- `MIKA_KG_EXTRACTION_MODEL` — Model for NER + fact-triple extraction (#690). Falls back to `MIKA_KG_INGESTION_MODEL` if unset. Task is mechanical JSON extraction — cheap/fast tier recommended.
- `MIKA_KG_RESOLUTION_MODEL` — Model for entity resolution disambiguation (#691). Falls back to `MIKA_KG_INGESTION_MODEL` if unset. Mid-tier model recommended for better judgment on ambiguous matches.
- `MIKA_KG_BATCH_BUDGET` — Per-batch LLM call cap on KG startup extraction and resolution (#757). Default `500`. Worst-case per-startup cost is `2 × N_agents × budget` (extraction batch + resolution batch, one of each per agent). Overflow emits a `kg_budget_exhausted` WARN and leaves remaining work for the next restart. `0` disables the phase entirely. Extraction idempotency (see `crates/mika-agent/src/db/kg_schema.rs` → **Idempotency key**) keeps the second and subsequent restarts free of extraction calls once marker rows are populated.

### Post-restart safety check (#757)

After KG-related deploys, four signals tell you the fix is working. The second restart after deploy is the steady-state signal — the first restart backfills NULL `source_doc_hash` rows from v26 under the budget.

- **Signal A — extraction not re-running.** `grep subject_extraction_start server.log | jq 'select(.pending_docs == 0)'` should list every agent by the second post-deploy restart. Note: does NOT imply resolver is caught up — see Signal C.
- **Signal B — budget not exhausted.** `grep kg_budget_exhausted server.log` returns zero lines on a healthy restart.
- **Signal C — resolver backlog drains over time.** `SELECT agent_id, COUNT(*) FROM kg_subject_entities se WHERE NOT EXISTS (SELECT 1 FROM kg_resolutions_log rl WHERE rl.agent_id = se.agent_id AND rl.subject_entity_id = se.id) GROUP BY agent_id;` → count trends to 0 across restarts (bounded by per-restart budget). May take multiple restart cycles for agents starting > 3,000 pending.
- **Signal D — concrete cost prediction.** With OpenRouter configured, expect ~`N_agents × budget` resolution LLM calls on the first restart + ~0 extraction calls. At OpenRouter cheap-tier pricing (~\$0.0001/call) that's **\$0.05–\$0.50 per restart** until the backlog drains. Substantially more than \$1 per restart indicates budget-guard failure, stale idempotency, or provider routing regression.

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
- `MIKA_DEV_MODE` — Enable dev mode (default: false). When true, auto-provisions well-known development agents (`mika-dev`, `mika-qa`, `mika-relay`) on startup with role-specific identity, soul, and skill assignments. mika-dev gets self-dev family skills; mika-qa gets qa-review family skills; mika-relay gets only permission-policy (haiku model for cheap permission classification). Idempotent — existing agents are never overwritten.
- `MIKA_DISABLE_BUNDLED_SKILLS` — Skip bundled skill re-sync on startup (default: false). WARNING: do not enable in production — prevents security updates to handler scripts.
- `MIKA_DISABLE_AGENT_PROVISIONING` — Skip well-known agent auto-creation on startup (default: false). When true, prevents `dev_mode` from creating or updating agent identity files, allowing manual edits to persist across restarts/deploys. Same pattern as `MIKA_DISABLE_BUNDLED_SKILLS`.

Optional (runtime observability):
- `MIKA_STORE_LLM_CALLS` — Store LLM call metadata (model, tokens, latency) in SQLite (default: true)
- `MIKA_STORE_TOOL_CALLS` — Store full tool call input/output in SQLite (default: true, 50KB cap per field)
- `MIKA_LOG_LLM_BODIES` — Log full LLM request/response bodies at debug level to log file (default: false, dev-only)

Optional (telemetry — requires `--features telemetry` build):
- `MIKA_TELEMETRY_ENABLED` — Enable OpenTelemetry trace export (default: false)
- `MIKA_OTLP_ENDPOINT` — OTLP HTTP endpoint URL with `/v1/traces` path
- `MIKA_OTLP_AUTH_HEADER` — OTLP auth header value (Base64-encoded credentials for Langfuse)

Optional (CLI -> server communication):
- `MIKA_SERVER_URL` — mika-server base URL for CLI dashboard commands (default: `http://localhost:8080`)
- `MIKA_GATEWAY_URL` — mika-gateway base URL for CLI webhook DLQ commands (default: `http://localhost:3001`)

Optional (dashboard): See `dashboard/CLAUDE.md`.

Optional (log format and files):
- `MIKA_LOG_FORMAT` — Stdout log format for mika-server and mika-gateway: `json` (default) or `pretty` (human-readable, for local dev). CLI always uses pretty.
- `MIKA_SERVER_LOG_FILE` — File path for mika-server log output (always JSON regardless of `MIKA_LOG_FORMAT`)

Gateway mode: See `crates/mika-gateway/CLAUDE.md` for gateway-specific env vars.

## Pending Work

- **Deployment:** Production deployment guide, Docker image CI, Kubernetes/cloud manifests
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `../openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `../lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.

## Workspace Context

This repo is part of the [mika-platform](../CLAUDE.md) workspace. For cross-repo navigation, development workflow, and the autonomous development loop, see `../CLAUDE.md`.

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
- Examples: `self-dev`, `claude-pilot`, `qa-review`, `permission-policy`

**Community Skills** (in `mika-skills` repo):
- Standalone skills with no engine dependencies
- Distributed via git-based marketplace
- Examples: external API integrations, utility skills

### Directory Structure (`skills/bundled/`)

```
skills/bundled/
├── self-dev/              # Main self-development orchestration (per-issue + milestone + project workflows)
├── self-dev-iterate/      # PR iteration handler for self-dev
├── self-dev-webhook-qa/   # QA webhook handler for self-dev
├── self-dev-webhook-ci/   # CI webhook handler for self-dev
├── claude-pilot/          # Claude Code integration (worktree setup, rebase-or-abort guard, claude-pilot dispatch)
├── qa-review/             # PR review skill
├── qa-review-build-callback/ # Build callback handler for QA review
├── permission-policy/     # Permission handler for claude-pilot sessions
├── resolve-pr-conflicts/  # Rebase + conflict resolution for CONFLICTING PRs
├── address-pr-comments/   # Address PR review comments
├── self-check/            # Self-check/health verification
├── build-mika/            # Build verification
├── deploy-mika/           # Deployment
├── agents-teams/          # Agent/team management
└── skill-review/          # Skill review handler
```

### Build-Time Discovery

The `build.rs` walks `skills/bundled/` and generates `BUNDLED_SKILL_MANIFESTS` containing:
- Skill name and manifest (from `skill.toml`)
- System prompt content (from `system_prompt.md`)

Skills are loaded at runtime from this generated constant — no filesystem access required.

### Adding a New Bundled Skill

1. Create `skills/bundled/<skill-name>/` directory
2. Add `skill.toml` with required fields:
   ```toml
   [skill]
   name = "<skill-name>"
   version = "0.1.0"
   keywords = ["trigger", "words"]
   ```
3. Add `system_prompt.md` with the skill's system prompt
4. Build — the skill is automatically discovered and available
