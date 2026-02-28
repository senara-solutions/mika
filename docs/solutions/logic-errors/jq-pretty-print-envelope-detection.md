---
title: "jq Pretty-Print Breaks __mika_v1 Envelope Detection"
problem_type: logic-error
severity: high
modules: [mika-agent, skills-executor, file-reader]
symptoms:
  - "Mika reports seeing __mika_v1 JSON envelope but cannot render images"
  - "read_file returns envelope text instead of being parsed as image protocol"
root_cause: "jq -n produces pretty-printed JSON by default; try_parse_envelope() prefix check expects compact single-line JSON"
resolution_type: bug-fix
tags: [jq, json-parsing, multimodal, image-protocol, defense-in-depth, toctou, memory-budget, performance]
date_resolved: "2026-02-28"
related_feature: multimodal-tool-results
---

# jq Pretty-Print Breaks `__mika_v1` Envelope Detection

## Problem Statement

After implementing the multimodal tool results infrastructure and making file-reader always-on, Mika still couldn't view images. The agent reported: "read_file returned the `__mika_v1` JSON envelope... But the client isn't rendering it to me as a visual yet."

The full pipeline was wired correctly. The bug was simpler: a JSON formatting assumption in protocol detection.

## Investigation

The multimodal feature pipeline has five stages, all implemented correctly:

1. **Exec handler scripts** output a JSON envelope: `{"__mika_v1": {"text": "...", "images": ["/path/to/file"]}}`
2. **Executor** (`executor.rs`) calls `try_parse_envelope()` to detect the sentinel key
3. **Image validator** (`read_and_validate_image()`) canonicalizes paths, checks magic bytes, reads file content
4. **ToolOutput** carries `images: Vec<ImageData>` back to the agent loop
5. **History builder** converts images to multi-block `tool_result` content arrays for the Claude API

The failure point was stage 2: `try_parse_envelope()` used a strict prefix check:

```rust
if !trimmed.starts_with(r#"{"__mika_v1""#) {
    return None;
}
```

But `read.sh` used `jq -n` (no `-c` flag), which produces pretty-printed JSON:

```json
{
  "__mika_v1": {
    "text": "Image file: /tmp/test.png (image/png)",
    "images": ["/tmp/test.png"]
  }
}
```

The prefix `{\n  "__mika_v1"` doesn't match `{"__mika_v1"`. The tool output was treated as plain text, passed through unchanged, and never entered the image extraction pipeline. The failure was silent.

## Root Cause

Format assumption in protocol detection. The Rust code assumed compact JSON output, but `jq -n` defaults to pretty-printed output. Neither side enforced or documented the format contract.

## Solution

### Fix 1: Compact JSON output (`read.sh`)

**File:** `templates/skills/file-reader/handlers/read.sh` line 24

```diff
-        jq -n --arg path "$PATH_VALUE" --arg mime "$MIME" \
+        jq -cn --arg path "$PATH_VALUE" --arg mime "$MIME" \
```

The `-c` flag ensures compact single-line JSON output.

### Fix 2: Relaxed prefix check (`executor.rs`) — defense-in-depth

**File:** `crates/mika-agent/src/skills/executor.rs` line 98

```diff
-    if !trimmed.starts_with(r#"{"__mika_v1""#) {
+    if !trimmed.starts_with('{') || !trimmed.contains(r#""__mika_v1""#) {
```

This handles both compact and pretty-printed JSON. The `starts_with('{')` check rejects 99%+ of exec outputs (which are plain text, not JSON objects), and `contains("__mika_v1")` handles any JSON whitespace formatting. Other skill authors might use `jq` without `-c`.

### Test added

```rust
#[test]
fn test_try_parse_envelope_pretty_printed() {
    let json = "{\n  \"__mika_v1\": {\n    \"text\": \"Image file: /tmp/shot.png (image/png)\",\n    \"images\": [\n      \"/tmp/shot.png\"\n    ]\n  }\n}";
    let env = try_parse_envelope(json).unwrap();
    assert_eq!(env.text, "Image file: /tmp/shot.png (image/png)");
    assert_eq!(env.images, vec!["/tmp/shot.png"]);
}
```

## Follow-Up Fixes from Code Review

The same review identified three P2 issues, all resolved:

### TOCTOU Race in `read_and_validate_image()`

**File:** `crates/mika-agent/src/skills/executor.rs`

**Before:** `fs::metadata()` checked size, then `fs::read()` read the file — a race window where the file could grow between check and read.

**After:** `File::open()` + `take(MAX_IMAGE_SIZE + 1)` + `read_to_end()` caps actual bytes read. Post-read size validation catches files that grew.

```rust
// Capped read prevents TOCTOU: even if file grew, we cap the read
let file = fs::File::open(&canonical)?;
let mut bytes = Vec::with_capacity(metadata.len() as usize);
file.take(MAX_IMAGE_SIZE + 1)
    .read_to_end(&mut bytes)?;

if bytes.len() as u64 > MAX_IMAGE_SIZE {
    return Err("image too large".into());
}
```

### `strip_prior_images` Inefficiency

**File:** `crates/mika-agent/src/agent.rs`

**Before:** Three iterations over blocks — (1) clone text strings into `Vec<String>`, (2) check `has_images`, (3) scan+replace user images. Clones happened even when no images were present.

**After:** Single `match` block handles both tool result images and user images in one pass. Uses `as_str()` borrows instead of `clone()`.

```rust
for block in blocks.iter_mut() {
    match block {
        ContentBlock::ToolResult { content, .. }
            if matches!(content, ToolResultBody::Blocks(_)) =>
        {
            if let ToolResultBody::Blocks(inner_blocks) = content {
                let mut combined: String = inner_blocks.iter()
                    .filter_map(|b| match b {
                        ToolResultBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                combined.push_str("\n[image(s) from previous turn omitted]");
                *content = ToolResultBody::Text(combined);
            }
        }
        ContentBlock::Image { .. } => {
            *block = ContentBlock::Text {
                text: "[user image from previous turn omitted]".to_string(),
            };
        }
        _ => {}
    }
}
```

### Aggregate Image Memory Budget

**File:** `crates/mika-agent/src/agent.rs`

**Before:** No limit on total image bytes within a single agent step. Worst case: 10 tools x 5 images x 6.67MB = ~333MB.

**After:** `MAX_IMAGE_BYTES_PER_STEP = 20MB` budget tracked in `process_tool_calls`. Images exceeding the budget are skipped with a text note and a warning is logged.

## Prevention & Best Practices

### Shell Script Output Conventions

- **Always use `jq -c`** (compact) for machine-readable output. Pretty-printing is for humans.
- Document the output format contract in script comments.
- `jq -cn` combines null input (`-n`) with compact output (`-c`).

### Protocol Detection Patterns

- **Never assume JSON formatting.** Always `trim()` before checking.
- Use two-step detection: (1) fast structural check (`starts_with('{')`), (2) semantic check (`contains(sentinel)`).
- Let `serde_json::from_str` handle whitespace; don't replicate JSON parsing with string ops.

### File I/O Safety

- Use **capped reads** (`file.take(limit)`) to close TOCTOU gaps between metadata checks and reads.
- Validate file properties (type, size) via metadata as a pre-check, but always verify post-read.
- Run blocking I/O in `tokio::task::spawn_blocking`.

### Memory Budgeting

- Define explicit budgets for unbounded data (per-item AND aggregate).
- Strip/remove accumulated data at the earliest boundary (before each API call, not after).
- Document memory lifecycle in comments.

### Testing

- Test format-sensitive code with both compact and pretty-printed JSON.
- Include pathological cases for memory accumulation (many images, max sizes).
- Add tests for the exact format that caused the bug.

## Related Documentation

- [`docs/solutions/feature-implementation/multimodal-tool-results.md`](../feature-implementation/multimodal-tool-results.md) — Primary implementation doc
- [`docs/solutions/security-issues/code-review-7aba1ec-shell-injection-memory-safety.md`](../security-issues/code-review-7aba1ec-shell-injection-memory-safety.md) — Shell script security patterns, jq migration
- [`docs/solutions/feature-implementation/telegram-image-support.md`](../feature-implementation/telegram-image-support.md) — Image pipeline from gateway
- [`docs/solutions/logic-errors/skill-availability-and-send-message-honesty.md`](skill-availability-and-send-message-honesty.md) — Always-on skill framework
- [`docs/solutions/logic-errors/tool-call-introspection-cross-turn-persistence.md`](tool-call-introspection-cross-turn-persistence.md) — Tool summary with `[+N image(s)]` annotation

## Files Changed

| File | Change |
|------|--------|
| `templates/skills/file-reader/handlers/read.sh` | `jq -n` -> `jq -cn` |
| `crates/mika-agent/src/skills/executor.rs` | Relaxed prefix check, TOCTOU fix with capped reads |
| `crates/mika-agent/src/agent.rs` | Single-pass `strip_prior_images`, image memory budget |
