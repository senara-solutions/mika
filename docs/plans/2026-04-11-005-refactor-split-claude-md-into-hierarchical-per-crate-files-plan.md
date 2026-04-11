---
title: "refactor: split CLAUDE.md into hierarchical per-crate context files"
type: refactor
status: completed
date: 2026-04-11
issue: "#476"
---

# refactor: split CLAUDE.md into hierarchical per-crate context files

## Overview

The root `CLAUDE.md` is ~79k characters, nearly double Claude Code's ~40k performance threshold. Split it into a hierarchy of scoped files: a lean root index plus per-crate/subdirectory files that Claude Code loads automatically based on working directory.

## Problem Statement

Claude Code loads every `CLAUDE.md` from git root down to the current working directory. With a single 79k file at root, every interaction pays the full context cost regardless of scope. Performance degrades (slower responses, less room for conversation context), and crate-irrelevant information (gateway details when working on the agent loop) displaces useful context.

## Proposed Solution

Split into 7 files. Root becomes a compact index (~20k); crate-specific detail moves to subdirectory files.

### File Layout & Budget

| File | Target chars | Content scope |
|------|-------------|---------------|
| `CLAUDE.md` (root) | ~20k | Project overview, stack summary, directory index, cross-cutting conventions, env vars, commands, versioning, pending work, workspace context |
| `crates/mika-agent/CLAUDE.md` | ~30k | Agent loop, skills system, task engine, tools, memory model, work items, management tools, observability, silent mode, compaction, rewind, schema version (condensed), mika-server HTTP endpoints, PR merge gate |
| `crates/mika-gateway/CLAUDE.md` | ~5k | Webhook routing, GitHub App integration, A2A proxy, gateway-specific env vars, request logging, agent identification, reply routing, build.rs |
| `crates/mika-cli/CLAUDE.md` | ~4k | TUI, clap subcommands, slash commands, tab completion, input modes, `mika ask` flags, team mode CLI, dashboard CLI |
| `crates/mika-common/CLAUDE.md` | ~4k | Config system, LLM providers, Claude API client, prompt caching, OAuth, GitHub App auth, model list cache, MockLlmProvider, typed errors, internal tag stripping, XML tool call extraction |
| `crates/mika-a2a/CLAUDE.md` | ~2k | A2A protocol v0.3 types, JSON-RPC, task state machine, SSE streaming, `a2a_call` builtin |
| `dashboard/CLAUDE.md` | ~2k | React/TS/Vite stack, pages, auth, `@senara-solutions/ui` package, build commands, dashboard-specific env vars |

**Total: ~67k across 7 files.** Root + any single crate ≤ 50k (within budget since 40k is a soft threshold per-file, not aggregate). The agent crate — the densest — loads as root(20k) + agent(30k) = 50k, which is acceptable.

### Root CLAUDE.md Design

The root keeps everything that is needed regardless of which crate you're working in:

**Keep in root (full detail):**
- Project overview & current phase
- Stack (high-level, remove per-crate detail)
- Directory structure (as an index — 2-3 lines per crate/dir with "See `crates/X/CLAUDE.md`" pointers)
- Cross-cutting conventions: error handling (`anyhow`/`thiserror`), naming, edition 2024, testing pattern, database (COLLATE NOCASE), secrets (redaction), labels, async DB, doc sync
- Environment variables (full — referenced from any crate)
- Top-level commands (`cargo build/test/clippy/fmt`, `make deploy`, Docker, `mika` subcommands)
- Versioning policy
- Pending work
- Reference repositories
- Workspace context

**Move out of root:**
- Architecture section (48.7k, 61.6% of file) — decomposed into crate CLAUDE.md files
- Crate-specific conventions (tools, prompt, skills, A2A, prompt caching)
- Crate-specific directory structure detail

**New in root:** A "Hierarchical context" note at the top explaining the loading behavior, so future maintainers understand the split rationale.

### Content Allocation for Cross-Crate Topics

Several topics span crate boundaries. Strategy: **full detail in the primary crate, 1-2 sentence summary with pointer in secondary crates.**

| Topic | Primary home | Secondary mention |
|-------|-------------|-------------------|
| A2A protocol types | `crates/mika-a2a/CLAUDE.md` | Agent: "A2A server endpoints at `/a2a/{agent_name}`; see `crates/mika-a2a/CLAUDE.md` for protocol". Gateway: "A2A proxy; see `crates/mika-a2a/CLAUDE.md`" |
| Skills system | `crates/mika-agent/CLAUDE.md` | CLI: "Skills CLI commands; skill architecture in `crates/mika-agent/CLAUDE.md`" |
| Schema/DB | `crates/mika-agent/CLAUDE.md` | Root: "Schema v21, SQLite per agent. See `crates/mika-agent/CLAUDE.md` for migration history" |
| LLM providers | `crates/mika-common/CLAUDE.md` | Agent: "LLM calls via `LlmProvider` trait; see `crates/mika-common/CLAUDE.md`" |
| Prompt caching | `crates/mika-common/CLAUDE.md` | Agent: pointer only |
| Observability | `crates/mika-agent/CLAUDE.md` | Common: "OTel export feature-gated; see `crates/mika-agent/CLAUDE.md` for observability" |
| Docker images | Root (cross-crate) | — |

### Schema Version Handling

The schema version history (4.7k) stays in `crates/mika-agent/CLAUDE.md` but in **condensed form**: current schema number, table listing, and migration notes for only the last 3-4 migrations. The full migration history is already documented in `docs/runtime-structure.md`. A pointer replaces the historical detail: "Full migration history: `docs/runtime-structure.md`".

This reduces the schema section from ~4.7k to ~2k, keeping the agent file at ~28k.

## Technical Considerations

### Claude Code Working Directory Behavior

Claude Code typically operates from the repo root. Crate-level CLAUDE.md files are loaded when:
1. The user explicitly `cd`s into a crate directory
2. Claude Code `cd`s for build commands (`cargo test -p mika-agent`)
3. Files are opened in a crate directory (some IDE integrations)

**Mitigation for root-only sessions:** The root CLAUDE.md retains compact summaries of each major subsystem (2-3 sentences each) so that working from root is still productive. The crate files add depth, not essential context.

### Doc Audit Command

The `/mika-doc-audit` command references "CLAUDE.md" as a single file. After the split:
- Update the command to audit all 7 CLAUDE.md files
- Include a mapping: "schema changes → `crates/mika-agent/CLAUDE.md`", "env var changes → root `CLAUDE.md`", etc.

### No `crates/CLAUDE.md` Intermediate

Creating `crates/CLAUDE.md` is unnecessary. Shared Rust conventions (edition, error handling, naming) live in root. Each crate file focuses on its own architecture, not shared Rust patterns.

## Acceptance Criteria

- [ ] Root `CLAUDE.md` under 40k characters (target: ~20k)
- [ ] No content removed — every piece of information from the current file exists in exactly one of the new files
- [ ] Each subdirectory CLAUDE.md is self-contained: includes enough context to understand the crate's architecture without reading root
- [ ] Root CLAUDE.md contains compact summaries of each major subsystem with pointers to crate files
- [ ] Cross-crate topics use "primary + pointer" pattern (full detail in one file, 1-2 sentence summary + pointer in others)
- [ ] `/mika-doc-audit` command updated to audit all CLAUDE.md files
- [ ] `wc -c` on each file confirms all are under 40k

## Implementation Phases

### Phase 1: Content Audit & Categorization

Read the current CLAUDE.md and categorize every section into one of the 7 destination files. Create a mapping document (or use this plan as the mapping). Verify no content is uncategorized.

**Files touched:** None (analysis only)

### Phase 2: Create Crate CLAUDE.md Files

Write the 6 new CLAUDE.md files (`crates/mika-agent/`, `crates/mika-gateway/`, `crates/mika-cli/`, `crates/mika-common/`, `crates/mika-a2a/`, `dashboard/`). Each file:
- Starts with a `# <Crate Name>` heading and 1-sentence purpose
- Contains the full detail relocated from root
- Includes cross-references to other crate files where topics span boundaries
- Is independently useful

**Files created:**
- `crates/mika-agent/CLAUDE.md`
- `crates/mika-gateway/CLAUDE.md`
- `crates/mika-cli/CLAUDE.md`
- `crates/mika-common/CLAUDE.md`
- `crates/mika-a2a/CLAUDE.md`
- `dashboard/CLAUDE.md`

### Phase 3: Rewrite Root CLAUDE.md

Replace the monolithic root with the lean index. Preserve all cross-cutting content. Replace crate-specific sections with compact summaries + pointers.

**Files modified:**
- `CLAUDE.md`

### Phase 4: Update Doc Audit Command

Modify `.claude/commands/mika-doc-audit.md` to reference all 7 CLAUDE.md files with guidance on which file to update for which category of change.

**Files modified:**
- `.claude/commands/mika-doc-audit.md`

### Phase 5: Verification

- `wc -c` on all 7 files — all under 40k
- Diff the concatenation of all 7 files against the original to verify no content was lost (semantic comparison, not byte-exact)
- Verify cross-references point to correct files

## Success Metrics

- Root `CLAUDE.md` ≤ 20k chars (74% reduction from 79k)
- No crate CLAUDE.md exceeds 35k chars
- Root + largest crate (agent) ≤ 50k chars combined
- Zero content loss (every section from original is in exactly one destination file)

## Dependencies & Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Agent CLAUDE.md too large (>35k) | Marginal improvement | Condense schema history, move full migration log to docs/ |
| Claude Code stays at root (crate files never loaded) | Reduced benefit | Root retains compact summaries; crate files add depth not essentials |
| Doc audit misses crate-level updates | Stale docs | Update `/mika-doc-audit` command in same PR |
| Content drift between files | Contradictions | Cross-references use "primary + pointer" pattern; only one file owns each topic |

## Sources & References

- Issue: #476
- Current CLAUDE.md: 78,999 chars, 61.6% is the Architecture section
- Claude Code docs: hierarchical CLAUDE.md loading (root → current working dir)
- `docs/runtime-structure.md`: existing schema documentation (target for schema history pointer)
- `docs/solutions/integration-issues/skills-doc-code-drift-and-validation-infrastructure.md`: doc drift prevention patterns
