---
title: "feat: Support images sent via Telegram"
type: feat
status: completed
date: 2026-02-28
---

# feat: Support images sent via Telegram

## Overview

Images work in the TUI CLI via `/attach` and Ctrl+V clipboard paste but are completely dropped when sent through Telegram. The gateway replies "I can only read text messages right now" and discards the photo. This plan adds end-to-end image support through the Telegram -> Gateway -> Agent Server -> Claude API pipeline.

## Problem Statement

The image pipeline has **6 distinct drop points** between Telegram and the agent:

1. `TelegramMessage` struct only has `text` field — `photo`/`caption` silently dropped by serde
2. `parse_update()` classifies non-text messages as `Unsupported`
3. `Unsupported` handler sends rejection message back to user
4. `MessageRequest` on agent server has no `images` field
5. `handle_message` hardcodes `user_images: &[]`
6. Gateway forward JSON only includes `text`

The core agent logic (`agent.rs:527-544`) and Claude API types (`claude.rs`) already fully support images — the gap is entirely in the gateway and server layers.

## Proposed Solution

Extend the gateway to parse Telegram photo messages, download the image via Telegram Bot API (`getFile` + file download), base64-encode it, and forward it alongside the text to the agent container. Extend the agent server to accept and pass images through to the agent loop.

## Technical Approach

### Phase 1: Gateway — Telegram Types & Parsing

**Files:** `crates/mika-gateway/src/telegram.rs`

1. Add `PhotoSize` struct:
   ```rust
   #[derive(Debug, Deserialize)]
   pub struct PhotoSize {
       pub file_id: String,
       pub file_unique_id: String,
       pub width: u32,
       pub height: u32,
       #[serde(default)]
       pub file_size: Option<u64>,
   }
   ```

2. Add `TelegramDocument` struct:
   ```rust
   #[derive(Debug, Deserialize)]
   pub struct TelegramDocument {
       pub file_id: String,
       pub file_unique_id: String,
       #[serde(default)]
       pub file_name: Option<String>,
       #[serde(default)]
       pub mime_type: Option<String>,
       #[serde(default)]
       pub file_size: Option<u64>,
   }
   ```

3. Extend `TelegramMessage`:
   ```rust
   pub struct TelegramMessage {
       pub chat: TelegramChat,
       pub text: Option<String>,
       #[serde(default)]
       pub photo: Option<Vec<PhotoSize>>,
       #[serde(default)]
       pub caption: Option<String>,
       #[serde(default)]
       pub document: Option<TelegramDocument>,
   }
   ```

4. Add `ParsedMessage::Photo` variant:
   ```rust
   Photo {
       chat_id: i64,
       file_id: String,
       caption: Option<String>,
       update_id: i64,
   },
   Document {
       chat_id: i64,
       file_id: String,
       mime_type: String,
       caption: Option<String>,
       update_id: i64,
   },
   ```

5. Update `parse_update()` to detect `photo` (pick last/largest PhotoSize) and image-type documents before falling through to text/unsupported.

### Phase 2: Gateway — Telegram File Download

**Files:** `crates/mika-gateway/src/telegram.rs`

1. Add `TelegramFile` response struct:
   ```rust
   #[derive(Debug, Deserialize)]
   struct TelegramFile {
       file_id: String,
       file_path: Option<String>,
   }
   ```

2. Add `TelegramClient::get_file(file_id)` — calls `https://api.telegram.org/bot<token>/getFile`

3. Add `TelegramClient::download_file(file_path)` — downloads from `https://api.telegram.org/file/bot<token>/<file_path>`, returns raw bytes

4. Enforce 5MB per-image size limit (check `Content-Length` header or byte count). Return user-friendly error for oversized images.

5. Add magic byte validation for downloaded content (match TUI's security posture in `input.rs:378-383`):
   - JPEG: `FF D8 FF`
   - PNG: `89 50 4E 47`
   - GIF: `47 49 46 38`
   - WebP: `52 49 46 46` ... `57 45 42 50`

6. Derive media type from magic bytes (not file extension) for robustness.

### Phase 3: Gateway — Webhook Photo Handler

**Files:** `crates/mika-gateway/src/routes.rs`

1. Add `handle_photo_message()` function (parallel to existing `handle_text_message()`):
   - Download image via `get_file` + `download_file`
   - Validate magic bytes and enforce size limit
   - Base64-encode the image
   - If no caption, use `"[Photo]"` as synthetic text
   - Claim dedup ID **after** successful download (prevents message loss on download failure)
   - Forward to agent container with `images` array in JSON payload

2. Add match arm in `handle_webhook()` for `ParsedMessage::Photo` and `ParsedMessage::Document`.

3. Update the forwarding JSON to include images:
   ```json
   {
     "text": "caption or [Photo]",
     "chat_id": 42,
     "channel": "telegram",
     "request_id": "uuid",
     "images": [{"media_type": "image/jpeg", "data": "base64..."}]
   }
   ```

4. Increase POST timeout from 2s to 10s when images are present (base64 payloads are large).

5. On download failure: reply "Sorry, I couldn't download your photo. Please try sending it again." — no dedup claim, no retry.

### Phase 4: Agent Server — Accept & Forward Images

**Files:** `crates/mika-agent/src/server/types.rs`, `crates/mika-agent/src/server/handlers.rs`, `crates/mika-agent/src/server/mod.rs`

1. Add `ImagePayload` struct to `types.rs`:
   ```rust
   #[derive(Debug, Deserialize)]
   pub struct ImagePayload {
       pub media_type: String,
       pub data: String,
   }
   ```

2. Add `images` field to `MessageRequest`:
   ```rust
   #[serde(default)]
   pub images: Option<Vec<ImagePayload>>,
   ```

3. Update `handle_message()` in `handlers.rs`:
   - Convert `req.images` to `Vec<ImageSource>` (reuse `mika-common::claude::ImageSource`)
   - Pass as `user_images` to `AgentParams` instead of `&[]`

4. Add `RequestBodyLimitLayer(10 * 1024 * 1024)` (10MB) to the `/message` route in `mod.rs` to prevent memory exhaustion from oversized payloads.

### Phase 5: Tests

1. **Gateway unit tests** (`telegram.rs`):
   - `parse_update` with photo message (single photo, multiple sizes)
   - `parse_update` with document-type image (image/jpeg mime)
   - `parse_update` with non-image document (application/pdf → Unsupported)
   - `parse_update` with photo + caption
   - `parse_update` with photo, no caption
   - `get_file` response parsing
   - Magic byte validation for each supported format

2. **Gateway route tests** (`routes.rs`):
   - Webhook with photo message triggers download flow
   - Download failure sends error reply to user
   - Oversized image sends size-limit error reply
   - Dedup not claimed on download failure

3. **Agent server tests** (`handlers.rs`):
   - `/message` with images deserializes correctly
   - `/message` without images still works (backward compatible)
   - Images converted to `ImageSource` and passed to agent
   - Body limit rejects oversized requests

## Acceptance Criteria

- [x] User sends a photo in Telegram → Mika processes it and responds about the image content
- [x] User sends a photo with caption → caption used as text, image attached
- [x] User sends a photo without caption → `[Photo]` synthetic text, image attached
- [x] User sends an image as document (original quality) → same as photo flow
- [x] User sends a non-image document (PDF, ZIP) → "I can only read text messages" (unchanged)
- [x] User sends a photo > 5MB → user-friendly size limit message
- [x] Telegram file download fails → user-friendly error, no message loss
- [x] Existing text messages continue to work unchanged (backward compatible)
- [x] Stickers remain unsupported (existing behavior)
- [x] All new code has unit tests

## Known Limitations (Phase 1)

- **Image context lost from history:** After the first turn, conversation history shows `[N image(s) attached]` but the actual image data is not preserved. Multi-turn image Q&A ("what color was the shirt in that photo?") won't work. Same as TUI behavior.
- **Photo albums processed individually:** Media groups (multi-photo sends) trigger separate agent calls. Only the first succeeds; others get 429 "agent busy." Users should send one photo at a time.
- **Stickers not supported:** Static WebP stickers could technically be processed but are excluded intentionally — they serve a communication function, not informational.
- **No image in agent responses:** The `/send` endpoint only supports text. Mika can describe what's in an image but cannot send images back.

## Dependencies & Risks

- **Telegram Bot API rate limits:** `getFile` shares rate limits with other API calls. Under heavy concurrent photo load, 429s from Telegram are possible. Mitigated by existing `TelegramApiError` retry handling.
- **Memory pressure:** A 5MB JPEG → ~6.7MB base64 in memory. Concurrent photo processing for multiple customers could spike memory. Mitigated by 5MB per-image limit and 10MB body limit on agent server.
- **POST timeout:** Transmitting ~7MB base64 JSON over cluster networking needs more than the current 2s timeout. Mitigated by increasing to 10s for image payloads.

## References

### Internal
- TUI image attachment: `crates/mika-cli/src/tui/attachment.rs`, `crates/mika-cli/src/tui/input.rs:378-383`
- Agent image handling: `crates/mika-agent/src/agent.rs:527-544`
- Claude API types: `crates/mika-common/src/claude.rs:111-121`
- Gateway webhook handler: `crates/mika-gateway/src/routes.rs:153-160`
- Gateway Telegram client: `crates/mika-gateway/src/telegram.rs:130-225`
- Agent server types: `crates/mika-agent/src/server/types.rs:5-14`
- Agent handler: `crates/mika-agent/src/server/handlers.rs:158`

### Learnings Applied
- `docs/solutions/feature-implementation/tui-slash-commands-web-search-image-paste.md` — magic byte validation pattern
- `docs/solutions/integration-issues/telegram-webhook-gateway-design.md` — dedup sequencing, body limits, async processing
- `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md` — channel awareness patterns
