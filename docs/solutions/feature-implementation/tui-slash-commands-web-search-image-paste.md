---
title: "TUI Slash Commands, Web Search & Image Paste Implementation"
date: 2026-02-27
category: feature-implementation
component: mika-cli, mika-agent
tags:
  - tui
  - slash-commands
  - thinking-level
  - model-switching
  - image-paste
  - web-search
  - builtin-handlers
  - clipboard-fallback
problem_type: feature-implementation
severity: enhancement
resolution_time: "~3 hours"
related_issues: []
---

# TUI Slash Commands, Web Search & Image Paste

## Problem Statement

Four TUI features were stubs or missing:

1. `/think` only sent one-shot messages with extended thinking — no persistent level toggle
2. `/model` displayed current model but couldn't switch at runtime
3. Ctrl+V for images failed silently on Linux (arboard clipboard errors swallowed)
4. `web_search` skill spawned a stub shell script instead of calling an API

## Root Cause

- `/think` was designed as one-shot only — no App state field for persistence
- `/model` handler was display-only — no `AgentRequest` variant for model switching
- `arboard::Clipboard::get_image()` errors mapped to `None` via `.ok()?` with no fallback
- Web search skill used exec handler type pointing to unimplemented `search.sh`

## Solution

### Feature 1: `/think` Persistent Thinking Level

Added `thinking_level: Option<(u32, &'static str)>` to `App` state. Dual-mode handler:

- `/think high` — sets persistent level (all subsequent messages use extended thinking)
- `/think high <prompt>` — one-shot message with thinking budget
- `/think off` — disables persistent thinking
- Footer shows active level in magenta

```rust
fn resolve_thinking_level(word: &str) -> Option<(u32, &'static str)> {
    match word.to_lowercase().as_str() {
        "low" => Some((5_000, "low")),
        "medium" | "med" => Some((10_000, "medium")),
        "high" => Some((50_000, "high")),
        _ => None,
    }
}
```

Integration: `send_message()` reads `self.thinking_level.map(|(b, _)| b)` to include budget in every `AgentRequest::Message`.

**Files:** `app.rs`, `handlers.rs`, `ui.rs`, `mod.rs`

### Feature 2: `/model` Runtime Switching

Added `AgentRequest::SetModel { model: String }` variant. Handler resolves shorthand aliases:

```rust
const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "claude-haiku-4-5", "Claude Haiku 4.5"),
];
```

Worker receives `SetModel` and mutates `worker_claude.model` directly. Both `app.model` and `app.claude.model` updated for UI consistency.

**Files:** `app.rs`, `handlers.rs`, `chat.rs` (worker needed `let mut worker_claude`), `mod.rs`

### Feature 3: Image Paste Clipboard Fallback

Replaced `Option<ImageAttachment>` with structured enum:

```rust
enum ClipboardResult {
    Image(ImageAttachment),  // Successfully read image
    NoImage,                 // Clipboard accessible, no image
    Error(String),           // Clipboard not accessible
}
```

Fallback chain: arboard -> xclip -> wl-paste (Linux only via `#[cfg(target_os = "linux")]`). Shared helper `png_bytes_to_attachment()` validates PNG magic bytes, size limits, and base64 encodes.

**Files:** `input.rs`

### Feature 4: Web Search Builtin Handler

Converted from shell exec to builtin handler calling Brave Search API:

- Reads `MIKA_BRAVE_API_KEY` from env (not from Settings — builtin handlers only have `ToolContext`)
- GET `https://api.search.brave.com/res/v1/web/search` with 15s timeout
- Returns top 5 results as numbered list (title, URL, description)
- Proper error handling: missing key, 401, 429, timeout

**Files:** `builtin_handlers.rs`, `bundled_skills.rs`, `templates/skills/web-search/tools.json`

## Key Patterns

### Adding TUI Slash Commands

1. Register in `COMMANDS` array (`mod.rs`) with description and args_hint
2. Add dispatch case in `handle_command()` (`handlers.rs`)
3. For persistent state: add field to `App` struct, integrate in `send_message()`
4. For worker communication: add `AgentRequest` variant, handle in worker match

### Adding AgentRequest Variants

1. Add variant to `AgentRequest` enum in `app.rs`
2. Add match arm in worker loop (`chat.rs`)
3. Send via `app.agent_tx.send()` from handler
4. Update both TUI state (immediate) and worker state (async via channel)

### Adding Builtin Handlers

1. Add function name to `KNOWN_BUILTINS` array
2. Add dispatch case in `execute()` match
3. Accept `serde_json::Value` input, return `ToolOutput`
4. Read secrets from env (not Settings — builtins don't have access)
5. Output auto-truncated to 10k chars

### Avoiding Dead Code in Config

If a builtin handler reads config from env directly, do NOT also add the field to `Settings`. The builtin handler only has `ToolContext` (db, session_id, home_dir) — not `Settings`. Adding a field to Settings that's never read creates dead code.

## Prevention Strategies

- **Test level resolution exhaustively** — case variations, boundary values
- **Validate at set time, not use time** — `resolve_thinking_level()` rejects invalid input immediately
- **Use debug-level logging for expected clipboard failures** — don't show errors for normal fallback behavior
- **Check HTTP status before parsing JSON** — prevents confusing parse errors on 401/500 responses
- **Magic byte validation for file formats** — map extension to expected bytes, reject mismatches

## Related Documentation

- `docs/solutions/ui-bugs/tui-ask-visibility-skill-seeder-config-tools.md` — TUI slash command architecture
- `docs/solutions/code-review-workflow/mika-cli-21-findings-parallel-resolution.md` — TUI rendering patterns
- `docs/plans/2026-02-27-feat-tui-slash-commands-and-web-search-plan.md` — Implementation plan
