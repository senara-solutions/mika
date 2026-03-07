# Changelog

All notable changes to this project will be documented in this file.

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