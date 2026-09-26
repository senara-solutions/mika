---
title: "fix: Strip internal context/metadata tags from TUI display"
type: fix
status: completed
date: 2026-03-27
issue: "#223"
---

# fix: Strip internal context/metadata tags from TUI display

## Overview

Internal metadata tags like `<context type="tool_history" trust="metadata">...</context>` are being displayed as raw text in the TUI, `mika ask`, and server outbound messages. These are internal tool context blocks injected by the history builder for LLM context — the LLM echoes them back in its response text, and no stripping exists in the display pipeline.

## Problem Statement

When the agent loop rebuilds conversation history for the LLM (`agent.rs` `format_tool_summary_block()`), it appends `<context type="tool_history" trust="metadata">` blocks to prior assistant messages. The LLM sometimes echoes these tags in its response text. Since there is **no tag-stripping anywhere** between `run_loop()` → `AgentOutput` → display, users see raw XML metadata.

Other internal tags that could similarly leak:
- `<callback_result trust="untrusted">...</callback_result>` — callback framing
- `<task-health>...</task-health>` — heartbeat health data (contains nested `<active-work-items>`, `<anomalies>`, `<task-health-instructions>`)
- `<rewind_reversals trust="internal">...</rewind_reversals>` — rewind context markers
- `<context type="summary" trust="data">` — compaction summaries
- `<context type="skill" trust="local">` — skill prompts

## Proposed Solution

Create a `strip_internal_tags()` function in `mika-common` and apply it at the `AgentOutput` construction boundary — before both display and database persistence. This is the single chokepoint where all response text passes through.

### Architecture Decision: Strip at AgentOutput boundary (before persistence)

**Why:** The tags are re-injected fresh each turn by the history builder (`agent.rs` line 1052-1089) — they are NOT read from stored messages. Stripping them from persisted text loses no information, and ensures the dashboard, introspection tools (`get_session_messages`, `query_timeline`), and all display paths see clean text.

**Alternative rejected:** Strip at each display point (TUI, ask, server, A2A, dashboard). This requires 6+ locations and risks missing one. The dashboard reads from the DB directly, so display-side stripping would require yet another layer.

## Technical Approach

### 1. New function: `strip_internal_tags()` in `mika-common`

**Location:** `crates/mika-common/src/llm/mod.rs` (new public function, accessible to both `mika-agent` and `mika-cli`)

```rust
// crates/mika-common/src/llm/mod.rs
use std::sync::LazyLock;
use regex::Regex;

static INTERNAL_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)<(context|callback_result|task-health|task-health-instructions|active-work-items|anomalies|rewind_reversals)\b[^>]*>.*?</\1>"
    ).expect("internal tag regex")
});

/// Strip internal metadata XML tags from LLM response text.
/// These tags are injected into conversation history for LLM context
/// but should never be displayed to users.
pub fn strip_internal_tags(text: &str) -> String {
    let result = INTERNAL_TAG_RE.replace_all(text, "");
    // Collapse multiple blank lines left by tag removal
    let trimmed = result.trim();
    trimmed.to_string()
}
```

**Key design choices:**
- **Lazy-compiled regex** via `LazyLock` (Rust edition 2024) — compiled once, reused across calls
- **Lazy `.*?` matching** — prevents greedy consumption of text between two separate tag blocks
- **`(?s)` dotall mode** — tags span multiple lines
- **Tag-name capture group with backreference `\1`** — ensures opening and closing tags match
- **Any attributes via `[^>]*`** — handles `type="..."`, `trust="..."` in any order
- **`<think>` excluded** — the existing `extract_think_block()` in `openai.rs` handles think extraction + stripping for OpenAI providers. Unifying is a separate concern.
- **Trim result** — removes whitespace left by tag removal
- **Empty result handling** — callers check for empty string and convert to `None`

### 2. Apply stripping at AgentOutput construction

**File:** `crates/mika-agent/src/agent.rs`

Strip `output.text` in `run_agent_inner()` before constructing `AgentOutput` (~line 1299-1306):

```rust
let cleaned_text = output.text.map(|t| {
    let stripped = mika_common::llm::strip_internal_tags(&t);
    if stripped.is_empty() { None } else { Some(stripped) }
}).flatten();
```

Also apply in `run_silent_agent()` for the silent mode path.

### 3. Apply stripping in `send_message` tool

**File:** `crates/mika-agent/src/tools/send_message.rs`

The `send_message` tool bypasses `AgentOutput` — it calls `MessageSender::send()` directly. Strip the text before sending:

```rust
let cleaned = mika_common::llm::strip_internal_tags(&input.text);
if !cleaned.is_empty() {
    sender.send(&cleaned).await?;
}
```

### 4. System prompt defense-in-depth

**File:** `crates/mika-agent/src/prompt.rs`

Add a brief instruction to the system prompt discouraging the LLM from echoing internal tags:

```
Never include internal XML tags like <context>, <callback_result>, <task-health>, or <rewind_reversals> in your responses. These are system metadata — not for user display.
```

This is preventive. The stripping function is the safety net.

## Acceptance Criteria

- [x] `strip_internal_tags()` function in `mika-common` strips all known internal tag patterns
- [x] Function handles multi-line content, varying attributes, and multiple tags in one response
- [x] Function preserves text between and around tags
- [x] Stripping applied at `AgentOutput` construction in `run_agent_inner()`
- [x] Stripping applied in `run_silent_agent()` return path (via `run_loop()` shared path)
- [x] Stripping applied in `send_message` tool before `sender.send()`
- [x] Empty-after-strip responses handled (convert to `None` / skip send)
- [x] System prompt includes instruction not to echo internal tags
- [x] Unit tests cover: each tag type, mixed tags with normal text, nested `<task-health>`, no-tag passthrough, empty-after-strip, partial/unclosed tags (should be left alone)
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## MVP

### `crates/mika-common/src/llm/mod.rs` — new function

```rust
pub fn strip_internal_tags(text: &str) -> String {
    // Regex strips <context>, <callback_result>, <task-health>,
    // <task-health-instructions>, <active-work-items>, <anomalies>,
    // <rewind_reversals> tags and their content
}
```

### `crates/mika-agent/src/agent.rs` — apply at AgentOutput boundary

Apply `strip_internal_tags()` to `output.text` before returning `AgentOutput` in both `run_agent_inner()` and `run_silent_agent()`.

### `crates/mika-agent/src/tools/send_message.rs` — strip before send

Apply `strip_internal_tags()` to the text before calling `sender.send()`.

### `crates/mika-agent/src/prompt.rs` — defense instruction

Add system prompt instruction discouraging internal tag echo.

## Sources

- Related issue: #223
- Related PR: #221 (added `<think>` tag stripping)
- `crates/mika-common/src/llm/openai.rs:666` — existing `extract_think_block()` pattern
- `crates/mika-agent/src/agent.rs:358` — `format_tool_summary_block()` where `<context>` tags are generated
- `crates/mika-agent/src/agent.rs:166` — `<callback_result>` injection
- `crates/mika-agent/src/prompt.rs:612` — `<task-health>` block construction
- `crates/mika-agent/src/rewind.rs:704` — `<rewind_reversals>` injection
