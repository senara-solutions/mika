---
title: mika-cli TUI thin-client — Phase 1 audit and refactor plan
issue: mika#1727
status: audit-in-progress
authored: 2026-07-06
authors: orchestrator-CC (MPC) via samidarko relay of Prime ratification
---

# TUI thin-client — Phase 1 audit and refactor plan

Companion audit for `senara-solutions/mika#1727`. This document exists to satisfy AC1 (duplication inventory), AC2 (spirit-side gap document), AC3 (wrapper-doctrine test), AC4 (standalone-mode disposition), and to sketch AC5 (structural enforcement) for the Phase 2 refactor.

**Scope discipline**: this is a *plan document*, not the refactor itself. Actual code changes belong to sub-tickets fanned out from AC2's gap list, per Prime's discipline that "missing spirit endpoints from the audit become individual follow-up sub-issues, NOT bundled into this refactor."

## Load-bearing structural findings

Established in recon before any per-module inventory. These shape the refactor's viability and are worth stating before the module-by-module comparison.

### F1 — mika-spirit is a binary of mika-agent, not a separate crate

Located at `crates/mika-agent/src/bin/mika-spirit.rs`. The binary is thin — bootstrap + logging + panic hook + `mika_agent::server::run_server(&settings).await`. All server-side logic (routes, handlers, tool execution, agent loop) lives in the `mika-agent` library. This is important because "moving TUI onto spirit" doesn't require creating a new crate — it means making TUI consume `mika-agent`'s existing HTTP surface (already exposed by the spirit binary) instead of calling `mika-agent`'s library API directly in-process.

### F2 — mika-cli already depends on mika-agent as a workspace dep

From `crates/mika-cli/Cargo.toml`:
```
mika-common.workspace = true
mika-agent.workspace = true
mika-a2a.workspace = true
```

TUI consumes mika-agent's library — that's the standalone-mode surface. The refactor makes this dependency ~empty (retaining only types the wire protocol shares), replacing runtime consumption with HTTP calls through `mika-a2a::client` (or a new HTTP client wrapper).

### F3 — Thin-client precedent exists at `mika-cli/src/remote_ask.rs`

For `mika ask --remote <URL>` (or `MIKA_REMOTE_AGENT_URL`), the CLI dispatches the prompt via A2A protocol using `mika_a2a::client::A2aClient`, receiving a `Task` back and rendering it. Comment header cites the ascension architecture: "local↔cloud Mika portability, R1 daily-use unblock slice."

This is the pattern the TUI refactor generalizes. `remote_ask.rs` covers `ask` (one-shot); `chat.rs` needs SSE streaming + interactive turns, which is a superset of what a2a's `message/stream` already exposes.

**Implication for AC4**: the "standalone-mode disposition" question is not purely doctrinal. Standalone-mode is already partially routed around when `--remote` is set. Deleting it means always taking the remote path, with `mika-spirit` either external (deploy target) or a local process (development / single-user). The doctrine says (b) — delete — is the right choice; F3 shows it's already partially the case.

### F4 — mika-spirit's HTTP surface is already rich

From `crates/mika-agent/src/server/mod.rs` route table, non-exhaustive enumeration:

| Route | Method | Handler | Scope |
|---|---|---|---|
| `/dashboard/timeline` | GET | `dashboard::handle_timeline` | dashboard |
| `/dashboard/agents` | GET | `dashboard::handle_agents_list` | dashboard |
| `/dashboard/agents/{id}` | GET | `dashboard::handle_agent_detail` | dashboard |
| `/dashboard/agents/{id}/audit` | GET | `dashboard::handle_agent_audit` | dashboard |
| `/dashboard/agents/{id}/facts` | GET | `dashboard::handle_agent_facts` | dashboard |
| `/dashboard/sessions` | GET | `dashboard::handle_sessions_list` | dashboard |
| `/dashboard/sessions/{id}` | GET | `dashboard::handle_session_detail` | dashboard |
| `/dashboard/tasks` | GET | `dashboard::handle_tasks_list` | dashboard |
| `/dashboard/tasks/{task_id}` | GET | `dashboard::handle_task_detail` | dashboard |
| `/dashboard/team-runs` | GET | `dashboard::handle_team_runs_list` | dashboard |
| `/dashboard/llm-calls` | GET | `dashboard::handle_llm_calls` | dashboard |
| `/dashboard/tool-calls` | GET | `dashboard::handle_tool_calls` | dashboard |
| `/a2a/*` | POST (JSON-RPC) | `a2a::handle_a2a_jsonrpc` | A2A: send/stream/get/cancel/resubscribe/push_config/agent_card |
| `/agents/*` (agent-scoped) | GET/POST/... | `handlers::*` | agent management surface (audit downstream) |
| `/investigate/*` | POST | `investigate::*` | investigation flows |
| `/checkpoint/*` | GET/POST | `checkpoint::*` | checkpoint infra |
| `/ci/*` | POST | `ci_failure_handler::*`, `ci_success_handler::*` | CI event ingestion |

**Note on missing `/healthz`**: samidarko's un-wedge diagnosis flagged that spirit lacks a `/healthz` endpoint. Not verified in this pass — call out as a small gap for follow-up. `/dashboard/timeline` was previously used as a de facto liveness probe.

## AC1 — Duplication inventory (in-process consumption from mika-cli)

Enumerated from `grep -rh '^use mika_agent::' crates/mika-cli/src/` + call-site inspection of `commands/chat.rs`, `commands/ask.rs`. This is the **standalone-path surface**; the refactor moves each of these to an HTTP boundary or eliminates them.

| mika_agent module | mika-cli consumers | What it does | Refactor disposition |
|---|---|---|---|
| `agent::{run_agent, AgentParams, check_onboarding}` | chat.rs:26, ask.rs | The agent loop entry — turn-taking, LLM dispatch, tool orchestration. Hot path. | **Replace with A2A `message/send` + `message/stream`** to spirit. Existing `remote_ask.rs` proves the shape for `ask`; TUI needs SSE consumption. |
| `agent_loop` (inner, via `agent`) | via `run_agent` | The `run_loop` core — retrieve-context → build-prompt → LLM → tool exec. | Removed from TUI's dep graph as a consequence of replacing `run_agent`. |
| `skills::{SkillRegistry, executor, git, index, install, manifest, marketplace, variants}` | chat.rs:29, commands/skills.rs, commands/skills_variants.rs | Skill loading + install + marketplace + variant management. | **Split**: skill *management* (install/update/list/marketplace) is legitimate CLI territory — filesystem operations Vincent runs at his shell. Skill *loading for turn execution* moves behind spirit (spirit has skills mounted at `~/.mika/agents/<name>/skills/`). |
| `task_engine::{TaskEngine, TaskDispatcher}` | chat.rs:29, ask.rs, commands/tasks.rs | Task queue: heartbeat scheduling, phantom-task detection, tick loop, prune. | **Move behind spirit**. TUI status pane consumes `GET /dashboard/tasks` (exists) + SSE task-event stream (**gap** — new spirit endpoint needed). CLI `mika tasks *` commands become HTTP wrappers. |
| `tools::{default_tools, management_tools_if_needed, create_skill::validate_skill_name}` | chat.rs:96-97 | Tool registry construction. | Removed from TUI. Spirit constructs its own tool registry per-session. |
| `messaging::{GatewayMessageSender, MessageSender}` | chat.rs, ask.rs | Message sending (cross-agent gateway path). | **Move to spirit** — TUI-side sending routes through spirit's a2a. |
| `db::{Database, AsyncDatabase, ...}` | Multiple commands, TUI panels | Persistence surface — sessions, messages, tasks, memory. | **Move behind spirit HTTP APIs.** `Database` direct-read is a doctrine violation for TUI. The `/dashboard/*` REST routes already cover most reads; **gap**: no unified session-message read that streams. |
| `mcp::McpManager` | chat.rs, commands/mcp.rs | MCP (Model Context Protocol) server management. | **TBD**: does an MCP server-config belong to the operator (edit-time, CLI-side) or the runtime (spirit-side)? Investigation follow-up. |
| `startup` | chat.rs | Pre-turn setup (async_db init, skill registry, task engine spawn, etc.). | Eliminated — spirit does its own startup; TUI just connects. |
| `bundled_skills` | commands/skills.rs | Enumerate skills bundled into the binary. | CLI-side — legitimate (build-time metadata). |
| `config_keys` | commands/config.rs | Settable config-key allowlist for `mika config set`. | CLI-side — config file management is operator territory. |
| `teams::types::{TeamEvent, TeamPhase, RunStatus}` | chat.rs, commands/teams.rs | Team run event types. | **Types stay** (wire schema); logic moves behind spirit. |
| `prompt` | ask.rs | Prompt-assembly primitives. | **Moves to spirit** — TUI provides raw user text, spirit assembles. |

**Comprehensive inventory count: 11 mika_agent-module use-sites in `commands/chat.rs`, 9 in `commands/ask.rs`, plus commands-panel-scoped uses across `commands/*.rs`.** Full pin-down of every call-site with file:line is Phase 1 follow-up work — the tables above are the first-pass structural map.

## AC2 — Spirit-side gap document

Cross-referencing TUI needs (from `chat.rs`, `tui/app.rs::App`) against spirit's current HTTP surface (§F4). Each named endpoint below either **exists** (verified via server/mod.rs), **is present but needs augmentation**, or **is missing** (Phase 2 sub-ticket candidate).

### Present and sufficient (no work)

- **Session read**: `GET /dashboard/sessions/{id}` — TUI's session-picker + session-detail panels can consume this.
- **Task list**: `GET /dashboard/tasks` — TUI status pane can consume this.
- **Agent list**: `GET /dashboard/agents` — TUI agent-picker consumes this.
- **A2A one-shot**: `POST /a2a/*` with `message/send` — covers `mika ask` remote-mode (already used by `remote_ask.rs`).
- **A2A streaming**: `POST /a2a/*` with `message/stream` — the primary hot-path stream the TUI's chat surface will consume.
- **A2A task retrieval**: `tasks/get`, `tasks/cancel`, `tasks/resubscribe` — session-resume + cancel semantics.

### Present but likely insufficient — verify in Phase 2

- **`a2a` `message/stream`** — need to verify it carries fine-grained tool-call events (`tool_call_start`, `tool_call_result`) in addition to assistant-turn text. TUI's "running Bash: X" rendering depends on this. If it only carries text turns, augment with intermediate SSE events. **Verification task: Phase 2 sub-ticket A.**
- **Task-event live stream** — dashboard `/dashboard/tasks` is a snapshot, not a subscription. TUI status pane wants live task-transition events (created / completed / failed). **New SSE endpoint required. Phase 2 sub-ticket B.**
- **LLM-call cost trend** — `/dashboard/llm-calls/cost-trend` exists; verify TUI's cost pane shape matches.

### Missing — Phase 2 sub-tickets required

- **Permission-decision request stream** — when spirit's classifier defers a tool call to the operator (Y/N approval), TUI needs to receive the request, prompt the operator, and reply. Mirrors claude-pilot's `canUseTool` protocol. Wire shape TBD; likely SSE event with correlated POST-back for the decision. **Phase 2 sub-ticket C — this is the biggest wire-protocol surface change.**
- **AskUserQuestion callback bridge** — same wire shape as C above but for structured questions. Could be the same channel with a discriminated event type. **Phase 2 sub-ticket D (may combine with C).**
- **Session-messages ordered stream** — for TUI's message pane, a session-scoped SSE of new assistant/user messages. `/dashboard/sessions/{id}` is snapshot-only. Augmentation of `message/stream` may cover this; verify in Phase 2 A.
- **`/healthz` liveness endpoint** — small but structural; samidarko's un-wedge diagnosis surfaced its absence. **Phase 2 sub-ticket E (small).**

**Missing endpoints tally: 3 substantive (permission-decision, task-event stream, session-messages stream) + 1 small (`/healthz`) + verifications (stream contents).** Sub-tickets fan out per Prime's discipline; the actual TUI refactor lands after those clear.

## AC3 — Wrapper-doctrine subsection (for the refactor plan)

Copied verbatim into the refactor's plan document once Phase 2 sub-tickets land:

> **TUI must not contain business logic mika-spirit already owns.** Every state read + every side effect goes through spirit's HTTP API. mika-spirit is the primitive; mika-cli's TUI is the wrapper. Any proposed TUI feature that requires new in-process logic (a state machine, a database read, a policy evaluation, a network call to a third-party service) triggers wrapper-doctrine — refuse without debate, or file a spirit-side sub-ticket to expose the capability behind an HTTP boundary first.
>
> This applies identically to `crates/mika-cli/src/commands/*.rs` **runtime paths** (chat, ask, tasks status view, etc.), not to CLI-side **management paths** (skill install, config set, doctor, agents create — file/config operations that legitimately belong at the operator's shell).
>
> Reference: D4 (`mika-platform/docs/decisions/2026-07-04-claude-spirit-shape.md`) applied one layer down. mika-spirit is the primitive being wrapped by TUI, exactly as `claude-agent-sdk` (via `claude-pilot-py`) is the primitive being wrapped by `claude-spirit`.

## AC4 — Standalone-mode disposition

**Recommendation: (b) delete standalone mode entirely.** Rationale:

1. **Doctrine**: standalone mode IS the anti-pattern the ticket is designed to eliminate. Retaining a "fallback" perpetuates the two-truths problem D3 and D4 rejected on the alias-and-persistence fronts.
2. **Structural precedent**: F3 above — `remote_ask.rs` already partially routes around standalone mode. Deleting completes what's begun.
3. **Deploy shape**: mika-spirit is packaged as an OpenRC daemon; single-user boxes run it as a local process on `127.0.0.1:8081`. "TUI without spirit" is a degraded case, not a supported one.
4. **AC5 requires it**: structural enforcement (§AC5) is only possible if standalone is gone. Keeping standalone means preserving the `mika_agent::agent::run_agent` call site in TUI, which is exactly the type-shape AC5 forbids.

Migration path for existing users:
- `mika chat` and `mika ask` default to `mika-spirit` at `http://127.0.0.1:8081` if `mika-spirit` is running.
- Fresh-install bootstrap starts `mika-spirit` as a user-service before first TUI launch.
- Explicit CLI flag `--spirit-url <url>` for non-loopback.
- `MIKA_REMOTE_AGENT_URL` remains the environment override.

## AC5 — Structural enforcement sketch

The load-bearing goal: **the type system in `crates/mika-cli` must not be able to construct `AgentLoop`, `ToolExecutor`, or `SessionManager` locally.** Not "we tell developers not to"; the code that would violate this must not compile.

### Shape sketch (not final — Phase 2 sub-ticket may amend)

Option A: **Public-in-crate-only types**. Move `agent_loop`, `tool_execution`, session-manager types from `mika-agent`'s public API into `pub(crate)` visibility. `mika-cli` cannot see them; only `mika-spirit`'s binary code (which is in the same crate) can construct them. TUI gets exactly what it needs via `mika-a2a` client + `mika-common` shared types.

Option B: **Extract to a separate `mika-agent-core` crate**. `mika-agent-core` is the private substrate; `mika-agent` re-exports only wire types; `mika-cli` depends only on `mika-agent` (public wire types), `mika-a2a` (client), and `mika-common` (utilities). Compile-error if TUI tries to `use mika_agent_core::*`.

**Recommendation: A first (lower blast radius; single-crate boundary flip), migrate to B if needed for cleaner semver.** Phase 2 sub-ticket owns the decision after gap-fill lands.

### Verification test (Phase 2 acceptance property)

After the refactor:
```
$ rg 'mika_agent::agent_loop|mika_agent::tool_execution|mika_agent::agent::run_agent' crates/mika-cli/src/
(empty)
```
And:
```
$ cargo check -p mika-cli
   Compiling mika-cli
$ echo "use mika_agent::agent_loop;" >> crates/mika-cli/src/main.rs
$ cargo check -p mika-cli
error[E0603]: module `agent_loop` is private
```
That compile error IS the AC5 property landing. Same lesson as `feedback_prompt_enforcement_fragile` — structural constraint beats any doctrinal note.

## Phase 2 sub-issue candidates (fan-out list)

Per Prime's discipline (missing spirit endpoints become individual sub-tickets, not bundled into this refactor). Draft one issue per line; each gets `mika#1727` as parent per `Refs: mika#1727`:

- **A** — Verify + augment `a2a` `message/stream` SSE frames to carry `tool_call_start` / `tool_call_result` events for TUI fine-grained rendering.
- **B** — New SSE endpoint: task-event live stream (`GET /dashboard/tasks/stream` or `/dashboard/events?class=tasks`) for TUI status pane subscription.
- **C** — Permission-decision request stream — wire protocol for spirit-defers-to-operator tool-call approval. Correlated request/response over SSE + POST. **Largest scope in the fan-out.**
- **D** — AskUserQuestion callback bridge — likely combined with C; discriminated event type on same channel.
- **E** — Add `/healthz` liveness endpoint (small; probably 20-line PR).
- **F** — Session-messages ordered stream — either verified as covered by A's augmentation of `message/stream`, or new endpoint.
- **G** — MCP server management surface — decide whether MCP config is CLI-side (edit-time) or spirit-side (runtime); land the boundary. Depends on the doctrine call, not just implementation.

Each sub-ticket is smaller than mika#1727 itself and can dispatch autonomously once mika#1727 (daemon-mode retrofit) lands.

## What lands in the refactor PR that closes mika#1727

Once sub-tickets A-G (or the subset that Phase 2 confirms needed) land, the closing PR:

1. Deletes the standalone-mode consumption of `mika_agent::agent::run_agent` from `chat.rs`.
2. Rewires `commands/chat.rs` to use `mika_a2a::client::A2aClient` for turn-streaming, augmented for tool-call events per sub-ticket A.
3. Wires permission-decision requests into TUI's existing approval-prompt UI (from claude-pilot familiarity).
4. Reduces `crates/mika-cli/Cargo.toml`'s `mika-agent` dependency to wire-type-only re-exports (or drops it if a `mika-agent-core` split lands per §AC5 Option B).
5. Applies Option A or B from §AC5 to make standalone-mode compilation impossible.
6. Deletes commands/paths made unreachable (`AgentParams` construction, `startup::*` in TUI, task_engine local spawn).
7. Documents fresh-install behavior: `mika-spirit` starts as user-service before first `mika chat` invocation.

Estimated refactor PR size (post sub-ticket clearance): ~500-1500 lines deleted, ~200-400 added. Net reduction is the point.

## What this document does NOT do

- **Not a code change beyond this document.** No source files in `crates/mika-cli/src/` or `crates/mika-agent/src/` are touched by this PR.
- **Not a per-file line-by-line inventory.** The tables in §AC1 are the first-pass structural map; per-file pin-down is a Phase 1 follow-up commit on the same branch.
- **Not sub-ticket bodies.** The A-G fan-out list above is the seed; individual sub-tickets get filed with their own ACs before Phase 2 implementation.
- **Not the interim-drive plan.** Vincent drives via `mika ask --agent mika-dev "..."` manually until Phase 2 sub-tickets land — that's `feedback_no_direct_impl_use_mika_spawn` extended for the current window and is documented separately (samidarko's dispatch, 2026-07-06).

## Cross-links

- `senara-solutions/mika#1727` — the parent ticket this doc satisfies Phase 1 for.
- `senara-solutions/control-monitor#101` — cm-side deferred B1 (cm as observability home for permission events); shares fire-and-forget discipline with sub-ticket C above.
- `senara-solutions/control-monitor#66` — claude-spirit implementation; same D4 wrapper-doctrine shape one layer up.
- `senara-solutions/control-monitor#99` — async-emit path (cpp permission events → cm event_log); the wire-protocol shape sub-ticket C above needs to reference for spirit's own async-emit path.
- `mika-platform/docs/decisions/2026-07-04-claude-spirit-shape.md` (D4) — wrapper-doctrine originating decision record.

## Sub-tickets filed (2026-07-06 evening)

Phase 2 fan-out list from §AC2 landed as individual issues. Each is a sub-issue of `senara-solutions/mika#1727`; cross-links populated in each body.

| Sub | Issue | Title / Shape |
|---|---|---|
| A | `senara-solutions/mika#1731` | Verify + augment a2a `message/stream` SSE to carry tool-call events |
| B | `senara-solutions/mika#1732` | New SSE endpoint: task-event live stream for TUI status pane |
| C | `senara-solutions/mika#1733` | **Permission-decision request stream — structural discipline.** Carries the 6 sharpenings from today's §12 pass 1: structural-only guarantee (not LLM classifier), `decision_authority` server-side only, provenance day-one (`classifier_verdict` + `operator_decision` + `override_used`), pre-registered flip conditions, scoped config, `override_event` D6 class + fire-and-forget async emit. Shipped default: STRICT + override OFF regardless of Vincent's founder-question answer. |
| D | `senara-solutions/mika#1734` | `AskUserQuestion` callback bridge — shares C's wire channel via discriminated event type. |
| E | `senara-solutions/mika#1735` | Add `/healthz` liveness endpoint (small; ~20-line PR). |
| F | `senara-solutions/mika#1736` | Session-messages ordered stream — verify sub-A coverage or file follow-up. |
| G | `senara-solutions/mika#1737` | **MCP boundary — Option (a) CLI-side ratified by mika-arch** (session_id `0e01b314-4085-439e-95a4-ecc9cc103d6d`). Ratifying rationale (C1 wrapper-doctrine precedent + C2 YAGNI + C3 session lifecycle alignment + C4 structural simplicity) carried verbatim in sub-G body. No Prime routing required. |

Sub-C is the largest wire-protocol scope; A/B/D/F/E are smaller each. G is the smallest doctrinally-load-bearing follow-up (arch-ratified, no Prime).

**Sub-C shape check**: sharpenings 1-6 came from a two-seat §12 pass on 2026-07-06 (chat seat + gentux seat); chat seat's LLM-fragility catch qualifies as outsideness class per anchor §12, resetting the pass-1 window to 3 fresh passes from that point (samidarko relay).

## Next action

Attack mika#1727 Phase 2 refactor after sub-issues clear. Per Prime's discipline: the closing PR for mika#1727 lands only after sub-tickets have shipped their surfaces (or been closed as no-op verifications). Per-file line-by-line inventory (Phase 1 follow-up work) is an amendment commit on this same branch, not a separate PR.
