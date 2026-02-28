---
status: complete
priority: p3
issue_id: "333"
tags: [code-review, performance]
dependencies: []
---

# Redundant .to_vec() Copy in File Download

## Problem Statement

`download_file_bytes` calls `resp.bytes().await?.to_vec()` which copies the entire response body. If `DownloadedImage` stored `Bytes` instead of `Vec<u8>`, the copy is avoided entirely, saving 5MB transient allocation per image.

## Findings

- Flagged by: performance-oracle
- Location: `crates/mika-gateway/src/telegram.rs:372-373`

## Proposed Solutions

### Option A: Return Bytes directly or use DownloadedImage with bytes::Bytes
- **Effort:** Small

## Acceptance Criteria

- [ ] No unnecessary copy of downloaded bytes
