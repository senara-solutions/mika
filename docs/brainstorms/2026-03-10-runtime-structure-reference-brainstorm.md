# Brainstorm: Runtime Structure Reference Doc

**Date:** 2026-03-10
**Status:** Ready for planning

## What We're Building

A single reference document (`docs/runtime-structure.md`) that comprehensively describes the `~/.mika` runtime structure: directory layout, SQLite database schema (all tables, columns, indexes, views), log file locations, config file formats, and config resolution order.

This document serves two consumers from one source of truth:
1. **Claude Code** — reads it when investigating runtime issues during development, referenced via a pointer in CLAUDE.md
2. **Mika agent** — reads it via the self-knowledge skill when users ask about their own setup ("where are my logs?", "what tables are in my DB?")

## Why This Approach

**Problem:** Every new Claude Code conversation starts from scratch when verifying `~/.mika` state. Claude wastes time rediscovering directory layout, DB schema, log locations, and config files. This is non-deterministic and slow.

**Solution:** A detailed reference doc that Claude Code and the Mika agent can consult on demand. Not inlined in CLAUDE.md (which is read every conversation and should stay concise), but available when needed.

**Rejected alternatives:**
- **Claude Code slash commands** — Would create executable verification scripts, but the core problem is knowledge, not automation
- **Mika agent skills with handlers** — Overkill; the agent already has tools to inspect its own filesystem and DB
- **Memory files** — Private to one user, not version-controlled, not available to the agent
- **Inline in CLAUDE.md** — CLAUDE.md is consumed every conversation; detailed schema definitions waste tokens when not needed

## Key Decisions

1. **Single doc at `docs/runtime-structure.md`** — follows existing doc-sync convention (build.rs copies to OUT_DIR, crate-local fallback for crates.io)
2. **CLAUDE.md gets a one-line pointer** — `## Runtime Structure` section with link to the doc
3. **Self-knowledge skill updated** — system prompt references `runtime-structure.md` so the agent knows to check it via `get_documentation`
4. **Content scope:**
   - `~/.mika/` global directory tree (annotated)
   - Per-agent directory tree (`~/.mika/agents/{name}/`)
   - Full SQLite schema v7 (all tables, columns, types, constraints, indexes, views)
   - Config resolution order (env vars → agent config.toml → global config.toml → defaults)
   - Log file locations (CLI, server, gateway)
   - Key file formats (config.toml, identity.toml, mcp.json, skill.toml, team.toml, marketplace.lock)
5. **Kept in sync manually** — schema changes require updating this doc (same as existing docs/ convention)

## Open Questions

None — design is straightforward.

## Next Steps

Run `/ce:plan` to generate the implementation plan, then implement.
