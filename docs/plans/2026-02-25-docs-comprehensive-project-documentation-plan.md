---
title: "docs: Comprehensive Project Documentation"
type: docs
status: active
date: 2026-02-25
---

# Comprehensive Project Documentation

## Overview

Mika currently has only internal engineering docs (plans, solutions, todos) and an outdated README. This plan creates user-facing and operator-facing documentation by extracting and organizing existing codebase knowledge.

## Problem Statement

The README references only 4 tools (now 8), 2 crates (now 4), wrong binary name (`mika-cli` vs `mika`), and 32 tests (now 226+). There is no getting-started guide, architecture overview, skills authoring guide, configuration reference, slash-command reference, or deployment guide. Two audiences — end users and operators — have no documentation pathway.

## Documents to Create

### 1. README.md (overhaul)

Rewrite the existing README as the documentation hub.

**Content:**
- Project tagline: "AI executive assistant with persistent memory and per-customer isolation"
- Feature highlights (memory, skills, proactive heartbeat, multi-channel)
- Architecture diagram (ASCII): CLI mode + hosted mode (gateway → per-customer containers)
- Quick start (5 steps): install Rust, clone, build, set API key, run `mika`
- Tech stack table (Rust, SQLite, Axum, Claude API, ratatui, crossterm)
- Audience routing: "End users → getting-started.md" / "Operators → deployment.md"
- Links to all docs
- Current status (Phase 4)

**Source material:**
- `CLAUDE.md` (authoritative project overview)
- Current `README.md` (replace entirely)
- OpenClaw README at `/home/samidarko/workspace/senara-solutions/openclaw/README.md` (structure inspiration)

**Checklist:**
- [ ] Project description and vision
- [ ] ASCII architecture diagram (CLI + hosted modes)
- [ ] Quick start (clone, build, API key, run)
- [ ] Tech stack table
- [ ] Audience routing (end users vs operators)
- [ ] Links to all 7 docs
- [ ] Correct binary name (`mika`), test count (226+), tool count (8)
- [ ] Crate structure (4 crates)

### 2. docs/getting-started.md

First-run guide for CLI users.

**Content:**
- Prerequisites: Rust 1.85+ (from `rust-toolchain.toml`), Anthropic API key
- Installation: `cargo build --release` then copy binary, or `cargo install --path crates/mika-cli`
- API key setup: `export MIKA_ANTHROPIC_API_KEY=sk-ant-...` (explain this is required BEFORE first chat)
- First run: `mika` auto-detects uninitialized → runs setup → bootstraps `~/.mika/` → opens TUI
- First chat: onboarding conversation, core memory seeding
- Directory structure overview (`~/.mika/` layout)
- CLI subcommands table: `mika`, `mika chat`, `mika setup`, `mika memory`, `mika reminders`, `mika status`, `mika config`, `mika ask`
- Non-interactive mode: `mika ask "What's on my calendar?"`
- Next steps: links to configuration.md, skills.md, slash-commands.md

**Source material:**
- `crates/mika-cli/src/cli.rs` (Clap definitions, all subcommands)
- `crates/mika-cli/src/commands/` (subcommand implementations)
- `crates/mika-common/src/home.rs` (bootstrap, directory structure, file seeding)
- `crates/mika-cli/src/init.rs` (initialization flow)

**Checklist:**
- [ ] Prerequisites (Rust, API key)
- [ ] Build and install instructions
- [ ] API key setup step (explicit, before first chat)
- [ ] First run walkthrough
- [ ] `~/.mika/` directory layout
- [ ] CLI subcommands table (all commands from cli.rs)
- [ ] `mika ask` non-interactive mode
- [ ] Links to next docs

### 3. docs/architecture.md

System architecture for contributors and operators.

**Content:**

**Two operating modes:**
1. CLI embedded mode: `mika` binary with TUI, local SQLite, direct Claude API calls
2. Hosted mode: gateway + per-customer containers on K8s, Telegram integration

**Crate structure:**
| Crate | Type | Purpose |
|-------|------|---------|
| `mika-common` | lib | Config, Claude API client, logging, home directory |
| `mika-agent` | lib+bin | Agent loop, tools, skills, DB, HTTP server (`mika-server`) |
| `mika-cli` | bin | TUI CLI (`mika`), clap subcommands, slash commands |
| `mika-gateway` | bin | Telegram webhook router, customer pairing, outbound relay |

**Agent loop:**
1. Save user message → DB
2. Load context (soul.md, identity, core memory, timezone, conversation summary)
3. Match skills → inject prompts + resolve tool definitions
4. Load last 20 messages from DB
5. Send to Claude API
6. Match stop_reason: `EndTurn` → save + return, `ToolUse` → execute tools → loop
7. Max 10 tool steps, 30s per tool, 5min total timeout
8. Post-turn compaction (if >50 messages)

**Memory model (3 layers):**
- Layer 1: Core memory (4 blocks, 2000 token cap, always in system prompt, agent-editable)
- Layer 2: Structured facts (People, Commitments, Preferences, Events — SQL-backed)
- Layer 3: Vector search (future: sqlite-vec + FTS5)

**8 builtin tools:** update_core_memory, store_fact, search_memory, update_fact, create_reminder, list_reminders, cancel_reminder, send_message

**AsyncDatabase pattern:** Closure-based dispatch to dedicated OS thread, Send+Sync wrapper, zero mutex contention

**Conversation compaction:** 50-message threshold, keep 20 recent, summarize older via Claude

**Heartbeat system:** CronJob → pre-filter (active hours 8-21, rate limits) → silent agent loop → send_message tool

**Silent mode:** Background tasks (heartbeat, reminders) where text output is NOT delivered; agent must use send_message tool explicitly

**Source material:**
- `crates/mika-agent/src/agent.rs` (agent loop, constants, run_agent, run_silent_agent)
- `crates/mika-agent/src/prompt.rs` (system prompt assembly)
- `crates/mika-agent/src/db.rs` (schema, core memory blocks)
- `crates/mika-agent/src/async_db.rs` (AsyncDatabase pattern)
- `crates/mika-agent/src/compaction.rs` (compaction logic)
- `crates/mika-agent/src/tools/` (all 8 tools)
- `crates/mika-agent/src/server/` (HTTP server, handlers, auth)
- `crates/mika-agent/src/scheduler.rs` (reminder recovery, heartbeat)
- `crates/mika-agent/src/messaging.rs` (MessageSender, failed sends)
- `crates/mika-gateway/src/` (routes, telegram, settings)
- `docs/solutions/architecture/async-database-wrapper-pattern.md`
- `docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md`

**Checklist:**
- [ ] Two operating modes (CLI vs hosted)
- [ ] Crate structure table
- [ ] Agent loop flowchart (numbered steps)
- [ ] Memory model diagram (3 layers)
- [ ] All 8 tools listed with brief descriptions
- [ ] AsyncDatabase pattern explained
- [ ] Conversation compaction explained
- [ ] Heartbeat pre-filter + silent mode explained
- [ ] Gateway architecture (inbound/outbound flows)
- [ ] Customer pairing flow

### 4. docs/skills.md

Skills system guide for users creating custom skills.

**Content:**
- What skills are: filesystem-based tool bundles that inject prompts and tool definitions per-message
- Directory structure: `~/.mika/skills/<name>/skill.toml` + optional `system_prompt.md` + optional `tools.json`
- Manifest format (skill.toml): all fields with types and defaults
- Handler types:
  - **Builtin**: references tools already in the Rust ToolRegistry
  - **Exec**: runs shell command, passes tool name as arg, input via `MIKA_TOOL_INPUT` env var
  - **Http**: POSTs to URL with `{"tool_name": "...", "input": {...}}`
- Trigger matching: `always_on` skills always active; others matched by case-insensitive keyword substring
- `system_prompt.md`: injected as `## <SkillName> Skill\n<contents>` into system prompt
- `tools.json` format: array of ToolDefinition objects (for exec/http handlers)
- Built-in skills reference:
  - `memory` (always_on, 7 keywords, 4 tools)
  - `reminders` (always_on, 5 keywords, 3 tools)
  - `messaging` (always_on, 3 keywords, 1 tool)
- Tutorial: create a custom exec skill (e.g., weather lookup with a shell script)
- Customization: modifying builtin skill manifests (safe — bootstrap preserves user changes)
- Security: exec handler runs unsandboxed commands; document trust boundary

**Source material:**
- `crates/mika-agent/src/skills/manifest.rs` (SkillManifest, Handler, Triggers, SkillOptions)
- `crates/mika-agent/src/skills/index.rs` (scanning)
- `crates/mika-agent/src/skills/matcher.rs` (matching logic)
- `crates/mika-agent/src/skills/loader.rs` (lazy loading)
- `crates/mika-agent/src/skills/handler.rs` (exec/http dispatch)
- `crates/mika-agent/src/skills/mod.rs` (SkillRegistry, resolve_matched_skills)
- `templates/skills/` (builtin skill templates)
- `docs/solutions/architecture-decisions/filesystem-skill-registry-implementation.md`

**Checklist:**
- [ ] What skills are and why they exist
- [ ] Directory structure diagram
- [ ] Complete skill.toml reference (all fields, defaults, types)
- [ ] Handler types with examples for each
- [ ] Trigger matching explained (substring, case-insensitive, always_on)
- [ ] system_prompt.md injection explained
- [ ] tools.json format documented
- [ ] 3 built-in skills reference table
- [ ] Tutorial: custom exec skill step-by-step
- [ ] Security considerations for exec/http handlers
- [ ] Customization: modifying builtin skills

### 5. docs/configuration.md

Complete configuration reference.

**Content:**

**`~/.mika/` directory layout:**
```
~/.mika/
  config.toml      # User config overrides
  identity.toml    # Agent name + emoji
  soul.md          # Personality definition
  heartbeat.md     # Heartbeat checklist
  user.md          # User self-description
  data/
    mika.db        # SQLite database
  logs/
  skills/          # Skill registry
  exports/         # Conversation exports
```

**Configuration cascade (highest priority wins):**
1. `MIKA_*` environment variables
2. `~/.mika/config.toml`
3. `config/local.toml` (gitignored, for dev)
4. `config/default.toml` (bundled defaults)

**Settings reference table:** All fields from `config.rs` Settings struct with type, default, env var

**identity.toml reference:**
```toml
name = "Mika"
emoji = "✦"
```

**soul.md guide:** What it does (injected into system prompt), how to customize personality, boundaries, communication style. Show default content.

**heartbeat.md guide:** Checklist the agent evaluates during proactive check-ins. Show default.

**user.md:** Free-form self-description. Note: loaded by prompt builder for context.

**Environment variables — CLI mode vs server mode:**
| Variable | CLI | Server | Gateway |
|----------|-----|--------|---------|
| `MIKA_ANTHROPIC_API_KEY` | Required | Required | — |
| `MIKA_CLAUDE_MODEL` | Optional | Optional | — |
| `MIKA_DB_PATH` | Optional | Optional | — |
| `MIKA_ROUTING_URL` | — | Required | — |
| `MIKA_INTERNAL_TOKEN` | — | Required | Required |
| `MIKA_CUSTOMER_ID` | — | Required | — |
| `MIKA_DATABASE_URL` | — | — | Required |
| `MIKA_TELEGRAM_BOT_TOKEN` | — | — | Required |
| etc. |

**Model configuration:** Supported models, how to switch (`MIKA_CLAUDE_MODEL` or config.toml)

**Source material:**
- `crates/mika-common/src/config.rs` (Settings struct, all fields)
- `crates/mika-common/src/home.rs` (directory structure, default file contents)
- `crates/mika-agent/src/prompt.rs` (how soul.md, identity, heartbeat are loaded)
- `crates/mika-gateway/src/settings.rs` (GatewaySettings)
- `.env.example`

**Checklist:**
- [ ] Directory layout diagram
- [ ] Configuration cascade with priority order
- [ ] Settings reference table (all fields)
- [ ] identity.toml reference
- [ ] soul.md guide with defaults
- [ ] heartbeat.md guide with defaults
- [ ] user.md explained
- [ ] Environment variables table (CLI vs server vs gateway)
- [ ] Model configuration
- [ ] $MIKA_HOME override

### 6. docs/slash-commands.md

Complete slash command reference for TUI users.

**Content:**
- What slash commands are: client-side commands in the TUI, never sent to the agent
- Autocomplete: type `/` to see popup, Tab/Up/Down to navigate, Enter to execute, Esc to dismiss
- Command reference table with usage examples for each of the 13 commands:
  - `/help` (`/h`, `/?`) — list all commands
  - `/clear` — clear chat display
  - `/exit` (`/quit`, `/q`) — quit
  - `/compact` — trigger conversation compaction
  - `/memory` (`/mem`) — show core memory blocks
  - `/memory search <query>` — search all memory layers
  - `/reminders` (`/remind`) — list active reminders
  - `/status` (`/stat`) — show health info
  - `/soul` — display soul.md
  - `/config` (`/cfg`) — show config
  - `/model` — show active model
  - `/export` — export conversation to markdown
  - `/skills` — list loaded skills
  - `/skill <name>` — show skill details
- Keyboard shortcuts in TUI: Ctrl+C quit, Esc clear input, PageUp/PageDown scroll, Up/Down history

**Source material:**
- `crates/mika-cli/src/tui/commands/mod.rs` (COMMANDS array, SlashCommand struct)
- `crates/mika-cli/src/tui/commands/handlers.rs` (handler implementations)
- `crates/mika-cli/src/tui/commands/autocomplete.rs` (autocomplete behavior)
- `crates/mika-cli/src/tui/input.rs` (key bindings)

**Checklist:**
- [ ] What slash commands are (client-side only)
- [ ] Autocomplete usage guide
- [ ] All 13 commands with aliases, arguments, example output
- [ ] TUI keyboard shortcuts

### 7. docs/deployment.md

Hosted mode deployment guide for operators.

**Content:**

**Architecture overview:** Gateway (shared, stateless, Postgres-backed) + per-customer containers (SQLite, PVC, one-per-customer)

**Prerequisites:**
- Kubernetes cluster (EKS/GKE/etc.)
- Postgres database (for gateway customer registry)
- Anthropic API key
- Telegram Bot (via BotFather)
- Container registry (ECR or similar)
- Two namespaces: `mika-system` (gateway), `mika-agents` (customer containers)

**Step 1: Build Docker images**
```bash
docker build -f Dockerfile.agent -t mika-agent:dev .   # ~95MB
docker build -f Dockerfile.gateway -t mika-gateway:dev . # ~90MB
```
Push to registry.

**Step 2: Create Telegram Bot**
- BotFather → /newbot → get token
- Generate webhook secret: `openssl rand -hex 32`

**Step 3: Deploy gateway**
- `scripts/setup-gateway.sh` (creates K8s secret)
- `helm install mika-gateway helm/mika-gateway/`
- Gateway auto-creates Postgres `customers` table on startup

**Step 4: Provision a customer**
- `scripts/provision.sh <customer-name>` (creates K8s secret, helm install, registers in Postgres, generates Telegram deep link)
- Send deep link to customer → they click → pairing completes → onboarding begins

**Step 5: Set up heartbeat CronJob**
- `scripts/heartbeat-all.sh` as K8s CronJob (e.g., every 15 minutes)

**Deprovisioning:** `scripts/deprovision.sh <customer-id>` (suspends → helm uninstall → delete PVC → delete secret → remove from Postgres)

**Helm values reference:** Key values for both charts

**Security:**
- All tokens: 64-char hex, generated via `openssl rand -hex 32`
- Constant-time token comparison (subtle crate)
- Non-root containers, read-only root filesystem
- Deterministic container URLs (no SSRF risk)
- Encrypted volumes (depends on cloud provider EBS/disk encryption)

**Troubleshooting:** Common issues (agent busy 429, pairing token expired, bot blocked)

**Source material:**
- `Dockerfile.agent`, `Dockerfile.gateway`
- `helm/mika-customer/`, `helm/mika-gateway/`
- `scripts/provision.sh`, `scripts/deprovision.sh`, `scripts/setup-gateway.sh`, `scripts/heartbeat-all.sh`
- `crates/mika-gateway/src/` (routes, telegram, settings)
- `crates/mika-agent/src/server/` (handlers, auth)
- `docs/solutions/integration-issues/telegram-webhook-gateway-design.md`

**Checklist:**
- [ ] Architecture diagram (gateway + containers)
- [ ] Prerequisites list (K8s, Postgres, Telegram, registry)
- [ ] Docker image build commands
- [ ] Telegram Bot setup (BotFather walkthrough)
- [ ] Gateway deployment steps
- [ ] Customer provisioning walkthrough
- [ ] Heartbeat CronJob setup
- [ ] Deprovisioning steps
- [ ] Helm values reference for both charts
- [ ] Security section (tokens, isolation, encryption)
- [ ] Troubleshooting (common issues)
- [ ] Token generation commands (`openssl rand -hex 32`)

---

## Implementation Order

Each document is independent — they can be written in parallel with sub-agents.

1. **Phase 1 — README.md** (hub page, sets the tone, links to everything else)
2. **Phase 2 — All other docs in parallel:**
   - docs/getting-started.md
   - docs/architecture.md
   - docs/skills.md
   - docs/configuration.md
   - docs/slash-commands.md
   - docs/deployment.md

## Verification

- [ ] All code references verified against actual source files
- [ ] All environment variables checked against Settings structs
- [ ] All slash commands verified against COMMANDS array
- [ ] All tools verified against tools/ directory
- [ ] All Helm values verified against values.yaml files
- [ ] All provisioning steps verified against scripts
- [ ] Links between docs are bidirectional
- [ ] No invented features — everything extracted from code
- [ ] Binary name is `mika` (not `mika-cli`) throughout
- [ ] Test count matches actual `cargo test` output
