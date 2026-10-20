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
- `packages/ui/` — `@samidarko/ui` shared React component library (Vite library mode, published to npmjs.org as a public package — token-free anonymous install, mika#1386). Components: StatusBadge (six-variant: success/warning/error/info/neutral/blocked), Pagination, EmptyState (with optional action affordance), LoadingState (list/detail skeleton variants with ARIA), ErrorState (retry + details affordances with ARIA), CopyButton, MarkdownContent, TaskStatusBadge (thin adapter delegating to StatusBadge), ListRow (three-variant: static/navigable/expandable — canonical row primitive for all list/table surfaces), SelectFilter (categorical one-of-N filter dropdown), AgentFilter (thin adapter delegating to SelectFilter with consumer-injected agents prop), TimeRangeFilter (presets + custom picker, ISO 8601 emission, server-side enforcement), TokenBudgetBar (three-tier color threshold progress bar with ARIA meter semantics), CostMeter (unbounded threshold-based cost display with ARIA status semantics — two variants: full/chip), LiveRefreshToggle (auto-refresh toggle switch + LIVE badge — canonical affordance for all dashboard live-refresh surfaces). **Hand-rolled implementations of these primitives are review fails — see `packages/ui/CLAUDE.md` for enforcement rules and escape-hatch criteria.** See `packages/ui/CLAUDE.md`.
- `dashboard/` — React observability dashboard. See `dashboard/CLAUDE.md`.
- `docs/` — Public documentation (architecture, configuration, deployment, runtime-structure, skills, slash-commands, getting-started) — **single source of truth** for all docs. See [docs/runtime-structure.md](docs/runtime-structure.md) for full `~/.mika` directory layout, DB schema, and log paths.
- `docs/adr/` — Architecture Decision Records (numbered)
- `docs/architecture/` — Architecture references including `review-guide.md` (SOLID/DRY/YAGNI/KISS/Orthogonality with citations to mika code; primary consumer is `mika-arch`'s plan-review skills, but applies to any code authored or reviewed in this repo).
- `docs/design/` — Design system: `north-star.md` (the WHY behind every visual decision across the Mika ecosystem) + `luminescent-core.md` (the rulebook) + `dashboard-stitch-map.md` (Dashboard ↔ Stitch reconciliation, milestone #13 sequence, workflow agreement). Single design system across Observability Dashboard, Cloud Console, and Landing Page; consumed via `packages/ui/` (`@samidarko/ui`). The rulebook is owned by Vincent and updated via direct commits, not PRs; implementation PRs apply it but do not relitigate it.
- `docs/openapi/` — OpenAPI specs (mika-spirit.yaml, gateway.yaml)
- `docs/solutions/` — Documented solutions to past problems (bugs, best practices, workflow patterns), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`). Relevant when debugging or implementing in documented areas.
- `skills/bundled/` — Source tree for engine-coupled bundled skills discovered at build time via `crates/mika-agent/build.rs`. See `crates/mika-agent/CLAUDE.md` Skills System for details.
- `scripts/` — Utility scripts (sync-agent-docs.sh for crates.io publish prep)
- `Makefile` — Development workflow targets: `make build`, `make deploy` (dashboard+build+install+restart), `make test`, `make lint`, `make fmt`, `make check`
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands. **This repo tracks exactly four** (`/mika` — full dev workflow, `/mika-doc-audit` — standalone documentation audit, `/mika-issue` — create a single GitHub issue, `/mika-issues` — batch-create GitHub issues). The orchestration commands (`/mika-groom-ticket`, `/mika-groom-plan-only`, `/mika-groom-milestone`, `/mika-revise-plan`, …) live in `mika-platform` — removed from here deliberately by `b831cbd5` (2026-05-26) — and are seeded into each dispatch worktree by `_seed_worktree_slash_commands` (mika#1415), which also hides them from `git status`. **So a ticket opened on `senara-solutions/mika` cannot edit them, and their absence from `git ls-files .claude/commands/` while they sit on disk is the nominal state, never evidence of a deletion** (misread that way once — mika-qa on PR#1994 — then re-established from scratch twice more, by mika#2306 and mika#2001). The one command that settles it: `git log --oneline --all -- .claude/commands/`. Full reasoning: `docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md` § Effet de lecture.

## Versioning

- **Pre-1.0 breaking changes:** Until v1.0, breaking changes do not require backward compatibility. They are shipped as minor or patch releases (no major version bump). PRs that introduce breaking changes must document the required manual migration steps in the PR description.

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run. Integration tests for the agent loop live in `crates/mika-agent/tests/eval/` — these use `MockLlmProvider` (sequence-based, no network) via the `EvalHarness` builder to exercise the full `run_agent()` path deterministically. `EvalHarness` supports optional dependency injection via builder methods: `.embedding_client()`, `.brave_api_key()`, `.github_token()`, `.mcp_manager()` (all default `None`). `MockLlmProvider` is in `mika-common::llm::mock`, gated behind `#[cfg(any(test, feature = "test-utils"))]`. `Settings::test_defaults()` in `mika-common` provides a canonical test `Settings` constructor (also `test-utils` gated). Real-provider eval matrix tests are gated behind `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` env var — run with `cargo test -p mika-agent --test eval -- --ignored` after setting `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq` (comma-separated, or `all`). Calibration mode (`MIKA_EVAL_CALIBRATE=1`) writes ephemeral artifacts to `target/eval-calibration/`. KG provider comparison eval (#762) lives at `tests/eval/kg_provider_eval/` and is gated separately behind `#[ignore]` + `MIKA_EVAL_KG_PROVIDERS` (comma-separated `provider/model` strings, or `default` for the four-provider minimum set) — run with `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval`. Fixtures live in `docs/solutions/kg/eval-fixtures-2026-04-24/`; decision matrix in `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md`.
- **Dead endpoints (mika#2569):** A test that needs an endpoint **nothing answers on** takes `mika_common::dead_endpoint::DeadEndpoint::reserve()` (feature `test-utils`) and holds the guard for the whole call — never `TcpListener::bind("127.0.0.1:0")` followed by a `drop`. A released port is handed to the next `bind(:0)` of a **parallel** test, which then serves the call that was supposed to fail: measured 2026-09-28 on the required `Check` job of mika#2561, where `"artifact-output"` — a neighbouring mock's answer — came back from a port declared dead. The guard binds without `listen()` and without `SO_REUSEADDR`, so the port is **reserved** (no concurrent `bind(:0)` can receive it) and the connection is **refused** immediately; both properties are asserted at reservation rather than assumed. A non-routable address is **not** an alternative — the dispatch sandbox's egress proxy intercepts it and returns an HTTP status, never a transport error (guard mika#2495), so the test would be green on CI and red in pilot. Held by `canonical_tokens::tests::mika2569_aucune_fixture_nobtient_un_endpoint_mort_en_liberant_un_port`; when it fires, migrate the site — do not add a census entry.
- **Model calibration (#1190):** Never swap an agent's base model or skill-override model without a passing `make calibrate-<role>` run. The calibration framework at `crates/mika-agent/src/calibration/` provides role-scoped scenario suites (mika-dev: 5 scenarios anchored on #1168/#1166/#1173; mika-arch: 5 scenarios for disposition/citation/finding-list contracts; mika-qa: 5 scenarios for verdict format/AC enumeration/absence grounding/wip-rescue/replay consistency). Run: `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6`, `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6`, or `make calibrate-mika-qa MODEL=anthropic/claude-sonnet-4-6`. The `calibrate` binary produces JSON artifacts + markdown reports. Baselines live at `docs/eval/calibration/baselines/`. Every model-swap PR must include the calibration report and update the baseline.
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Doc sync:** `docs/` is the single source of truth. `crates/mika-agent/build.rs` copies docs into `OUT_DIR` at build time via `include_str!(concat!(env!("OUT_DIR"), ...))`. Crate-local copies in `crates/mika-agent/docs/` are fallbacks for crates.io; sync them with `scripts/sync-agent-docs.sh` before publishing. CI enforces sync via the `docs-sync` job in `ci.yml` — PRs that modify `docs/` without running the sync script will fail.
- **Proactive state checking:** The system prompt instructs the agent to check existing state before any write operation to prevent duplicates after compaction. New write tools should have a corresponding query tool.
- **Grounding rule:** The system prompt prohibits the agent from claiming downstream system state unless a tool result confirms it. Reinforced in `format_callback_framing` and `SilentTrigger::Callback`.
- **Confirmation before action:** The system prompt instructs the agent to answer informational questions directly without starting multi-step workflows.
- **Context priority:** current user message > core memory > active skill context > conversation summary > conversation history > search results. See `docs/memory-classification.md`.
- **Database:** Case-insensitive COLLATE NOCASE on unique text columns. Schema v56. See `crates/mika-agent/CLAUDE.md` for schema details.
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
- **Labels:** `.github/labels.yml` is the canonical label taxonomy (type, priority, component, state, sprint hooks, automation). All issue-creation paths reference it. The `ready` label is the canonical positive-consent signal for mika-dev autonomous dispatch (mika#841). The `release` label was applied by the `release-pr` CI workflow to release PRs; that workflow is disabled (mika#2048), so nothing applies it today. **An enforcement label that is not declared here fails in silence** — the sync workflow runs with `delete-other-labels: true`, so an undeclared label is deleted from the repo *and from every issue carrying it*, with no `unlabeled` event and no log (`docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`; three occurrences to date). The `dispatch:<seat>` family is guarded structurally: `scripts/check-dispatch-seats-declared.sh` compares this file against `KNOWN_DISPATCH_SEATS` (`crates/mika-agent/src/webhook_dispatch.rs`) **both ways** and fails CI (`dispatch-seats-lint`) on any divergence, so the two lists must move in the same commit (mika#2092). `dispatch:zorglub` is the Rust unknown-seat test fixture and must stay undeclared. `loop-substrate` carries a **machine consequence** since mika#2542 — it raises the pilot turn ceiling to 200 via `PILOT_LABEL_TURN_CEILINGS` — and `scripts/check-pilot-turn-ceiling-labels.sh` (CI `pilot-turn-ceiling-labels-lint`) refuses any key of that table left undeclared here.
- **Operational ledger:** Writes to `operational_items.evidence_refs` MUST go through `crates/mika-agent/src/operational/types.rs::EvidenceRef`. Direct SQL writes that bypass the Rust type are unsupported and will break the closed-enum guarantee from foundation Decision F.
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `tokio::sync::mpsc::channel(512)` bounded channel (closure-based dispatch). Clone-able, Send+Sync. `with_db` releases the mutex before calling `send().await` — async backpressure yields to the Tokio executor instead of pinning worker threads (#1258). Worker thread uses `blocking_recv()` (sync bridge for the async channel; `rusqlite::Connection` is `!Send` so the worker must be a dedicated OS thread). Closures take `&mut Database` (required by `rusqlite::Transaction` RAII — see #636).

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~3463 tests)
- `cargo test -p mika-agent --test eval` — Run agent loop integration tests (eval harness)
- `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai cargo test -p mika-agent --test eval -- --ignored` — Run real-provider eval matrix (requires API keys)
- `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval` — Run KG provider comparison eval (requires API keys for all selected providers)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, `mika kg status`, etc.)
- `mika agents reprovision <name> [--tier <t>] [--identity-only] [--dry-run] [--yes]` — Re-apply an existing agent's authoritative `identity.toml` **and** `soul.md` (mika#2230). Both, because a tier has two axes since mika#2023 and the mika#1962 guard reads either. Differential, backs up what it overwrites in `0600`, refuses a template whose `[skills].allowlist` is absent or empty, and touches neither `config.toml` nor the DB. Runbook: `docs/operator/agent-identity-reprovision.md` § 7.
- `mika agents budget [--agent <name>]` — Quel budget et quel modèle un agent fait-il **tourner**, et par quelle porte de la cascade (mika#2457). Lit le record que mika-spirit a figé à l'`init_agent` ; la première ligne porte `resolved_at`, donc la sortie dit *quand* elle a été vraie. Ne résout **rien** localement : serveur injoignable ou 404 ⇒ *« non attesté »*, aucune valeur affichée. Depuis mika#2473 elle porte aussi une ligne `code` : ce que le **dépôt** déclare pour cet agent, et si le runtime en sert un autre. C'est la commande de la sonde du § *Observabilité du budget effectif* ci-dessous.
- `cargo run --bin mika-spirit` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev:dashboard` — Run dashboard dev server (builds `@samidarko/ui` first, requires mika-spirit on :8080)
- `npm run build --prefix dashboard` — Build dashboard for production (sets `VITE_BASE_PATH=/dashboard/` automatically)
- `make deploy` — Full deploy: build dashboard + release binaries with telemetry, install to `~/.local/bin/`, restart services. Prints the built SHA and warns when local HEAD is behind `origin/main`. A change under `skills/bundled/` reaches an agent **only through this rebuild** — see [Deploying a bundled-skill change](#deploying-a-bundled-skill-change-mika2340) below.
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image
- `docker compose up` — Run agent + gateway (add `--profile db` for local Postgres)
- `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6` — Run mika-dev calibration suite
- `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6` — Run mika-arch calibration suite
- `make calibrate-mika-qa MODEL=anthropic/claude-sonnet-4-6` — Run mika-qa calibration suite
- `scripts/pr-origin-report.sh --since <date> --until <date>` — Merged PRs over a window, split by origin (see below)

### Deploying a bundled-skill change (mika#2340)

**`~/.mika/skills/` is a projection of the binary, not of the checkout.** Nothing
reads `skills/bundled/` at runtime: `build.rs` compiles it into
`BUNDLED_SKILL_MANIFESTS`, and the library every agent symlinks into is written
from that constant by `seed_bundled_skill_library()`. An edited prompt in the
working tree is invisible to every agent until a binary that embeds it runs a
seed. The chain is **rebuild → seed → read**:

1. `make deploy` embeds the new manifest set and its hash.
2. Any `mika` subcommand routed through `init_base_for_agent`, the daemon at
   startup, and `mika skills --agent <a> update` call
   `seed_bundled_skills_if_needed()` — hash-gated re-extraction of the library,
   then re-materialisation of the agent's symlinks. Since mika#2340 `update`
   resyncs the library **content**; before, it refreshed the symlinks alone and
   printed "Refreshed bundled-skill symlinks.", which read as a content refresh
   while `system_prompt.md` stayed a week old (2026-09-16, deploy of #2339).
3. `system_prompt.md` is lazy-loaded through the symlink on the next turn.

Running `update` from a stale binary re-extracts a stale library and reports
success honestly — the hash it checks is its own. The remedy is step 1, never a
second run of step 2.

**Probe:** `cat ~/.mika/skills/.manifest-writer` — a JSON sidecar
(`version`, `git_hash`, `attested_at`, `manifest_hash`, `extracted`) written
atomically on **every** seed pass, including the ones the hash gate
short-circuits, so a rebuild that touched no bundled prompt never shows an older
sha on a conformant library. `mika skills --agent <a> update` prints the same
record as `attested by: mika <version> (<sha>) at <instant>`. After a deploy that
sha must be the one you just built; if it is and the agent's resolved prompt
still differs from the checkout, the defect is upstream (build-time discovery or
extraction), not in the seed. `git_hash: "unknown"` means a binary built outside
a git checkout, not a failed deploy. A seed by a binary whose `version` is
strictly lower than the recorded one emits `bundled_library_downgrade` (WARN)
and proceeds — the daemon and the CLI both write this library and nothing orders
them; the guard names the rollback, it does not refuse it. Full reference in
`docs/skills.md § Deploying a change to a bundled skill`.

### PR origin (mika#2026)

Whether a merged PR came out of the autonomous loop or was opened by hand is a
**fact stamped by its producer**, never reconstructed afterwards. `dispatch-lib.sh`
applies `origin:loop` in shell (`_stamp_pr_origin`) at each of the three points
where it holds a PR it just produced. Branch name, author, and merge time are all
identical between the loop and by-hand work — inferring from them fails exactly on
the day the answer matters.

Read it with `scripts/pr-origin-report.sh`. Two rules govern the reading:

- **An unmarked PR reads "unknown", never "by hand."** A default that looks like an
  answer is how an instrument lies.
- **Absence of the label only means something after the marker went live.** The
  cut-off is the instant the producer first stamped anything, recorded once by
  `_record_pr_origin_epoch` in `~/.mika/state/pr-origin-epoch`; override with
  `MIKA_PR_ORIGIN_EPOCH`. With no cut-off, the report classifies nothing and says
  so — and the cut-off is born of the first real stamp, not of `make deploy`. It is deliberately *not* a file mtime: `seed_support_dirs` rewrites the
  installed `dispatch-lib.sh` on every daemon start, so an mtime would track the
  last restart and walk the cut-off forward all day. The test is on a PR's
  **opening** date, not its merge — a PR in flight across the cutover could never
  have been stamped, so it reads unknown rather than not-loop.

Bounds: `--since`/`--until` take `YYYY-MM-DD` (whole days, UTC) or a full
`YYYY-MM-DDTHH:MM:SSZ` instant; anything else is refused rather than silently
mis-answered. The report also refuses (exit 3) when `--limit` filled the page, so
a truncated fetch can never read as a real count.

When opening a PR by hand — orchestrator or spawn — apply `origin:manual` or
`origin:spawn` yourself (`gh pr edit <n> --add-label origin:manual`). Nothing does
it for you: only the loop half has a structural producer.

## Architecture Summary

For detailed architecture of each subsystem, see the crate-level CLAUDE.md files. Key architectural decisions:

- **One container per customer** — per-customer isolation with SQLite
- **Three-layer memory model** — core memory (system prompt) + structured facts + hybrid search (FTS5 + vector). See `crates/mika-agent/CLAUDE.md`.
- **Agent loop** — max 20 tool steps, 5-min timeout, 12 post-condition guards on EndTurn (includes intent-precondition registry, required-suffix-line guard, required-finding-list guard, milestone-close-claim guard, assert-grounded guard, and the unacknowledged-send-failure guard). See `crates/mika-agent/CLAUDE.md`.
- **Skills marketplace** — git-based distribution, per-provider/model prompt variants, dependency resolution. See `crates/mika-agent/CLAUDE.md`.
- **Unified task engine** — SQLite-backed scheduler, callback/resume lifecycle, team suspend/resume. See `crates/mika-agent/CLAUDE.md`.
- **HTTP server (mika-spirit)** — Axum, two auth layers, embedded dashboard. See `crates/mika-agent/CLAUDE.md`.
- **Gateway** — Telegram + GitHub webhook routing, A2A proxy, Postgres. See `crates/mika-gateway/CLAUDE.md`.
- **A2A protocol** — v0.3, JSON-RPC, task state machine. See `crates/mika-a2a/CLAUDE.md`.
- **Knowledge Graph** — Three-layer KG (domain/lexical/subject) in SQLite. Domain graph builder (deterministic, startup) projects skills/tools/agents/problem_types/concepts into `kg_entities`/`kg_relationships`. Concept entities (#928) use hierarchical naming (`concept:cross-repo:*`, `concept:infra:*`) to cover cross-repo workflow and Helm/K8s infrastructure concepts for mika-platform and mika-cloud corpora. Lexical ingestor (#689) chunks `docs/solutions/**/*.md` per-agent into `kg_chunks` + FTS5/vec search. Subject extractor (#690) runs LLM-based NER to extract entities and fact triples from chunks into `kg_subject_entities`/`kg_subject_relationships` with provenance tracking. Extraction runs async at startup (background per-agent) and sync on compound hook. Entity resolver (#691) bridges subject graph to domain graph via two-stage pipeline (exact-match then LLM disambiguation) into `kg_subject_resolutions`/`kg_resolutions_log`. Resolution runs async at startup and as background spawn after compound extraction. Per-agent KG scoping via `identity.toml` `[kg]` section (#778) — `enabled` (default true) and `docs_root` (optional) control per-agent corpus isolation; agents with matching `docs_root` share extraction via `docs_root_hash` (v27). **KG topology (#800):** mika-arch is the sole KG consumer among well-known agents; mika-dev and mika-qa are provisioned with `[kg].enabled = false` (zero `query_knowledge_graph` usage — retrieval goes through `search_memory`). Re-enable per-agent with one identity.toml edit + restart if a dev/qa flow needs KG. See `crates/mika-agent/CLAUDE.md`.
- **Docker images:** Multi-stage builds with BuildKit cache. `Dockerfile.agent` (95MB) for per-customer containers. `Dockerfile.gateway` for the stateless gateway. Both use rustls, non-root user `mika` (UID 1000). Release profile: LTO + strip. `docker-compose.yml` defines agent, gateway, and postgres services. **Host dependency:** `jq` is required by all skill handler scripts.
- **CI/CD:** Five GitHub Actions workflows: `ci.yml` (PR checks), `pr-body-validation.yml` (PR body validation), `release-pr.yml` (versioning/changelog via release-please — **disabled** since 2026-08-29, `workflow_dispatch` only; see mika#2048), `release.yml` (cross-platform binaries), `publish-ui.yml` (`@samidarko/ui` to npmjs.org as a public package). All actions pinned to commit SHAs. CI includes a `byte-slice-lint` job that runs `scripts/check-byte-slices.sh` to prevent unsafe `&str` byte-slicing patterns that panic on multi-byte UTF-8 (#764), a `loop-select-lint` job that runs `scripts/check-loop-select.sh` to reject `tokio::select!` inside `run_loop`'s body — the deadline-check guarantee depends on iteration-top semantics not being shadowed (#848), an `a2a-timeout-literal-lint` job that runs `scripts/check-a2a-timeout-literals.sh` to reject a call budget written as a literal at the site that bounds it on the a2a path — the predicate is on the **bounding site**, never on the value, because "duration ≥ 60 s" also catches cache TTLs and JWT lifetimes and an allowlist full of those is where the regression would pass unnoticed (#2309), a `landing-tokens-lint` job that runs `scripts/check-landing-tokens.sh` to reject any colour literal under `site/src/` or `site/index.html` — the landing consumes the rulebook §2 palette from `@samidarko/ui/theme.css`, so a hardcoded colour is one that stops tracking the rulebook silently, and the rule is on the **shape** rather than on a list of superseded values, because the realistic regression is not retyping `#7c6af7` but writing a purple that looked close enough (mika#1804; the three macOS window-chrome pips of the terminal mockups are the named by-value exception, never a by-file one), a `suite-wiring-lint` job that runs `scripts/check-suite-wiring.sh` to reject a test suite present under `scripts/` or `skills/bundled/_shared/` that **no workflow reaches** — directly, or through a `make` target a workflow names, prerequisites and `$(MAKE)` recursion included — and that `scripts/suite-wiring.tsv` does not declare. A guard the pipeline never runs refuses nothing: it passes review, it passes CI by construction, and the defect it would have caught lands in production (mika#2578 once, mika#2752 twice, doctrine written from the first occurrence and enforced by nothing). The registry holds the **residue only**, four TAB-separated columns `path`/`mode`/`reason`/`ticket`, with `mode` a closed vocabulary — `operator` (needs the outside world, or is a deliberate instrument) or `unwired` (runs offline, nothing runs it: that is the backlog, and the lint's own `unwired=<n>` report field is its size — **zero since mika#2793 wired all 25**, four of which turned out red on their first run, so a new `unwired` line opens a backlog entry and carries its own Fire-Disposition ticket). The comparison is bidirectional, so an entry for a suite CI has since reached **fails** — the gesture that makes an exception pointless is the gesture that makes it expire, which is what keeps this from becoming the allowlist mika#2309 condemns. "Needs bwrap" is not "needs the outside world": a suite that skips cleanly is wirable (ci.yml:133-149). Reachability through another suite's shell *body* is deliberately not proven — under-approximating yields false reds, never false greens (mika#2761), and a `docker-build` job that builds all Dockerfiles (agent, gateway, mika-os, mika-runtime-server, mika-runtime-gateway, mika-runtime-cli, mika-runtime-all) on every PR to catch structural bugs before merge. **PR Body Validation (#527):** `pr-body-validation.yml` runs `scripts/check-pr-body-consistency.sh` on every `pull_request` event (opened, edited, synchronize). Two checks: (a) closure-consistency — when the PR body declares `Closes #N`, the script walks #N's formal sub-issues via GitHub GraphQL `trackedIssues`; if any are OPEN and not acknowledged, the gate hard-fails (`exit 1`); (b) follow-up tracker — when the body contains a deferral trigger phrase (e.g., "will be fixed in a follow-up", "deferred to a separate PR"), a `Tracked in: <ref>` line naming the tracker issue/PR is required. To resolve failures: add `Tracked in: senara-solutions/<repo>#<number>` lines to the PR body for each deferred item, or close the sub-issues in the same PR.

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

The full reference lives in [`docs/operator/environment-variables.md`](docs/operator/environment-variables.md): every `MIKA_*` variable, its default, and the operator surfaces, probes and STOP switches that go with it. Search it by variable name or by ticket number (`grep -n 'MIKA_FOO\|mika#1234' docs/operator/environment-variables.md`); `.env.example` lists the variables.

**Document a new variable or operator surface THERE, not here.** This file is loaded whole into every pilot's prompt. It reached 825 KB and pilots on this repo died on turn 1 ("Prompt is too long"). `crates/mika-common/tests/claude_md_size.rs` fails the build when any `CLAUDE.md` in the repository goes over the cap.

## Pending Work

- **Deployment:** Production deployment guide, Docker image CI, Kubernetes/cloud manifests
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Workspace Context

This repo is part of the [mika-platform](../CLAUDE.md) workspace. For cross-repo navigation, development workflow, and the autonomous development loop, see `../CLAUDE.md`.

**Auto-groom on dispatch (mika#996):** The autonomous loop now auto-grooms ungroomed `ready`-labelled or milestone-child tickets before dispatching them to `dev-pilot`. When a ticket reaches dispatch (via `ready` label webhook or milestone-cascade M4) without a `Plan: docs/plans/` callout in its issue body, mika-dev dispatches `dev-groom` first (two-pass architect review via `mika-arch-groom-ticket`). On `Verdict: GROOMED`, the handler re-enters the dispatch flow and fires `dev-pilot`. On `Verdict: ESCALATE`, dispatch halts and surfaces to operator. Grooming and dispatch run serially as two phases of the same child task. Orchestrator-manual `/mika-groom-ticket` remains available for free-text dispatch and human-driven grooming.

**Pilot flights end lean (mika#2694).** In a session started by the loop with `/ce-work <plan>`, the shipping workflow skips two steps, because mika-qa reviews every PR at code level and the turn ceiling cannot hold them:
- **Simplification threshold: none.** Do not invoke `ce-simplify-code`.
- **Code review: delegated to mika-qa.** Do not invoke `ce-code-review`; write `Code review: delegated to mika-qa (mika#2694)` in the PR body. Still run the full test suite, clippy, the repo lints and `scripts/verify-pipeline.sh origin/main`, then commit, push and open the PR. Interactive `/mika` keeps its review step.

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
