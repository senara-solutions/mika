---
title: "Hierarchical CLAUDE.md split for per-crate context"
category: infrastructure
date: 2026-04-11
severity: medium
tags: [claude-code, documentation, context-management, developer-experience]
module: root
issue: "#476"
---

# Hierarchical CLAUDE.md Split

## Problem

The root `CLAUDE.md` grew to ~79k characters (217 lines of dense content), nearly double Claude Code's ~40k performance threshold. Every Claude Code interaction paid the full context cost regardless of which crate was being worked on, reducing response quality and displacing useful conversation context.

## Root Cause

Organic growth — every new feature, tool, schema migration, and architectural decision was appended to a single monolithic file. The Architecture section alone was 48.7k characters (61.6% of the file), containing detailed internals for all 5 crates.

## Solution

Split into 7 hierarchical CLAUDE.md files leveraging Claude Code's automatic loading behavior (loads every CLAUDE.md from git root down to current working directory):

| File | Size | Content |
|------|------|---------|
| `CLAUDE.md` (root) | ~16k | Project overview, cross-cutting conventions, env vars, commands, architecture summary with pointers |
| `crates/mika-agent/CLAUDE.md` | ~16k | Agent loop, skills, tools, tasks, memory, observability, schema |
| `crates/mika-gateway/CLAUDE.md` | ~5k | Webhooks, routing, GitHub App, gateway env vars |
| `crates/mika-cli/CLAUDE.md` | ~6k | TUI, CLI commands, slash commands |
| `crates/mika-common/CLAUDE.md` | ~5k | LLM providers, API client, prompt caching |
| `crates/mika-a2a/CLAUDE.md` | ~1k | A2A protocol types and state machine |
| `dashboard/CLAUDE.md` | ~4k | React dashboard, build commands |

**Key design decisions:**
1. **Root retains compact summaries** — even when working from repo root (most common), the root file provides enough context with 2-3 sentence summaries per subsystem
2. **Cross-crate topics use "primary + pointer" pattern** — full detail in one file, 1-2 sentence summary with pointer in secondary locations
3. **Schema history condensed** — only last 3 migrations in CLAUDE.md; full history in `docs/runtime-structure.md`
4. **No `crates/CLAUDE.md` intermediate** — shared Rust conventions stay in root; developers work in specific crate dirs, not `crates/`

**Content loss prevention:** Automated grep audit of 15 critical terms verified all content was relocated. One gap found (missing function name `detect_completion_claim`) and immediately fixed.

**Doc audit command updated:** `.claude/commands/mika-doc-audit.md` now includes a mapping table directing which change categories update which CLAUDE.md file.

## Prevention

- **Size monitoring:** When adding content to any CLAUDE.md, check `wc -c` stays under 35k (leaving headroom below the 40k threshold)
- **Content placement:** Use the doc audit mapping table to place content in the most locally relevant CLAUDE.md
- **One owner per topic:** Each architectural topic should have a single primary CLAUDE.md file. Use pointers from secondary files, not duplication.

## Related

- Issue: #476
- `docs/solutions/integration-issues/skills-doc-code-drift-and-validation-infrastructure.md` — doc-code drift patterns
