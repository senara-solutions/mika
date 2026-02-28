---
title: "feat: TUI slash command improvements and web search handler"
type: feat
status: active
date: 2026-02-27
---

# TUI Slash Command Improvements and Web Search Handler

## Overview

Four improvements to the Mika TUI and agent skill system:

1. **`/think` persistent level** — Change `/think` to set the thinking level for all subsequent messages, not just send a single thinking-enabled message
2. **`/model` runtime switching** — Change `/model` to allow selecting and switching models at runtime
3. **Image paste feedback** — Fix clipboard image pasting and improve visual feedback
4. **Web search builtin handler** — Replace the stub `search.sh` with a real Brave Search API builtin handler

## Feature 1: `/think` Persistent Level

### Problem

Currently `/think [low|medium|high] <prompt>` requires a prompt and sends ONE message with thinking enabled. The user expects `/think high` (without a prompt) to set the thinking level for all subsequent messages — the same mental model as Claude Code's `/think` toggle.

### Proposed Solution

Make `/think` dual-mode:
- **`/think [level]`** (no prompt) — sets persistent thinking level for all future messages
- **`/think [level] <prompt>`** (with prompt) — one-shot, same as current behavior
- **`/think off`** — disables persistent thinking

### Technical Approach

**`crates/mika-cli/src/tui/app.rs`:**
- Add field: `pub thinking_level: Option<(u32, &'static str)>` — `(budget, level_name)` or `None` (off)
- Initialize to `None` in `App::new()`

**`crates/mika-cli/src/tui/commands/handlers.rs` — `handle_think()`:**
- If args is exactly `"off"` → set `app.thinking_level = None`, return confirmation message
- If args is exactly `"low"` / `"medium"` / `"high"` (single word, no prompt) → set `app.thinking_level = Some((budget, level))`, return confirmation
- If args has a prompt (existing behavior) → send one-shot message with thinking budget (unchanged)

**`crates/mika-cli/src/tui/app.rs` — `send_message()`:**
- Currently calls `send_message_with_thinking(None)`
- Change to: `send_message_with_thinking(self.thinking_level.map(|(budget, _)| budget))`
- This makes every regular message inherit the persistent thinking level

**`crates/mika-cli/src/tui/ui.rs` — `draw_footer()`:**
- When `app.thinking_level.is_some()`, show `thinking: high` in the footer status bar

**`crates/mika-cli/src/tui/commands/mod.rs`:**
- Update `description` to `"Set thinking level or think once"`
- Update `args_hint` to `Some("[low|medium|high|off] [prompt]")`

### Acceptance Criteria

- [ ] `/think high` sets persistent thinking level, shows "Thinking level: high (5000 tokens)" in command output
- [ ] `/think off` disables persistent thinking, shows confirmation
- [ ] `/think` (no args) shows current level and usage help
- [ ] `/think high explain X` still works as one-shot (unchanged)
- [ ] Regular messages (Enter) use persistent thinking level when set
- [ ] Footer shows `thinking: high` when active
- [ ] Thinking level does NOT persist across sessions (runtime-only)

### Files

```
crates/mika-cli/src/tui/app.rs           — Add thinking_level field, modify send_message()
crates/mika-cli/src/tui/commands/mod.rs   — Update command description/args_hint
crates/mika-cli/src/tui/commands/handlers.rs — Modify handle_think() for dual-mode
crates/mika-cli/src/tui/ui.rs            — Show thinking level in footer
```

---

## Feature 2: `/model` Runtime Switching

### Problem

Currently `/model` is display-only (`format!("Current model: {}", app.model)`). The user wants to switch models at runtime from the TUI — e.g., `/model opus` to switch to Opus for a complex task.

### Proposed Solution

Make `/model` switch models when given an argument:
- **`/model`** (no args) — show current model + available models
- **`/model <name>`** — switch to the named model

Support shorthand names: `sonnet` → `claude-sonnet-4-6`, `opus` → `claude-opus-4-6`, `haiku` → `claude-haiku-4-5`.

### Technical Approach

**Model name resolution** — add a helper function in `handlers.rs`:

```rust
fn resolve_model_name(input: &str) -> Option<&'static str> {
    match input.to_lowercase().as_str() {
        "sonnet" | "claude-sonnet-4-6" => Some("claude-sonnet-4-6"),
        "opus" | "claude-opus-4-6" => Some("claude-opus-4-6"),
        "haiku" | "claude-haiku-4-5" => Some("claude-haiku-4-5"),
        _ => None,
    }
}
```

**`crates/mika-cli/src/tui/app.rs` — `AgentRequest`:**
- Add variant: `SetModel { model: String }`

**`crates/mika-cli/src/tui/commands/handlers.rs` — `handle_model()`:**
- No args: show current model + list of available models (sonnet, opus, haiku)
- With args: resolve shorthand, update `app.model` for display, send `AgentRequest::SetModel` to worker

**`crates/mika-cli/src/commands/chat.rs` — agent worker:**
- Handle `AgentRequest::SetModel { model }` — update `worker_claude.model = model`

**`crates/mika-cli/src/tui/commands/mod.rs`:**
- Update `description` to `"Show or switch model"`
- Update `args_hint` to `Some("[sonnet|opus|haiku]")`

### Acceptance Criteria

- [ ] `/model` shows current model and available options
- [ ] `/model opus` switches to `claude-opus-4-6`, shows confirmation
- [ ] `/model sonnet` switches to `claude-sonnet-4-6`
- [ ] `/model haiku` switches to `claude-haiku-4-5`
- [ ] `/model invalid` shows error with available options
- [ ] Footer shows updated model name after switch
- [ ] Subsequent messages use the new model
- [ ] Model does NOT persist across sessions (runtime-only)

### Files

```
crates/mika-cli/src/tui/app.rs              — Add AgentRequest::SetModel variant
crates/mika-cli/src/tui/commands/mod.rs      — Update command description/args_hint
crates/mika-cli/src/tui/commands/handlers.rs — Implement model switching with resolution
crates/mika-cli/src/commands/chat.rs         — Handle SetModel in worker loop
```

---

## Feature 3: Image Paste Feedback

### Problem

The user reports not seeing anything happen when pasting images. The code infrastructure exists:
- `try_clipboard_image()` in `input.rs` uses `arboard::Clipboard::get_image()`
- `attach_image()` adds to `pending_images`
- `draw_input()` shows `Attached: [Image #1 123KB] (Esc to clear)` when images exist

The likely issue is that `arboard` clipboard image support fails silently on Gentoo Linux (X11/Wayland). The `get_image()` call returns `Err()` which maps to `None` via `.ok()?`, and the Ctrl+V falls through to normal text paste with no feedback.

### Proposed Solution

1. **Add xclip/wl-clipboard fallback** for image clipboard reading on Linux
2. **Show feedback on Ctrl+V** — when clipboard is checked for images, show a brief status if no image found
3. **Improve attachment indicator** to be more noticeable

### Technical Approach

**`crates/mika-cli/src/tui/input.rs` — `try_clipboard_image()`:**
- Add logging: `tracing::debug!("clipboard image attempt: ...")` for diagnosis
- Add Linux fallback: if `arboard` fails, try `xclip -selection clipboard -t image/png -o` via `Command`
- Return informative error type instead of just `Option` to distinguish "no image" from "clipboard error"

**New return type:**

```rust
enum ClipboardImageResult {
    Image(ImageAttachment),
    NoImage,        // clipboard accessible but no image
    Error(String),  // clipboard not accessible
}
```

**`crates/mika-cli/src/tui/input.rs` — `handle_key_normal()`:**
- On Ctrl+V: call `try_clipboard_image()`
- `Image(att)` → attach as before
- `NoImage` → fall through to text paste (no message)
- `Error(msg)` → show one-time hint: "Clipboard image access failed: {msg}. Use /attach <path> instead."

**`crates/mika-cli/src/tui/input.rs` — Linux xclip fallback:**

```rust
fn try_xclip_image() -> Option<ImageAttachment> {
    let output = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    // Validate PNG magic bytes, encode to base64
    // ... (reuse existing validation logic)
}
```

**`Cargo.toml` (mika-cli):**
- No new dependencies needed — `xclip`/`wl-paste` are called via `std::process::Command`

### Acceptance Criteria

- [ ] Ctrl+V on image clipboard shows "Attached: [Image #1 245KB] (Esc to clear)"
- [ ] If arboard fails, tries xclip fallback (Linux only)
- [ ] If both fail, falls through to text paste (no error spam)
- [ ] First clipboard failure in session logs a debug hint about /attach
- [ ] `/attach <path>` continues to work as direct fallback
- [ ] Esc clears attachments (existing behavior preserved)
- [ ] Max 10 attachments, 20MB total (existing limits preserved)

### Files

```
crates/mika-cli/src/tui/input.rs  — Add fallback, improve error handling
```

---

## Feature 4: Web Search Builtin Handler

### Problem

The bundled `web-search` skill has a stub `search.sh` that prints "TODO: Implement web search". The user wants a real web search implementation like OpenClaw's, which supports Brave Search API.

### Proposed Solution

Implement web search as a **builtin handler** (Rust, not shell script) using the Brave Search API. This provides:
- Type safety and proper error handling
- No subprocess overhead
- Direct reqwest integration (already a dependency)
- Proper result formatting for LLM consumption

### Technical Approach

**Phase 1: Builtin handler implementation**

**`crates/mika-agent/src/skills/builtin_handlers.rs`:**

Add `web_search` to `KNOWN_BUILTINS`:
```rust
pub const KNOWN_BUILTINS: &[&str] = &[
    "get_cli_reference",
    "get_api_spec",
    "get_architecture_overview",
    "web_search",
];
```

Add dispatch case:
```rust
"web_search" => web_search(&input, ctx).await,
```

Implement the handler:
```rust
async fn web_search(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim(),
        _ => return ToolOutput::error("Missing or empty 'query' parameter".to_string()),
    };

    // Read API key from config file in home_dir
    let api_key = match load_brave_api_key(ctx.home_dir) {
        Some(key) => key,
        None => return ToolOutput::error(
            "Brave Search API key not configured. Set brave_api_key in ~/.mika/config.toml \
             or MIKA_BRAVE_API_KEY env var.".to_string()
        ),
    };

    // Call Brave Search API
    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", &api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", "5")])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolOutput::error(format!("Search request failed: {e}")),
    };

    if !resp.status().is_success() {
        return ToolOutput::error(format!("Search API returned {}", resp.status()));
    }

    // Parse and format results
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return ToolOutput::error(format!("Failed to parse response: {e}")),
    };

    format_brave_results(&body, query)
}
```

**Result formatting** — concise, LLM-friendly:
```
Search results for "query":

1. Title
   URL: https://...
   Description text here

2. Title
   URL: https://...
   Description text here
```

**`crates/mika-common/src/config.rs`:**
- Add field: `pub brave_api_key: Option<String>` with `#[serde(default)]`
- Add to Debug impl with redaction

**`templates/skills/web-search/tools.json`:**
- Change handler from `exec` to `builtin`:
```json
{
    "handler": {
        "type": "builtin",
        "function": "web_search"
    }
}
```

**`templates/skills/web-search/handlers/search.sh`:**
- Remove (no longer needed since handler is builtin)

**`crates/mika-agent/src/bundled_skills.rs`:**
- Remove the `search.sh` handler entry from `WEB_SEARCH_SKILL`

### Acceptance Criteria

- [ ] `web_search` tool works when `MIKA_BRAVE_API_KEY` is set
- [ ] Missing API key returns clear error message with setup instructions
- [ ] Results formatted concisely for LLM consumption
- [ ] 15-second timeout on HTTP request
- [ ] Output truncated to 10,000 chars (existing truncation)
- [ ] Empty results handled gracefully ("No results found for...")
- [ ] Network errors return clear error messages
- [ ] Skill triggers on keywords: "search", "look up", "find online", "google", "latest"

### Files

```
crates/mika-agent/src/skills/builtin_handlers.rs  — Add web_search handler + KNOWN_BUILTINS
crates/mika-common/src/config.rs                   — Add brave_api_key field
templates/skills/web-search/tools.json             — Change handler to builtin
templates/skills/web-search/handlers/search.sh     — Remove (replaced by builtin)
crates/mika-agent/src/bundled_skills.rs            — Remove search.sh from WEB_SEARCH_SKILL
.env.example                                        — Add MIKA_BRAVE_API_KEY
```

---

## Implementation Order

1. **Feature 1: `/think` persistent level** — smallest change, self-contained in CLI crate
2. **Feature 2: `/model` runtime switching** — touches CLI + worker communication
3. **Feature 3: Image paste feedback** — input handling improvements
4. **Feature 4: Web search handler** — agent crate changes, new config field

## Dependencies & Risks

- **No new crate dependencies** — all features use existing deps (reqwest, arboard, serde)
- **Feature 4** requires a Brave Search API key for testing (free tier available at brave.com/search/api)
- **Feature 3** xclip fallback is Linux-only; macOS/Windows continue using arboard directly
- **Feature 2** worker model switch uses existing channel; no architectural changes

## References

- Existing slash command architecture: `docs/reviews/2026-02-25-slash-commands-architecture-review.md`
- Security patterns: `docs/solutions/security-issues/code-review-7aba1ec-shell-injection-memory-safety.md`
- OpenClaw web search (studied): `/home/samidarko/workspace/senara-solutions/openclaw/src/agents/tools/web-search.ts`
- Brave Search API: `https://api.search.brave.com/app/documentation/web-search/get-started`
