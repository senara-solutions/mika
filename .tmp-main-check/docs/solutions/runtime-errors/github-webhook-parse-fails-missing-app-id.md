---
title: "GitHub webhook parse fails on missing app_id field"
category: runtime-errors
date: 2026-04-03
tags: [gateway, webhook, serde, deserialization, github]
issue: "#403"
---

# GitHub webhook parse fails on missing app_id field

## Problem

Gateway logs showed `WARN GitHub webhook body parse failed error=missing field 'app_id'` for some GitHub App webhook event types. Events were lost (400 BAD_REQUEST returned).

## Root cause

`GitHubInstallation` struct had a required `app_id: u64` field, but not all GitHub webhook event types include `app_id` in the `installation` object. Some send only `{"id": N}`. The `app_id` field was dead code — its only consumer (the bot self-event filter) was removed in #401/#402.

## Solution

Removed the `app_id` field from `GitHubInstallation` entirely in `crates/mika-gateway/src/github.rs`:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct GitHubInstallation {
    pub id: u64,
    // app_id removed — dead code after #401/#402 self-event filter removal
}
```

Serde's default `#[derive(Deserialize)]` ignores unknown fields, so payloads that still include `app_id` parse correctly without any additional attributes.

## Prevention

- When adding fields to webhook deserialization structs, use `Option<T>` or `#[serde(default)]` unless the field is guaranteed present in ALL event types. GitHub's webhook schema varies by event type.
- Remove dead struct fields promptly when their consumers are deleted. Dead required fields on deserialization structs silently become parsing landmines.
- Add regression tests with realistic payloads when removing or adding fields to webhook structs.

## Related

- #401/#402 — removed bot self-event filter (made `app_id` dead code)
- `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md` — webhook endpoint architecture
