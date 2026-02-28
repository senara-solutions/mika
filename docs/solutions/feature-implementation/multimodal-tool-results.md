---
title: "Multimodal Tool Results: Images in tool_result Content Blocks"
date: 2026-02-28
category: feature-implementation
tags:
  - claude-api
  - tool-system
  - image-processing
  - file-io
  - security
  - serde
components:
  - mika-common/src/claude.rs
  - mika-agent/src/agent.rs
  - mika-agent/src/tools/mod.rs
  - mika-agent/src/skills/executor.rs
  - mika-agent/src/prompt.rs
  - templates/skills/file-reader/
severity: high
symptoms: "Agent says 'I can't read the screenshot file from disk directly' despite having screenshot, file-reader, and Claude vision capabilities"
root_cause: "ToolOutput was text-only; ContentBlock::ToolResult accepted only strings; no bridge existed between filesystem artifacts and multimodal Claude API payloads"
---

# Multimodal Tool Results: Images in tool_result Content Blocks

## Problem

Mika had all the pieces to analyze images from its own tools:
- Shell command execution (screenshot skills)
- File reading (file-reader skill)
- Claude's vision capabilities

But these were disconnected. When Mika took a screenshot and tried to read the file, it responded:

> I can't read the screenshot file from disk directly -- I can only analyze images you send me. But hyprctl gave me a clear picture of what's on workspace 1...

**Root cause:** Three layers of text-only assumptions:

1. `ToolOutput` was `{ content: String, is_error: bool }` -- no image field
2. `ContentBlock::ToolResult` had `content: String` -- only string shorthand
3. No protocol existed for exec handlers to signal "this output includes images"

The Claude Messages API already supports multi-block `tool_result` content (`[{type: "text"}, {type: "image"}]` arrays), but Mika only implemented the string shorthand form.

## Solution

A 5-phase generic infrastructure change enabling **any tool** to return images alongside text, plus 6 review fixes.

### Phase 1: Claude API Types (mika-common/src/claude.rs)

Extended the type system to match the Claude API spec:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultBody {
    Text(String),
    Blocks(Vec<ToolResultBlock>),
}
```

Changed `ContentBlock::ToolResult.content` from `String` to `ToolResultBody`. The `#[serde(untagged)]` enum serializes `Text` as a plain string (backward compatible) and `Blocks` as an array.

### Phase 2: ToolOutput + Agent Loop (mika-agent/src/tools/mod.rs, agent.rs)

Extended `ToolOutput` to carry images:

```rust
#[derive(Debug, Clone)]
pub struct ImageData {
    pub media_type: String,  // "image/png", "image/jpeg", etc.
    pub data: String,        // base64-encoded
}

pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    pub images: Vec<ImageData>,  // new field, defaults to vec![]
}
```

Updated `process_tool_calls()` to build multi-block content when images are present:

```rust
let content = if output.images.is_empty() {
    ToolResultBody::Text(output.content)
} else {
    let mut blocks = vec![ToolResultBlock::Text { text: output.content }];
    for img in output.images {
        blocks.push(ToolResultBlock::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: img.media_type,
                data: img.data,
            },
        });
    }
    ToolResultBody::Blocks(blocks)
};
```

### Phase 3: Exec Handler Envelope Protocol (mika-agent/src/skills/executor.rs)

Introduced the `__mika_v1` JSON envelope for exec handlers to return images:

```json
{"__mika_v1": {"text": "Screenshot taken.", "images": ["/tmp/screenshot.png"]}}
```

Detection uses a fast prefix check before JSON parse:

```rust
fn try_parse_envelope(stdout: &str) -> Option<MikaOutput> {
    let trimmed = stdout.trim();
    if !trimmed.starts_with(r#"{"__mika_v1""#) {
        return None;
    }
    serde_json::from_str::<MikaEnvelope>(trimmed)
        .ok()
        .map(|e| e.__mika_v1)
}
```

Image validation runs in `spawn_blocking` to avoid blocking the tokio runtime:

```rust
async fn read_and_validate_image(path: &str) -> Result<ImageData, String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let canonical = fs::canonicalize(&path)?;   // resolve symlinks
        let metadata = fs::metadata(&canonical)?;
        // Regular file check, 5MB size limit
        let bytes = fs::read(&canonical)?;
        let media_type = detect_image_type(&bytes)?; // magic-byte validation
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        drop(bytes); // free raw bytes, keep only base64
        Ok(ImageData { media_type, data })
    }).await
}
```

Security: path canonicalization, regular file check, 5MB limit, magic-byte validation (JPEG/PNG/GIF/WebP), max 5 images per result.

### Phase 4: File Reader Skill (templates/skills/file-reader/)

Updated `read.sh` to detect image files and emit the envelope using `jq` for safe JSON:

```bash
MIME=$(file -b --mime-type "$PATH_VALUE" 2>/dev/null)
case "$MIME" in
    image/jpeg|image/png|image/gif|image/webp)
        jq -n --arg path "$PATH_VALUE" --arg mime "$MIME" \
            '{"__mika_v1":{"text":"Image file: \($path) (\($mime))","images":[$path]}}'
        ;;
    *)
        cat "$PATH_VALUE"
        ;;
esac
```

### Phase 5: System Prompt (mika-agent/src/prompt.rs)

Added agent guidance:

```
- Tools may return images (screenshots, image files); you will see and can describe their contents.
```

### Review Fixes

Six issues found during code review and resolved:

| Fix | Problem | Solution |
|-----|---------|----------|
| `spawn_blocking` | Sync file I/O blocking tokio runtime | Wrap `read_and_validate_image` in `spawn_blocking` |
| `strip_prior_images()` | Base64 data re-sent every turn | Strip image blocks from prior turns before API call |
| `jq` JSON construction | Incomplete sed escaping in read.sh | Use `jq -n --arg` for safe JSON |
| Prefix check | JSON parse on every exec output | Check `starts_with(r#"{"__mika_v1""#)` first |
| Prompt shortening | Verbose image guidance | Condensed to single line |
| `drop(bytes)` | Raw bytes + base64 coexist in memory | Explicit drop after encoding |

## Key Design Decisions

1. **`#[serde(untagged)]` for ToolResultBody** -- Enables backward-compatible serialization (plain string when text-only, array when images present). Risk: deserialization order matters; mitigated with comprehensive round-trip tests.

2. **`__mika_v1` sentinel key** -- Unique enough to avoid false positives. Prefix check avoids JSON parsing overhead for 99%+ of exec outputs.

3. **Image stripping across turns** -- Prior-turn images replaced with `[image(s) from previous turn omitted]` text before each API call. Claude already described the image in its response, so the data is not needed on subsequent turns.

4. **Generic infrastructure, not skill-specific** -- Any tool (builtin Rust, exec handler, future HTTP handler) can return images. No special-casing of screenshot or file-reader skills.

## Prevention & Best Practices

### Avoiding Similar Capability Gaps

- Audit the Claude API spec when adding new tool types -- check what content blocks are supported
- When the API supports a modality Mika doesn't, file a tracking issue
- The `ToolOutput` struct is the extension point -- add new fields with `Vec` defaults for backward compatibility

### Security Checklist for File-Based Tool Output

- [ ] Path canonicalization (`std::fs::canonicalize`) to resolve symlinks
- [ ] Regular file check (`metadata.is_file()`)
- [ ] Size limit checked via metadata before reading
- [ ] Magic-byte validation for expected file types
- [ ] Maximum count per result (prevent resource exhaustion)
- [ ] Blocking I/O in `spawn_blocking` (not on async runtime)

### Testing Guidelines

- Serde round-trip tests for any new `#[serde(untagged)]` enum
- Magic-byte tests for each supported format
- Path security tests (symlinks, traversal, non-regular files)
- Integration test: mock exec handler with envelope -> verify API request construction
- Backward compatibility: existing tools produce same output as before

## Related Documentation

- [Implementation plan](../../plans/2026-02-28-feat-multimodal-tool-results-plan.md) -- Full 5-phase plan with acceptance criteria
- [Telegram image support](telegram-image-support.md) -- Upstream image pipeline (gateway -> agent server)
- [Tool call introspection](../logic-errors/tool-call-introspection-cross-turn-persistence.md) -- `ToolCallSummary` persistence, updated with `[+N image(s)]` suffix
- [TUI image paste](tui-slash-commands-web-search-image-paste.md) -- Existing image handling patterns (magic bytes, base64)
