---
title: "docs: Add runtime structure reference document"
type: feat
status: completed
date: 2026-03-10
origin: docs/brainstorms/2026-03-10-runtime-structure-reference-brainstorm.md
---

# docs: Add runtime structure reference document

## Overview

Create a comprehensive runtime structure reference document that describes the `~/.mika` directory layout, SQLite schema v7, and log file locations. This gives Claude Code and the Mika agent deterministic knowledge of the runtime environment without rediscovering it each conversation.

## Problem Statement

Every new Claude Code conversation starts from scratch when verifying `~/.mika` state. Claude wastes time rediscovering directory layout, DB schema, log locations, and config files. The existing `docs/configuration.md` covers config file formats and env vars but not the database schema, directory tree details, or log paths. The self-knowledge skill doesn't have a topic for runtime structure.

## Proposed Solution

Create `docs/runtime-structure.md` as a single reference document, integrate it into the `get_documentation` builtin handler as a new topic, and add a pointer from CLAUDE.md.

### Key Design Decisions

**Truncation strategy (critical):** The `get_documentation` handler truncates output at 10,000 characters. To stay within this limit while being comprehensive:
- Focus the document on what's NOT already in other docs: directory tree, DB schema, log locations
- Cross-reference `docs/configuration.md` for config file formats (don't duplicate)
- Cross-reference `docs/skills.md` for `skill.toml` format
- Use concise table format for schema columns instead of full DDL
- Current v7 schema only (no migration history)
- Target: under 10K characters. If it exceeds, split into `runtime-structure` (directory + logs + schema overview) and `runtime-schema` (full table details) topics

**Content deduplication with `configuration.md`:**
- `runtime-structure.md` owns: directory tree, SQLite schema, log file locations, file permissions summary
- `configuration.md` owns: config file formats, config cascade, environment variables
- `runtime-structure.md` cross-references `configuration.md` for config details

(See brainstorm: `docs/brainstorms/2026-03-10-runtime-structure-reference-brainstorm.md`)

## Acceptance Criteria

- [x] `docs/runtime-structure.md` exists with directory tree, schema v7 (21 tables, 2 virtual tables, 1 view), and log locations
- [x] Document is under 10K characters (or split into multiple topics if needed)
- [x] `CLAUDE.md` has a pointer in the Directory Structure section
- [x] `get_documentation` builtin handler serves topic `"runtime-structure"`
- [x] Self-knowledge skill's `system_prompt.md` lists `runtime-structure` as an available topic
- [x] `build.rs` copies `runtime-structure.md` to OUT_DIR
- [x] `crates/mika-agent/docs/runtime-structure.md` exists as crates.io fallback
- [x] `scripts/sync-agent-docs.sh` includes `runtime-structure.md`
- [x] Error message in `builtin_handlers.rs` for invalid topics includes `runtime-structure`
- [x] Test `test_get_documentation_all_embedded_topics` includes `"runtime-structure"`
- [ ] `npm run build --prefix dashboard` still passes
- [x] `cargo test` passes

## MVP

### Phase 1: Write the reference document

#### `docs/runtime-structure.md`

Content sections:
1. **Directory Layout** — Annotated tree of `~/.mika/` (global level + one representative agent subtree). Annotate shared vs. per-agent resources. Note `$MIKA_HOME` override.
2. **SQLite Database** — Location (`~/.mika/data/mika.db`), PRAGMAs, schema version. Concise table-per-section with columns (name, type, constraints). Group by domain:
   - Core: `schema_version`, `agents`, `sessions`, `messages`
   - Memory: `core_memory`, `people`, `commitments`, `preferences`, `events`, `search_content` (+ FTS5/vec virtual tables)
   - Tasks: `tasks`
   - Teams: `teams`, `team_runs`, `team_workspace`
   - Audit: `audit_events`, `audit_event_summaries`
   - System: `heartbeat_sends`, `reflection_runs`, `customer_config`, `failed_sends`, `skill_overrides`
   - Views: `unified_timeline`
   - Notable indexes (unique partial indexes for dedup)
3. **Log File Locations** — CLI (daily-rotating in `{agent_home}/logs/`), server (stdout + optional `MIKA_SPIRIT_LOG_FILE`), gateway (optional `MIKA_GATEWAY_LOG_FILE`), team mode (`{team_dir}/logs/`)
4. **Cross-references** — Links to `configuration.md` (config formats, env vars, cascade), `skills.md` (skill.toml format)

### Phase 2: Integration (all files)

#### `crates/mika-agent/build.rs`

Add `"runtime-structure.md"` to the docs list that gets copied to `OUT_DIR/docs/`.

#### `crates/mika-agent/src/tools/builtin_handlers.rs`

1. Add `static DOC_RUNTIME_STRUCTURE: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/runtime-structure.md"));`
2. Add match arm `"runtime-structure" => DOC_RUNTIME_STRUCTURE`
3. Update error message string to include `runtime-structure` in valid topics list
4. Add `"runtime-structure"` to test topic list in `test_get_documentation_all_embedded_topics`

#### `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`

Add `runtime-structure` to the available topics list with description: `runtime-structure -- ~/.mika directory layout, SQLite schema v7, log file locations`

#### `CLAUDE.md`

Add one line in the Directory Structure bullet list:
```
- See [docs/runtime-structure.md](docs/runtime-structure.md) for full ~/.mika directory layout, DB schema, and log paths
```

#### `scripts/sync-agent-docs.sh`

Add `runtime-structure.md` to the `DOCS` array.

#### `crates/mika-agent/docs/runtime-structure.md`

Create as crate-local fallback copy (run `scripts/sync-agent-docs.sh` to generate).

## System-Wide Impact

- **Build:** `build.rs` change adds one more `include_str!` — negligible compile-time impact
- **Binary size:** One additional embedded doc (~8-10KB) — negligible
- **Self-knowledge skill:** Adding a topic is additive, no breaking changes
- **Maintenance:** Schema changes (v7→v8+) must update `runtime-structure.md` in the same PR. Add a note in CLAUDE.md conventions.

## Sources

- **Origin brainstorm:** [docs/brainstorms/2026-03-10-runtime-structure-reference-brainstorm.md](docs/brainstorms/2026-03-10-runtime-structure-reference-brainstorm.md) — key decisions: single doc + CLAUDE.md pointer, update self-knowledge skill, no slash commands needed
- Schema DDL: `crates/mika-agent/src/db.rs` (migrate_v1 through migrate_v7)
- Builtin handlers: `crates/mika-agent/src/tools/builtin_handlers.rs`
- Self-knowledge template: `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`
- Build script: `crates/mika-agent/build.rs`
- Doc sync script: `scripts/sync-agent-docs.sh`
- Existing config docs: `docs/configuration.md`
- Existing skills docs: `docs/skills.md`
- Solution: `docs/solutions/integration-issues/self-knowledge-missing-home-directory-files.md` — 2-layer reinforcement pattern
- Solution: `docs/solutions/database-issues/consolidate-per-agent-team-dbs-into-single-container-db.md` — single container DB invariant
