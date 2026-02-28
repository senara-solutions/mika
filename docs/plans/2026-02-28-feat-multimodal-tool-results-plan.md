---
title: "feat: Multi-modal tool results (images in tool_result content blocks)"
type: feat
status: completed
date: 2026-02-28
---

# Multi-Modal Tool Results

## Overview

Enable Mika's agent loop to handle multi-modal tool results — specifically, images in `tool_result` content blocks. When a tool produces an image (e.g., a shell script takes a screenshot, or the file-reader opens an image file), the agent should be able to "see" that image through Claude's vision capabilities.

## Problem Statement

The screenshot below captures the exact failure mode:

```
You: can you take a screenshot of my workspace 1 and tell me what you see?

Mika:
I can't read the screenshot file from disk directly — I can only analyze
images you send me. But hyprctl gave me a clear picture of what's on workspace 1...
```

Mika has **all the pieces** — it can execute shell commands (screenshot skill), it can read files (file-reader skill), and Claude's vision can analyze images. But these capabilities are disconnected because:

1. **`ToolOutput` is text-only** — `{ content: String, is_error: bool }` (`crates/mika-agent/src/tools/mod.rs:64-68`)
2. **`ContentBlock::ToolResult` is text-only** — `content: String` (`crates/mika-common/src/claude.rs:98-104`)
3. **No bridge exists** between "tool produced a file on disk" and "include that file as an image in the next Claude API call"

The Claude Messages API already supports multi-block `tool_result` content (array of text + image blocks), but Mika only implements the string shorthand form.

## Proposed Solution

A generic infrastructure change across three layers — not about making specific skills into builtins, but about enabling **any tool** to return images alongside text.

## Technical Approach

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Claude API (already supports images in tool_result)    │
└───────────────────────────┬─────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────┐
│  ContentBlock::ToolResult { content: ToolResultBody }   │  ← Phase 1
│  ToolResultBody::Text(String) | Blocks(Vec<...>)        │
└───────────────────────────┬─────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────┐
│  process_tool_calls() in agent.rs                       │  ← Phase 2
│  Converts ToolOutput → ContentBlock::ToolResult         │
│  If images present → multi-block, else → text shorthand │
└───────────────────────────┬─────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────┐
│  ToolOutput { content, is_error, images }               │  ← Phase 2
│  images: Vec<ImageData> — base64-encoded                │
└──────┬──────────────┬──────────────┬────────────────────┘
       │              │              │
  ┌────┴────┐   ┌─────┴─────┐  ┌────┴──────┐
  │ Builtin │   │ Exec      │  │ Builtin   │  ← Phase 3
  │ Tools   │   │ Handlers  │  │ Handlers  │
  │ (Rust)  │   │ (scripts) │  │ (Rust)    │
  └─────────┘   └─────┬─────┘  └───────────┘
                      │
            ┌─────────┴──────────┐
            │ Image Envelope     │  ← Phase 3
            │ Protocol           │
            │ __mika_v1 JSON     │
            └────────────────────┘
```

### Implementation Phases

#### Phase 1: Claude API Type System (`crates/mika-common/src/claude.rs`)

**Goal:** Make `ContentBlock::ToolResult` support both string and multi-block content, matching the Claude API spec.

**Changes:**

- [x] Define `ToolResultBlock` enum for content inside tool results:
  ```rust
  // crates/mika-common/src/claude.rs
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(tag = "type")]
  pub enum ToolResultBlock {
      #[serde(rename = "text")]
      Text { text: String },
      #[serde(rename = "image")]
      Image { source: ImageSource },
  }
  ```

- [x] Define `ToolResultBody` enum for the `content` field (string or array):
  ```rust
  // crates/mika-common/src/claude.rs
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(untagged)]
  pub enum ToolResultBody {
      Text(String),
      Blocks(Vec<ToolResultBlock>),
  }
  ```

- [x] Update `ContentBlock::ToolResult` to use `ToolResultBody`:
  ```rust
  ToolResult {
      tool_use_id: String,
      content: ToolResultBody,
      #[serde(skip_serializing_if = "Option::is_none")]
      is_error: Option<bool>,
  }
  ```

- [x] Add comprehensive serde round-trip tests:
  - Serialize text-only → `"content": "text here"` (string shorthand)
  - Serialize with images → `"content": [{"type": "text", ...}, {"type": "image", ...}]` (array form)
  - Deserialize string form back to `ToolResultBody::Text`
  - Deserialize array form back to `ToolResultBody::Blocks`

**Files:** `crates/mika-common/src/claude.rs`

**Estimated tests:** ~6 new serde round-trip tests

#### Phase 2: ToolOutput and Agent Loop (`crates/mika-agent/src/`)

**Goal:** Extend `ToolOutput` to carry images and update `process_tool_calls()` to convert them into multi-block tool results.

**Changes to `crates/mika-agent/src/tools/mod.rs`:**

- [x] Define `ImageData` struct:
  ```rust
  #[derive(Debug, Clone)]
  pub struct ImageData {
      pub media_type: String,  // "image/png", "image/jpeg", etc.
      pub data: String,        // base64-encoded
  }
  ```

- [x] Add `images` field to `ToolOutput`:
  ```rust
  pub struct ToolOutput {
      pub content: String,
      pub is_error: bool,
      pub images: Vec<ImageData>,
  }
  ```

- [x] Update `ToolOutput::success()` and `ToolOutput::error()` to default `images: vec![]` (zero breaking changes to existing callers)

- [x] Add `ToolOutput::success_with_images(text, images)` constructor

**Changes to `crates/mika-agent/src/agent.rs` (`process_tool_calls()`, ~line 646):**

- [x] When `output.images` is empty → use `ToolResultBody::Text(output.content)` (backward compatible, same as today)
- [x] When `output.images` is non-empty → build `ToolResultBody::Blocks(...)` with text block + image blocks:
  ```rust
  if output.images.is_empty() {
      ContentBlock::ToolResult {
          tool_use_id: id.clone(),
          content: ToolResultBody::Text(output.content),
          is_error: ...,
      }
  } else {
      let mut blocks = vec![ToolResultBlock::Text { text: output.content }];
      for img in &output.images {
          blocks.push(ToolResultBlock::Image {
              source: ImageSource {
                  source_type: "base64".to_string(),
                  media_type: img.media_type.clone(),
                  data: img.data.clone(),
              },
          });
      }
      ContentBlock::ToolResult {
          tool_use_id: id.clone(),
          content: ToolResultBody::Blocks(blocks),
          is_error: ...,
      }
  }
  ```

- [x] Update `ToolCallSummary.output_summary` to append `[+N image(s)]` when images are present

- [x] Fix all compilation errors from the `ToolResult { content: String }` → `ToolResult { content: ToolResultBody }` change across the codebase (history builder, compaction, any pattern matches)

**Files:** `crates/mika-agent/src/tools/mod.rs`, `crates/mika-agent/src/agent.rs`

**Estimated tests:** ~4 new tests (process_tool_calls with/without images, ToolOutput constructors)

#### Phase 3: Exec Handler Image Protocol (`crates/mika-agent/src/skills/executor.rs`)

**Goal:** Define and implement a protocol for exec handler scripts to signal image file references in their output.

**The `__mika_v1` envelope protocol:**

Scripts that want to return images output a JSON object to stdout with the sentinel key `__mika_v1`:

```json
{"__mika_v1": {"text": "Screenshot saved to /tmp/screenshot.png", "images": ["/tmp/screenshot.png"]}}
```

Schema:
- `__mika_v1.text` (string, required) — the text portion of the result
- `__mika_v1.images` (array of strings, required) — absolute file paths to image files

Detection heuristic in `execute_exec()`:
1. Attempt `serde_json::from_str()` on stdout
2. If parse succeeds AND the root object has `__mika_v1` key → treat as image envelope
3. Otherwise → treat as plain text (backward compatible)

**Changes to `crates/mika-agent/src/skills/executor.rs`:**

- [x] Define envelope types:
  ```rust
  #[derive(Deserialize)]
  struct MikaEnvelope {
      __mika_v1: MikaOutput,
  }
  #[derive(Deserialize)]
  struct MikaOutput {
      text: String,
      images: Vec<String>,  // file paths
  }
  ```

- [x] Add `try_parse_envelope()` function that attempts JSON parse with sentinel key detection

- [x] Add `read_and_validate_image(path)` function:
  - Check `std::fs::metadata().len()` ≤ 5MB before reading
  - Read file bytes
  - Magic-byte validation: JPEG (`FF D8 FF`), PNG (`89 50 4E 47`), GIF (`47 49 46 38`), WebP (`52 49 46 46...57 45 42 50`)
  - Base64-encode
  - Return `ImageData { media_type, data }`

- [x] Add security validation for image paths:
  - `std::fs::canonicalize()` to resolve symlinks
  - Reject paths outside allowed directories (`/tmp`, skill directory, Mika home dir)
  - Reject non-regular files (devices, sockets, etc.)

- [x] Update `execute_exec()`:
  - After capturing stdout, call `try_parse_envelope()`
  - If envelope: extract text, process image paths → `ToolOutput::success_with_images(text, images)`
  - If not envelope: existing behavior (`ToolOutput::success(stdout)`)
  - `truncate_output()` applies only to the text portion; images have their own 5MB-per-file limit
  - Maximum 5 images per tool result

- [x] If an image file path is invalid/missing, include error text in the text portion but don't fail the entire tool call. The text result is still valuable.

**Files:** `crates/mika-agent/src/skills/executor.rs`

**Estimated tests:** ~8 new tests (envelope parsing, image validation, security path checks, backward compat)

#### Phase 4: File-Reader Skill Update (`templates/skills/file-reader/`)

**Goal:** Make the file-reader skill image-aware — detect image files and use the envelope protocol.

**Changes to `templates/skills/file-reader/handlers/read.sh`:**

- [x] Before `cat`, check if the file is an image using `file --mime-type`:
  ```sh
  MIME=$(file -b --mime-type "$PATH_VALUE")
  case "$MIME" in
    image/jpeg|image/png|image/gif|image/webp)
      # Output envelope with image path
      printf '{"__mika_v1":{"text":"Image file: %s (%s)","images":["%s"]}}' \
        "$PATH_VALUE" "$MIME" "$PATH_VALUE"
      ;;
    *)
      # Existing behavior: cat the file
      cat "$PATH_VALUE"
      ;;
  esac
  ```

- [x] Update `templates/skills/file-reader/system_prompt.md` to mention image support:
  - "When asked to read an image file (JPEG, PNG, GIF, WebP), the tool returns the image for visual analysis."
  - "For binary non-image files, warn the user that the content cannot be displayed."

**Files:** `templates/skills/file-reader/handlers/read.sh`, `templates/skills/file-reader/system_prompt.md`

**Estimated tests:** Manual testing (shell script)

#### Phase 5: System Prompt and Agent Awareness (`crates/mika-agent/src/prompt.rs`)

**Goal:** Tell the agent it can view images produced by tools.

**Changes to `crates/mika-agent/src/prompt.rs` (in `## Tool Usage` section, ~line 213):**

- [x] Add guidance bullet:
  ```
  - Some tools can return images alongside text. When a tool produces an image (e.g., a screenshot or reading an image file), you will see the image and can describe its contents to the user.
  ```

- [x] Add file-reader awareness:
  ```
  - The read_file tool can read image files (JPEG, PNG, GIF, WebP) — you will see the image contents, not raw binary.
  ```

**Files:** `crates/mika-agent/src/prompt.rs`

**Estimated tests:** Update existing prompt content tests

## Acceptance Criteria

### Functional Requirements

- [x] `ToolOutput` supports returning images alongside text via `images: Vec<ImageData>`
- [x] `ContentBlock::ToolResult` serializes as multi-block array when images are present, string shorthand when text-only
- [x] Exec handler scripts can return images via the `__mika_v1` envelope protocol
- [x] File-reader skill detects image files and returns them for visual analysis
- [x] Agent loop converts `ToolOutput` images into proper Claude API image blocks in tool results
- [x] Existing tools (all ~20 builtins + all exec/http handlers) continue to work without changes
- [x] System prompt tells the agent it can view images from tools

### Non-Functional Requirements

- [x] Per-image size limit: 5MB raw file (pre-base64)
- [x] Maximum 5 images per tool result
- [x] Security: image file paths validated (canonicalized, restricted directories, no symlink escape)
- [x] Magic-byte validation for image types (JPEG/PNG/GIF/WebP only)
- [x] Text portion of tool output still truncated at 10,000 chars
- [x] No regressions: `cargo test` passes, `cargo clippy` clean

### Quality Gates

- [x] Serde round-trip tests for `ToolResultBody` (string ↔ array forms)
- [x] Unit tests for envelope parsing (valid, invalid, backward compat)
- [x] Unit tests for image path validation (security)
- [x] Integration test: mock exec handler with image envelope → verify correct API request construction
- [x] All existing tests pass without modification

## Dependencies & Prerequisites

- None — all changes are internal to Mika's codebase
- Claude API already supports multi-block tool_result content (no API changes needed)

## Risk Analysis & Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| Serde `#[serde(untagged)]` deserialization order bugs | API calls fail | Comprehensive round-trip tests, test against known Claude API JSON |
| False positive envelope detection (script output happens to be JSON with `__mika_v1` key) | Misinterpreted output | Sentinel key is unique enough; document the protocol clearly |
| Path traversal via image file references | Data exfiltration | Canonicalize + directory allowlist + symlink resolution |
| Large images causing memory pressure | OOM or slow API calls | 5MB per-image limit checked via metadata before reading |
| Breaking existing `ToolResult { content: String }` pattern matches | Compilation errors | Phase 2 explicitly handles all callsite updates |

## Explicitly Out of Scope

- **HTTP handler image support** — defer to follow-on iteration
- **Silent mode (heartbeat) image support** — exec handlers already filtered by `safe_always_on_skills()`
- **Image persistence in SQLite** — images are ephemeral (within-turn only); agent's text description persists
- **Making specific custom skills (hyprland, screenshot) into builtins** — this is about generic infrastructure
- **Token budget tracking for vision** — not needed for MVP; Claude API handles its own limits
- **Image forwarding via `send_message` to Telegram** — separate feature

## Future Considerations

- HTTP handler `Content-Type: image/*` detection for APIs that return images directly
- `send_message` tool could relay images to Telegram (requires gateway changes)
- Image description persistence in conversation metadata for cross-turn recall
- Builtin tools (Rust-native) returning images (e.g., a chart/visualization tool)
- Configuration flag to disable tool-result image support for cost control

## References

### Internal References

- `ToolOutput` struct: `crates/mika-agent/src/tools/mod.rs:64-68`
- `ContentBlock::ToolResult`: `crates/mika-common/src/claude.rs:98-104`
- `process_tool_calls()`: `crates/mika-agent/src/agent.rs:615-663`
- `execute_exec()`: `crates/mika-agent/src/skills/executor.rs:72-121`
- `build_system_prompt()`: `crates/mika-agent/src/prompt.rs:157-241`
- File-reader skill: `templates/skills/file-reader/`
- Telegram image pipeline: `docs/solutions/feature-implementation/telegram-image-support.md`
- Tool introspection: `docs/solutions/logic-errors/tool-call-introspection-cross-turn-persistence.md`

### External References

- Claude Messages API tool_result content: supports string or `[{type: "text"}, {type: "image"}]` array
- Supported image types: JPEG, PNG, GIF, WebP (Claude vision)
