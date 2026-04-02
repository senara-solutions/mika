# Changelog

All notable changes to this project will be documented in this file.

## [0.3.2](https://github.com/senara-solutions/mika/releases/tag/v0.3.2) — 2026-04-02

### Added

- *(agent)* mika-dev auto-retry on fixable QA blocks (#377)
- *(agent)* persist work item metadata at callback time (#376)

### Fixed

- *(agent)* raise callback max_steps to 20 with continuation turn fallback (#378)
- *(ci)* use build.rs + OUT_DIR for dashboard asset embedding (#387)
- *(ci)* use pinned Rust toolchain in release-plz workflow (#394)
- clear GH_TOKEN identity separation — active removal + defense-in-depth scrub (#388)

## [0.3.1](https://github.com/senara-solutions/mika/releases/tag/v0.3.1) — 2026-03-31

### Fixed

- remove remaining fallback references in config field doc and reference table
- remove MIKA_INVESTIGATE_GITHUB_TOKEN fallback from agent_github_token()

## [0.3.0](https://github.com/senara-solutions/mika/releases/tag/v0.3.0) — 2026-03-30

### Added

- [#329] add agent eval & testing harness with MockLlmProvider
- [#289] add MIKA_GITHUB_TOKEN for agent GitHub operations
- *(agent)* surface parent_task_id in callback framing and simplify to generic routing
- *(agent)* inject active work items into callback turn context
- *(agent)* add context priority semantics and search_tool_history tool
- add runtime observability — LLM calls, tool calls, skills loading
- *(agent)* add proactive task health awareness and self-knowledge of task lifecycles
- *(agent)* promote delegate session channel_type from "system" to "delegate"
- *(skills)* add [llm] section to skill.toml for per-skill provider/model override
- implement model-level skill variant resolution
- *(llm)* per-provider LLM configuration ([#239](https://github.com/senara-solutions/mika/pull/239))
- claude-pilot dashboard integration — structured run metadata + Dev Runs page
- *(db)* schema v14 — add metadata column to tasks table
- *(a2a)* implement A2A protocol v0.3 for agent-to-agent communication
- [#198] embed dashboard SPA in mika-server binary
- *(skills)* raise prompt snippet size limit and make it configurable per-skill
- add --last-run flag and enhance teams log output
- [#182] add browser control via Playwright MCP integration
- add public documentation site with Starlight
- *(server)* add paginated tasks and team-runs dashboard API endpoints
- custom compact log formatter with startup banner for pretty mode
- add MIKA_LOG_FORMAT env var for human-readable server/gateway logs
- strengthen trace_id structural linkage (schema v11)
- team workspace restructure and CLI unification
- delegated agent Telegram delivery, agent identification, and reply routing
- *(skill)* add Google Workspace builtin skill via gws CLI ([#75](https://github.com/senara-solutions/mika/pull/75))
- *(tools)* add smart work item status transitions ([#257](https://github.com/senara-solutions/mika/pull/257))
- *(tui)* add /provider slash command ([#239](https://github.com/senara-solutions/mika/pull/239))
- *(oauth)* add Anthropic OAuth PKCE token exchange ([#232](https://github.com/senara-solutions/mika/pull/232))
- *(llm)* add first-class MiniMax, Qwen, and Kimi provider prefixes
- add skill dependency resolution at install time ([#216](https://github.com/senara-solutions/mika/pull/216))
- *(skills)* add local source support for mika skills install with --link mode
- [#196] show previous run context in TUI when using --last-run or --run-id
- add --model flag to `mika ask` and `mika chat` for one-shot LLM override
- add guided wizards for agent and team creation
- [#178] dashboard markdown rendering & session lifecycle fixes
- [#71] refactor all consumers to use LlmProvider trait
- add --format text|json flag to mika ask

### Changed

- [#329] address review findings — derive Clone on LlmError, prune unused APIs
- simplify CacheControl and SystemContentBlock types
- [#350] extract timezone helpers and improve type safety
- *(skills)* address review findings — use named constants, remove redundant check
- resolve code review findings for task health awareness
- address review findings — remove weak test and trim doc comment
- address review findings — remove duplicated llm field, simplify override logic
- *(server)* hoist tool_defs conversion before investigation loop
- *(a2a)* consolidate v12→v13 and v13→v14 migrations into single v12→v13
- *(a2a)* replace parallel a2a tables with orthogonal persistence
- address code review findings for embedded dashboard
- address review findings for failed callback delivery
- **BREAKING** convert all timestamps from Unix i64 to ISO 8601 TEXT strings
- **BREAKING** unify LLM API key — remove MIKA_ANTHROPIC_API_KEY
- address review findings — docs, dispatcher helper, comments
- resolve code review findings (670-673)
- remove redundant telegram_chat_id from TeamAgentParams
- remove builtin calendar skill
- remove MIKA_GOOGLE_TOKEN, use native gws keyring auth
- extract shared CLI helpers and fix flag=value bypass
- *(tui)* address code review findings
- *(oauth)* address code review findings
- rename --session to --session-id and --parent-task to --parent-task-id
- [#71] rename claude_model/claude_max_tokens to llm_model/llm_max_tokens

### Documentation

- [#350] sync crate-local docs after architecture update
- fix stale three-level fallback comments in agent.rs and index.rs
- update dashboard dev command to use dev:dashboard script
- sync crate-local docs for v0.1.7 publish
- add pre-1.0 breaking changes policy
- update documentation for team workspace restructure

### Fixed

- deprecate MIKA_LLM_API_KEY and fix OAuth setup env var mismatch
- add Claude Code identity headers for OAuth subscription tokens
- *(llm)* enable Anthropic prompt caching for agent LLM calls
- address review findings for tag stripping
- strip internal context/metadata tags from LLM response display
- [#350] address QA HOLD-2 and HOLD-3 on timezone reminder PR
- [#350] add timezone support to reminders to prevent off-by-one day errors
- use per-agent LLM provider in server mode ([#323](https://github.com/senara-solutions/mika/pull/323))
- [#346] inject MIKA_GITHUB_TOKEN as GH_TOKEN in run_gh for platform identity separation
- add CI gate for crate-local docs sync drift
- *(skills)* fail loudly when always_on skill prompt exceeds size limit
- [#321] restore dashboard endpoints in OpenAPI spec via utoipa annotations
- regenerate OpenAPI spec to include dashboard endpoints
- *(db)* escape LIKE metacharacters in keyword search
- [#303] address review findings in migration and dedup response
- [#303] make create_work_item idempotent with reference_url dedup
- [#285] use per-agent LLM config in team engine
- *(dashboard)* include github_issue-sourced dev runs in dashboard
- clippy unnecessary_map_or with telemetry feature
- resolve code review findings (P1 + P2)
- UTF-8 safe truncation and warn on observability DB write failures
- *(agent)* enforce tool execution before accepting assistant responses ([#270](https://github.com/senara-solutions/mika/pull/270))
- *(agent)* prevent callback processing race and add workflow-aware triggers
- *(agent)* add grounding guardrail to prevent downstream state hallucination
- persist delegate_task messages in delegate's session for observability
- collapse nested if let to satisfy clippy collapsible_if
- remove provider-level prompt layer from skill variant resolution
- prevent dashboard asset regression on deploy
- review fixes for per-provider skill variants ([#241](https://github.com/senara-solutions/mika/pull/241))
- *(server)* use configured LLM provider in investigation panel ([#224](https://github.com/senara-solutions/mika/pull/224))
- *(tools)* improve update_core_memory schema for non-Anthropic models
- filter non-LLM spans from Langfuse and add gen_ai semantic conventions
- *(a2a)* remove dual-write message persistence and update solution doc
- *(a2a)* resolve code review findings from A2A protocol PR
- guard update_task_failed against terminal states and add signal distinction
- skip stale failed callbacks to prevent conversation flooding
- runtime dashboard toggle API and TUI footer buttons
- embedded dashboard white page and add root landing route
- [#203] deliver failed callback tasks to agent
- clippy cmp_owned warning in cron test
- link callback tasks to work items and document claude-pilot security fixes
- strengthen confirmation-before-action guardrail for status questions
- make TimelineRow.agent_id optional to handle team_workspace NULL values
- [#182] address pattern review findings
- [#182] address review findings — SSRF guidance and credential clarity
- address review findings — UTF-8 safe truncation, DRY filter builders
- observability polish — request_id linkage, session cleanup ([#162](https://github.com/senara-solutions/mika/pull/162))
- inject referenced run context into orchestrator prompt via --run-id
- wire trace_id through callbacks and team resume, add team_workspace to unified_timeline
- address code review findings for team workspace restructure
- pass agent_name in CLI sender and auto-relay delegate text responses
- add diagnostic tracing to delegate send_message flow and improve tool description
- pass explicit chat_id to delegate agent sender for correct Telegram prefixing
- *(tui)* improve /clear, /provider, /model slash command reliability
- use workflow-aware callback framing in TUI chat path ([#269](https://github.com/senara-solutions/mika/pull/269))
- scan parent directory for sibling skills during dependency resolution
- resolve clippy collapsible_if warning in --link validation
- /rewind N not updating TUI display
- [#196] address review findings for previous run context display
- [#178] address review findings — session lifecycle and iteration numbering
- plumb --run-id through chat --team path
- suppress stderr logs for mika ask command

## [0.2.0](https://github.com/senara-solutions/mika/releases/tag/v0.2.0) — 2026-03-20

### Added

- [#198] embed dashboard SPA in mika-server binary
- [#71] refactor all consumers to use LlmProvider trait
- add LlmProvider trait and multi-provider abstractions
- custom compact log formatter with startup banner for pretty mode
- strip file paths, line numbers, and targets from pretty log output
- add MIKA_LOG_FORMAT env var for human-readable server/gateway logs
- team workspace restructure and CLI unification
- *(skill)* add Google Workspace builtin skill via gws CLI ([#75](https://github.com/senara-solutions/mika/pull/75))
- add skill dependency resolution at install time ([#216](https://github.com/senara-solutions/mika/pull/216))
- *(skills)* raise prompt snippet size limit and make it configurable per-skill
- add --last-run flag and enhance teams log output
- [#182] add browser control via Playwright MCP integration
- add public documentation site with Starlight
- *(server)* add paginated tasks and team-runs dashboard API endpoints
- strengthen trace_id structural linkage (schema v11)
- delegated agent Telegram delivery, agent identification, and reply routing
- *(skills)* add local source support for mika skills install with --link mode
- [#196] show previous run context in TUI when using --last-run or --run-id
- add --model flag to `mika ask` and `mika chat` for one-shot LLM override
- add guided wizards for agent and team creation
- [#178] dashboard markdown rendering & session lifecycle fixes
- add --format text|json flag to mika ask

### Changed

- **BREAKING** unify LLM API key — remove MIKA_ANTHROPIC_API_KEY
- [#71] rename claude_model/claude_max_tokens to llm_model/llm_max_tokens
- remove MIKA_GOOGLE_TOKEN, use native gws keyring auth
- address code review findings for embedded dashboard
- address review findings for failed callback delivery
- **BREAKING** convert all timestamps from Unix i64 to ISO 8601 TEXT strings
- address review findings — docs, dispatcher helper, comments
- resolve code review findings (670-673)
- remove redundant telegram_chat_id from TeamAgentParams
- remove builtin calendar skill
- extract shared CLI helpers and fix flag=value bypass
- rename --session to --session-id and --parent-task to --parent-task-id

### Documentation

- add pre-1.0 breaking changes policy
- update dashboard dev command to use dev:dashboard script
- sync crate-local docs for v0.1.7 publish
- update documentation for team workspace restructure

### Fixed

- address code review findings from PR #193
- [#71] address review findings for multi-provider LLM
- [#71] address review findings for multi-provider LLM
- address code review findings for team workspace restructure
- guard update_task_failed against terminal states and add signal distinction
- skip stale failed callbacks to prevent conversation flooding
- runtime dashboard toggle API and TUI footer buttons
- embedded dashboard white page and add root landing route
- [#203] deliver failed callback tasks to agent
- clippy cmp_owned warning in cron test
- link callback tasks to work items and document claude-pilot security fixes
- strengthen confirmation-before-action guardrail for status questions
- make TimelineRow.agent_id optional to handle team_workspace NULL values
- [#182] address pattern review findings
- [#182] address review findings — SSRF guidance and credential clarity
- address review findings — UTF-8 safe truncation, DRY filter builders
- observability polish — request_id linkage, session cleanup ([#162](https://github.com/senara-solutions/mika/pull/162))
- inject referenced run context into orchestrator prompt via --run-id
- wire trace_id through callbacks and team resume, add team_workspace to unified_timeline
- pass agent_name in CLI sender and auto-relay delegate text responses
- add diagnostic tracing to delegate send_message flow and improve tool description
- pass explicit chat_id to delegate agent sender for correct Telegram prefixing
- scan parent directory for sibling skills during dependency resolution
- resolve clippy collapsible_if warning in --link validation
- /rewind N not updating TUI display
- [#196] address review findings for previous run context display
- [#178] address review findings — session lifecycle and iteration numbering
- plumb --run-id through chat --team path
- suppress stderr logs for mika ask command

## [0.1.6](https://github.com/senara-solutions/mika/releases/tag/v0.1.6) — 2026-03-13

### Added

- *(dashboard)* add copy-to-clipboard and investigation GitHub issue creation
- add Acceptance Criteria section to all issue creation channels
- unified GitHub label taxonomy with auto-sync
- *(skills)* add label operation docs and keywords to github skill
- *(dashboard)* add session ID prefix search to sessions page
- fill dev workflow gaps — dashboard CI, supply chain security, issue linking, smoke tests
- *(tui)* persist /model slash command to config.toml
- *(tui)* add Alt+Enter as fallback for multi-line input

### Changed

- [#115] simplify progressive truncation to single pass
- *(agent)* resolve code review findings for skill dependencies
- tighten transparency rule wording per code review
- *(skills)* add non-zero exit signal to tool history and prompt guidance
- *(tui)* extract placeholder constants, fix Esc handler and ui.rs renderer
- *(cli)* use exhaustive match arms for compile-time safety

### Documentation

- advertise Alt+Enter as primary multi-line input method

### Fixed

- *(config)* rename github_token to investigate_github_token and improve setup
- [#144] set success=false when non_zero_exit is true in tool call metadata
- add tilde (~) home directory expansion to file tools ([#145](https://github.com/senara-solutions/mika/pull/145))
- *(server)* add Rust context to investigation agent system prompt
- [#115] truncate tool_calls metadata per-field instead of dropping tail entries
- *(skills)* address review findings for label docs
- *(agent)* add tool_history guardrail to prevent skipping actions ([#135](https://github.com/senara-solutions/mika/pull/135))
- *(agent)* add skill dependency resolution and unsolicited action guard ([#134](https://github.com/senara-solutions/mika/pull/134))
- *(agent)* migrate run_gh to Rust builtin handler with JSON array protocol ([#119](https://github.com/senara-solutions/mika/pull/119))
- *(agent)* add transparency rule for non-zero exit codes in responses
- *(trace)* fix broken trace messages endpoint and missing trace_id propagation
- *(investigate)* address review findings — input limits, repo validation, UX consistency
- *(skills)* return output on non-zero exit instead of discarding it
- *(tui)* make history navigation team-mode-aware for placeholder text
- *(cli)* scope --agent and --team flags to relevant subcommands only ([#102](https://github.com/senara-solutions/mika/pull/102))

## [0.1.5](https://github.com/senara-solutions/mika/releases/tag/v0.1.5) — 2026-03-11

### Added

- out-of-sandbox file writes and task completeness prompts
- *(cli)* add mika doctor and config set/get/list commands
- *(common)* add serde defaults to TeamDefinition for flexible parsing
- *(server)* add rewind preview and execute API endpoints
- *(agent)* add conversation rewind engine with preview and execute
- *(agent)* schema v9 and audit trail completeness for conversation rewind
- *(agent)* add generic work item tracking with create_work_item, update_task_status, list_work_items tools
- *(teams)* add conversation continuity across team runs
- *(prompt)* instruct agent to check own files before answering self-knowledge questions
- *(dashboard)* convert team session view to conversational timeline
- *(tools)* add read-only introspection tools for agent-native parity
- *(skills)* respect user overrides for builtin skill always_on flag ([#73](https://github.com/senara-solutions/mika/pull/73))
- *(server)* add investigation SSE endpoint with read-only agent loop
- *(agent)* add create_team, delete_team, update_team tools
- *(agent)* enrich list_teams output with full team configuration
- *(agent)* add create_team guidance to system prompt
- *(db)* add mention_count to people table
- *(agent)* add create_agent tool for runtime agent creation
- *(dashboard)* add React observability dashboard
- *(server)* add dashboard REST API endpoints
- *(observability)* thread trace_id through all write paths
- *(db)* schema v5 — rename memory_events to audit_events, add trace_id columns
- *(reminders)* add periodic reminder support via cron_expr
- *(cli)* add /undo and /rewind TUI slash commands
- *(cli)* add --session flag to mika ask for session reuse
- enhance mika setup with multi-secret wizard, proper TOML parsing, and TTY guard
- *(tui)* add textarea selection rendering and mouse support
- *(tui)* add mouse-based text selection and clipboard copy
- *(cli)* improve /skills display with grouped columnar layout

### Changed

- address code review findings for doctor and config commands
- simplify config cascade from 6 layers to 4 sources with dotenvy .env support
- *(teams)* simplify based on code review findings
- *(prompt)* trim filler paragraph from self-knowledge skill prompt
- *(prompt)* tighten multi-action and continuity instructions for token efficiency
- *(reminders)* unify NewTask construction and add min interval guard
- *(tui)* unify textarea selection highlighting via post-processing
- *(tui)* extract shared unicode-width wrapping iterator
- *(tui)* derive Ord on TextPosition, clear selection on keypress, add u16 guards

### Documentation

- update documentation for recent changes
- add runtime structure reference for ~/.mika layout, DB schema, and logs
- update documentation for recent changes
- add callback result display plan from PR #92

### Fixed

- harden dotenv and setup — newline injection, atomic writes, non-interactive mode
- move trace.rs to mika-common to fix telemetry feature build
- *(rewind)* code review fixes and documentation updates
- *(rewind)* inject context marker after rewind to prevent agent confabulation
- *(rewind)* cross-session support and code review improvements
- *(agent)* drop unified_timeline view before tasks table rebuild in v7→v8 migration
- *(agent)* address code review findings for work item tracking
- *(agent)* truncate tool call metadata to prevent silent entry drops
- *(ci)* regenerate OpenAPI spec to match utoipa annotations
- *(dashboard)* consolidate token env var to VITE_MIKA_DASHBOARD_TOKEN
- *(teams)* address code review findings
- *(teams)* store correct agent_id in team session messages
- *(tui)* display callback task results as system messages instead of 'You:'
- *(prompt)* guide agent to use update_skill for always_on changes
- make is_bundled_skill case-insensitive and deduplicate override logic
- *(server)* use char-based truncation in investigation tools to avoid UTF-8 panics
- *(dashboard)* store full tool call data and add quick-copy pills
- *(dashboard)* resolve code review findings from observability dashboard
- suppress too_many_arguments clippy warnings for trace_id params
- address code review findings from PR #88
- *(db)* migrate commitments to partial unique index for status-aware dedup
- *(db)* add DB-constraint duplicate detection for reminders and events
- *(clippy)* collapse nested if-let in create_reminder
- *(prompt)* add proactive state checking before write operations
- *(prompt)* add multi-action batching and conversation continuity guidance
- exclude team messages from TUI chat history
- *(cli)* validate session ownership for --session flag
- *(tui)* correct TextPosition comparison after Ord derive
- *(cli)* show error details when team loading fails

### Performance

- *(server)* combine data+count queries and remove response type ceremony
- *(tui)* optimize text selection rendering and layout caching

### Security

- *(server)* separate dashboard token from mutation endpoints

## [0.1.4](https://github.com/senara-solutions/mika/releases/tag/v0.1.4) — 2026-03-07

### Added

- add /mika-issue and /mika-issues Claude Code commands
- unified task engine (schema v1 baseline)
- wire OTel export into CLI, fix OTLP endpoint docs to require /v1/traces
- *(observability)* add feature-flagged OpenTelemetry/OTLP export with Langfuse support
- *(observability)* add tracing spans to agent loop, Claude API, team engine, and server
- *(cli)* add --team option for TUI team mode
- *(task-engine)* mark callback tasks delivered after dispatch
- *(tui)* callback delivery polling and loop prevention
- *(db)* add delivered status and tool_result role (schema v2)
- team-aware async callbacks with long_running skill flag
- implement task engine gap analysis — create_task, cancel_task, resume_agent, callback endpoint
- resolve todos 425, 455, 476 — builtins, team progress, and type constants
- *(observability)* add TeamPhase enum, new TeamEvent variants, and TUI split-pane dashboard
- *(teams)* inject conversation history into orchestrator context
- *(teams)* replace TOML history with SQLite DB queries
- *(teams)* engine persists to DB, emits typed TeamEvents, adds verbose mode
- *(teams)* add migration v11 with team_runs/team_messages tables and TeamEvent enum
- *(tools)* add write_file tool with overwrite confirmation flow
- add periodic memory reflection system
- add agents-teams built-in skill
- *(skills)* add marketplace origin detection and lock cleanup
- *(skills)* implement marketplace install/uninstall/update CLI commands
- *(skills)* add git operations module for marketplace
- *(skills)* add marketplace lock file model and repo scanner
- add config editing and shellcheck guidance to shell-exec skill
- *(cli)* simplify mika ask --task-id to mark-and-exit
- *(tui)* shell-like slash command autocompletion with argument completion
- add agent and team management tools

### Changed

- rename default agent from "main" to "mika"
- resolve code review findings 511, 517, 520, 521
- consolidate to single database per container
- resolve code review findings from observability PR
- *(teams)* resolve code review findings from team TUI mode
- redesign schema with sessions + messages tables (v3)
- replace silent let _ = with warn logging on task DB ops
- resolve code review findings from callback TUI delivery
- unify self-knowledge tools into single get_documentation handler
- rename persona → self_model + agent-aware core memory defaults
- *(teams)* deduplicate team DB opening and test helpers
- *(teams)* address code review findings
- *(tools)* extract shared validate_and_resolve_path helper
- *(tui)* resolve code review findings for autocompletion

### Documentation

- update documentation for unified task engine completion
- update test count to ~837 after code review fixes
- update documentation for team TUI mode changes
- update documentation for skills marketplace changes
- add contributing guide with Claude Code workflow
- update documentation for shell-like autocompletion
- update documentation for graph-structured team persistence
- update documentation for agent management tools

### Fixed

- feature-gate turbofish on build_otel_layer test
- add type annotation to generic build_otel_layer call in test
- *(observability)* wire OTel layer into subscriber and address review findings
- use orchestrator agent_id for invoke_orchestrator parent task
- remove agent_id filtering from task tree traversal queries
- resolve team task agent_id mismatch causing orphaned pending tasks
- resolve 8 code review findings (543-552)
- use silent agent for CLI callback tasks and add orchestrator guards
- correct docs to say exec handlers receive input via stdin, not env var
- strengthen self-knowledge prompt to consult docs before answering
- read reflection timezone from identity.toml
- enforce plain first-name convention for people table
- sync agent display name from identity.toml on startup
- register_agent upsert + CLI memory reset agent-aware default
- auto-seed user in people table after onboarding + strengthen store_fact prompts
- cross-channel poll parameter mismatch in load_messages_after
- resolve 20 code review findings (498-519) — task engine hardening
- resolve cargo fmt and clippy warnings
- resolve 20 code review findings (478-497)
- resolve 14 code review findings from unified task engine review
- *(teams)* resolve code review findings 441-449
- *(teams)* emit per-agent completion and "all done" progress events
- *(teams)* add global timeout and progress heartbeats to team runs
- *(teams)* prevent wasted tool steps and ensure workspace writes
- *(cli)* move init_sqlite_vec into DB constructors and persist team conversations
- *(tools)* apply absolute-path reporting to read_workspace for consistency
- *(tools)* report resolved absolute path in write_file and write_workspace
- *(tools)* add target-file symlink check and prompt docs for write_file
- address all 13 code review findings for periodic memory reflection
- remove duplicate timeouts section from agents-teams skill
- resolve 15 code review findings for skills marketplace
- convert reminders fire_at from TEXT to INTEGER to fix detection
- add background reminder poller so reminders fire at scheduled time
- improve bundled skill seeding disabled warning message
- add jq availability guard to file-reader handler
- *(agent)* retry on ETXTBSY in exec handler to fix flaky tests
- don't load agent chat history in team mode TUI
- *(cli)* address review findings for team TUI mode
- *(tui)* place cursor at end after tab completion
- *(tui)* config value completion and tilde expansion security

## [0.1.3](https://github.com/senara-solutions/mika/releases/tag/v0.1.3) — 2026-03-02

### Added

- add config editing and shellcheck guidance to shell-exec skill
- add agent and team management tools

### Changed

- standardize jq JSON parsing and add env scrubbing to handlers

### Fixed

- use jq for JSON parsing in shell-exec handler

### Documentation

- update documentation for agent management tools

## [0.1.2](https://github.com/senara-solutions/mika/releases/tag/v0.1.2) — 2026-03-01

### Added

- add automated release system with GitHub binary downloads

### Documentation

- update documentation for release system and rustls migration

### Fixed

- *(cli)* eliminate temp file permission race in history write
- *(cli)* persist input history across sessions and fix paste cursor positioning

## [0.1.1](https://github.com/senara-solutions/mika/releases/tag/v0.1.1) — 2026-03-01

### Added

- add automated release system with GitHub binary downloads

### Documentation

- update documentation for release system and rustls migration