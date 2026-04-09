---
title: "GitHub webhook observability: pre-routing trace and unroutable-event warning"
category: architecture-patterns
date: 2026-04-09
tags: [mika-gateway, github, webhook, observability, logging, tracing]
module: mika-gateway
issue: 487
---

# GitHub Webhook Observability: Pre-Routing Trace and Unroutable-Event Warning

## Problem

mika-qa-bot posted a `pull_request_review` webhook with `state=COMMENTED` containing a `VERDICT: hold[review]` token. mika-dev never received the webhook. The autonomous dev loop broke silently at the "QA verdict → mika-dev retry" handoff because there was no way to distinguish three failure modes:

1. **Never arrived** — GitHub never delivered the webhook (App subscription issue, network/proxy)
2. **Arrived but dropped** — Gateway received it but `route_event()` returned `None` (routing bug)
3. **Arrived and delivered** — Working as intended

The gateway had no log entry for case 1 vs 2. The unroutable-event log was at `debug!` level (invisible in production) and lacked the `delivery_id` field for correlation.

## Root Cause

Two observability gaps in `crates/mika-gateway/src/github.rs`:

1. **No pre-routing trace**: The first log for a webhook fired only on the success path (routing hit). Webhooks that were deduped, parse-failed, or unroutable had no common entry point log.
2. **Silent drops**: Unroutable events logged at `debug!` — filtered out in production. A `pull_request_review.submitted` that somehow failed routing would vanish without a trace.

## Solution

### 1. Pre-routing debug! log (step 5 in the pipeline)

Added immediately after signature validation and span creation, **before** ping handling, dedup, body parsing, and routing:

```rust
// 5. Pre-routing trace — fires for EVERY valid webhook before dedup/routing/filtering.
// Diagnostic chain:
//   - Never arrived:        no debug! at all for that delivery_id
//   - Arrived but deduped:  this debug! + dedup debug!
//   - Arrived, not routed:  this debug! + unroutable warn!
//   - Arrived and delivered: this debug! + routing info!
debug!(
    event_type,
    delivery_id = %delivery_id,
    "GitHub webhook received (pre-dedup, pre-routing)"
);
```

This is the **first log after HMAC validation** — if a webhook passes signature validation, this debug! fires unconditionally. The `action` field is not available here (body parsing happens later), but `event_type` + `delivery_id` are sufficient for correlation.

### 2. Unroutable events promoted to warn!

Changed the no-route log from `debug!` to `warn!` and added `delivery_id`:

```rust
warn!(
    event_type,
    action = ?event.action,
    delivery_id = %delivery_id,
    "GitHub webhook event not routable, dropping"
);
```

This ensures that any dropped event is visible in production logs without changing the log level configuration.

### 3. QA Verdict Contract documentation

Added a "QA Verdict Contract" section to `docs/skills.md` documenting that:
- mika-qa-bot posts verdicts as `state=COMMENTED` with a `VERDICT:` token in the body
- The `state` field is NOT authoritative — the body token is
- Any code that gates on `state` instead of body content is a bug

## Key Design Decisions

- **debug! not info!**: The pre-routing trace is at `debug!` level because it fires for every single webhook. At `info!`, it would double the log volume for the success path (pre-routing debug! + routing info!). When investigating delivery issues, operators enable debug temporarily or search by `delivery_id`.
- **warn! for drops**: Unroutable events are correctness signals, not noise. If the GitHub App is subscribed to event types the gateway doesn't route, the operator should trim the subscription — not suppress the warning.
- **Placement before ping**: The debug! fires even for ping events, which is intentional. Ping events are rare and the log confirms the webhook path is alive end-to-end.

## Prevention

- When adding new webhook event types to `route_event()`, the pre-routing trace and unroutable warn! automatically cover the new type — no additional logging needed.
- The diagnostic chain comment in the code documents the four observable states. Preserve this comment when modifying the pipeline.
- The QA Verdict Contract in `docs/skills.md` establishes the BAD/GOOD pattern for verdict parsing. Any new code that processes qa-bot reviews should reference this contract.

## Related

- [GitHub webhook endpoint on mika-gateway](github-webhook-endpoint-gateway.md) — original architecture
- [Multi-tenant GitHub webhook agent mapping](multi-tenant-github-webhook-agent-mapping.md) — routing pipeline
- [Gateway request logging with TraceLayer](gateway-request-logging-tracelayer-health-filtering.md) — log level conventions
- Issue [#487](https://github.com/senara-solutions/mika/issues/487) — incident report
