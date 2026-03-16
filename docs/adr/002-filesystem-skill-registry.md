---
title: "ADR-002: Filesystem-Based Skill Registry"
---

# ADR-002: Filesystem-Based Skill Registry

**Date:** 2026-02-25
**Status:** Accepted
**Component:** mika-agent (skills module)

## Context

Mika's 8 tools were hardcoded in Rust: registered in `default_tools()`, with
instructions baked into `prompt.rs`. Adding tools required recompilation. Every
prompt included all tool instructions regardless of relevance.

## Decision

Implement a filesystem-based skill registry at `~/.mika/skills/` with dynamic
loading and keyword-based matching.

### Architecture

Each skill is a directory containing:
- `skill.toml` — manifest (metadata, triggers, handler config)
- `system_prompt.md` — skill-specific prompt instructions (optional)
- `tools.json` — tool schemas for exec/http handlers (optional)

At startup, `SkillRegistry::from_dir()` scans subdirectories and builds an index.
Per turn, `match_message()` returns always-on skills plus keyword-matched skills.
Matched skills contribute tools and prompt snippets to that turn's agent loop.

### Handler Types

- **Builtin** — dispatches to Rust `ToolRegistry` (full DB/context access)
- **Exec** — spawns subprocess, pipes tool input JSON via stdin
- **Http** — POSTs `{tool_name, input}` to configured URL

### Key Design Choices

1. **Lazy loading**: `system_prompt.md` and `tools.json` re-read from disk each turn
   (changes take effect immediately without restart)
2. **Coarse keyword filter**: Simple substring matching; Claude makes the final tool
   selection from the matched set
3. **Graceful degradation**: Invalid manifests log warnings and skip — never prevent
   startup. Missing skills directory falls back to all builtin tools.
4. **Bundled skill re-sync**: Compiled-in templates are seeded on every startup,
   ensuring security updates propagate. Disable with `MIKA_DISABLE_BUNDLED_SKILLS`.

## Consequences

- New tools can be added without recompilation (exec/http handlers)
- Prompt size scales with matched skills, not total skills
- Exec handlers run unsandboxed — the skills directory is the trust boundary
- Skill manifests are scanned once at startup; new skill directories require restart
- Stdin is the data channel to exec handlers (tool input JSON piped via stdin)

### Security Considerations

- Exec handlers inherit Mika's full environment (including API keys)
- Http handler URLs and headers are stored in plaintext in `skill.toml`
- Protect `~/.mika/skills/` with filesystem permissions (bootstrap sets 0700)
- Timeout enforcement prevents misbehaving handlers from blocking the agent loop
