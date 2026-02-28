# Telegram Image Support

## Problem

The TUI supported image attachments via `/attach` and Ctrl+V paste, but sending a photo through Telegram resulted in it being silently dropped. The gateway had no image handling — photos hit the `Unsupported` message type handler and the user got "I can only read text messages."

## Root Cause

Six distinct drop points in the Telegram → Agent pipeline:

1. **Telegram types** — `TelegramMessage` struct lacked `photo`, `caption`, `document` fields
2. **Parse logic** — `parse_update()` had no `Photo`/`Document` variants in `ParsedMessage`
3. **Webhook dispatch** — No match arm for photo messages in the webhook handler
4. **Agent server types** — `MessageRequest` had no `images` field
5. **Agent handler** — Hardcoded `user_images: &[]` when spawning agent loop
6. **Forward JSON** — Gateway built text-only JSON payloads with no image data

## Solution

### Gateway (mika-gateway)

- Added `PhotoSize`, `TelegramDocument` structs with `#[serde(default)]` fields
- Extended `ParsedMessage` with `Photo` and `Document` variants
- Updated `parse_update()` to pick largest photo from the `photo` array, detect image documents by MIME prefix
- Added `TelegramClient::download_image()` pipeline: `get_file()` → `download_file_bytes()` → magic-byte validation → `DownloadedImage`
- Magic-byte detection for JPEG (`FF D8 FF`), PNG (`89 50 4E 47`), GIF (`47 49 46 38`), WebP (`52 49 46 46...57 45 42 50`)
- 5MB per-image limit with Content-Length pre-check before download
- `file_path` validation against traversal/URL manipulation chars
- `handle_photo_message()` with dedup-after-download sequencing
- Base64 encode and forward with `"images": [{"media_type": ..., "data": ...}]`

### Agent Server (mika-agent)

- Added `ImagePayload` struct and `images: Option<Vec<ImagePayload>>` on `MessageRequest` (backward-compatible via `#[serde(default)]`)
- `media_type` allowlist validation at the `/message` boundary
- Move semantics instead of clone for image data (saves ~6.7MB per image)
- 10MB `RequestBodyLimitLayer` on `/message` route
- Empty text allowed when images are present (parity with TUI)

### Key Design Decisions

1. **Dedup-after-download** — Photo handler claims dedup *after* successful download, not before. If download fails, dedup isn't claimed, so Telegram retry can succeed. Text handler dedup-before-forward is fine because there's no download step.

2. **Magic bytes over file extensions** — Matches TUI security posture. Telegram's `mime_type` field is user-controlled and untrustworthy for documents.

3. **Defense in depth on size** — Content-Length header pre-check (early rejection) + post-download byte count (fallback) + 10MB body limit on agent server.

4. **Extracted routing helpers** — `resolve_customer()`, `claim_dedup()`, `reset_dedup()`, `handle_forward_result()` prevent text/photo handler divergence.

## Files Changed

- `crates/mika-gateway/src/telegram.rs` — Types, parsing, download, magic-byte validation
- `crates/mika-gateway/src/routes.rs` — Photo handler, shared routing helpers
- `crates/mika-agent/src/server/handlers.rs` — Image conversion, validation
- `crates/mika-agent/src/server/types.rs` — `ImagePayload` struct
- `crates/mika-agent/src/server/mod.rs` — Body limit, tests

## Lessons Learned

- When adding a new message type to a multi-layer pipeline, audit every layer for hardcoded assumptions (struct fields, match arms, JSON payloads, test fixtures)
- Dedup timing matters: stateful operations (like downloads) should complete before claiming dedup to allow retries on failure
- `bytes::Bytes` avoids a redundant `to_vec()` copy when passing through reqwest response data
- `Option::take()` + `into_iter()` enables zero-copy moves from request structs that are consumed by the handler
