---
title: "UTF-8 byte-slicing panic in dashboard DTO truncation"
category: runtime-errors
date: 2026-03-16
severity: medium
module: mika-agent (server/dashboard)
tags: [utf8, panic, truncation, dashboard, dto, rust]
---

## Problem

The `truncate_preview` function in `server/dashboard.rs` used byte-based slicing (`&s[..max_len]`) to truncate `action_config` and `result` fields in `TaskResponse`. In Rust, indexing a `&str` by byte position panics if the index falls in the middle of a multi-byte UTF-8 character sequence. Any task with non-ASCII content (internationalized text, emoji, or JSON with escaped unicode) in `action_config` or `result` would cause a runtime panic on the `GET /api/v1/tasks` endpoint.

```rust
// BROKEN — panics on multi-byte UTF-8
fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])  // ← byte index, not char boundary
    }
}
```

## Root Cause

`s.len()` returns byte count, and `&s[..N]` indexes by byte offset. Multi-byte UTF-8 characters (2-4 bytes each) mean byte offset N may land inside a character, which Rust's string safety checks catch as a panic.

## Solution

Replaced with the existing `db::truncate_chars()` function which uses `s.chars().take(max_len).collect()` for character-boundary-safe truncation.

```rust
// FIXED — uses char-safe truncation from db.rs
let action_config_preview = if t.action_config.is_empty() {
    None
} else {
    Some(db::truncate_chars(&t.action_config, 200))
};
```

The `truncate_chars` function was already available as `pub(crate)` in `db.rs` (used by `load_team_runs_for_prompt`). No new code needed — just reuse.

## Prevention

- **Never use `&s[..N]` on user-facing strings in Rust.** Always use `s.chars().take(N)`, `s.char_indices()`, or `s.floor_char_boundary(N)` (stable since Rust 1.82).
- **Grep for byte-slicing patterns:** `&content[..` or `&s[..` in code that handles user-generated text.
- **The same pattern existed in `strip_base64_images`** (`&content[..1000]`) — lower risk since base64 content is ASCII, but worth fixing for consistency.
- **Reuse `truncate_chars` from `db.rs`** for any future truncation needs in the server layer.

## Related

- `db::truncate_chars()` at `crates/mika-agent/src/db.rs` — the safe version
- `strip_base64_images` at `server/dashboard.rs` — same byte-slicing pattern (base64 is ASCII, so lower risk)
