---
title: "UTF-8 byte-slicing panic in KG resolver/extractor — 14 sites, production KG resolution broken"
date: 2026-04-23
category: runtime-errors
module: kg, mika-agent, mika-common
problem_type: runtime_error
component: tooling
symptoms:
  - "thread 'tokio-rt-worker' panicked at byte index N is not a char boundary"
  - "KG resolution silently stops — zero kg_resolutions_log writes after deploy"
  - "27 panic events in server.log, all inside multi-byte UTF-8 chars (em-dash, arrow, box-drawing)"
  - "tokio::spawn JoinHandle discarded — panics invisible to tracing"
root_cause: logic_error
resolution_type: code_fix
severity: critical
tags: [utf8, panic, byte-slice, kg, resolver, extractor, floor-char-boundary, tokio-spawn, ci-lint]
---

# UTF-8 byte-slicing panic in KG resolver/extractor — 14 sites, production KG resolution broken

## Problem

KG resolution was fully broken in production after the #757 deploy (schema v26). The entity resolver panicked at byte boundaries inside multi-byte UTF-8 characters. 27 panic events across all 11 agents. The panics were invisible to tracing because `tokio::spawn` JoinHandles were discarded — the spawned task was poisoned but the server continued running with no indication of failure.

## Symptoms

- `thread 'tokio-rt-worker' panicked at crates/mika-agent/src/kg/entity_resolver.rs:1194:27: byte index 2000 is not a char boundary; it is inside '—' (bytes 1998..2001)`
- Zero `kg_resolutions_log` writes for 5+ hours despite non-zero `pending` counts
- No resolver error/warn logs (panics bypassed the `match Err(e) => warn!()` handler)
- Server `/health` returned 200 — all dispatcher threads alive and idle

## What Didn't Work

- Checking `resolution_returned_err` warns — no warns because the panic happened before `resolve_pending()` could return an `Err`
- Checking LLM call logs — zero calls because the task panicked before making any
- The 2-second delay between extraction and resolution spawns was not the issue

## Solution

### 1. Shared `safe_truncate` helper

Added `mika_common::text::safe_truncate(s, max_bytes)` using `str::floor_char_boundary()` (stable since Rust 1.80, workspace MSRV 1.91). Returns `&str`, never panics, preserves byte-budget semantics.

```rust
// crates/mika-common/src/text.rs
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    let end = s.floor_char_boundary(s.len().min(max_bytes));
    &s[..end]
}
```

This is distinct from `db::truncate_chars()` which counts characters and appends "...". The 14 affected sites all use byte limits for log widths and prompt budgets — `safe_truncate` preserves those byte-level semantics.

### 2. Replaced 14 unsafe sites

All `&s[..s.len().min(N)]` and `&s[..N]` patterns on `&str` replaced with `safe_truncate(s, N)`:

- `entity_resolver.rs` — 3 sites (retry prompt, parse error context, disambiguation prompt)
- `subject_extractor.rs` — 2 sites (retry prompt, parse error context)
- `rewind.rs` — 1 site (session ID truncation)
- `skills/context.rs` — 1 site (GitHub API error body)
- `server/dashboard.rs` — 1 site (base64 content truncation)
- `tools/get_session_messages.rs` — 1 site (message preview)
- `tools/list_audit_events.rs` — 1 site (local `truncate()` helper was itself unsafe)
- `mika-common/embedding.rs` — 1 site (embedding API error body)
- `tests/eval/assertions.rs` — 1 site
- `tests/eval/test_task_not_found_retry.rs` — 2 sites

### 3. Spawn-site panic containment

Wrapped both KG `tokio::spawn` calls (extraction at server/mod.rs ~line 849, resolution at ~line 934) with an outer spawn that awaits the JoinHandle and logs panics:

```rust
let handle = tokio::spawn(async move { /* existing task logic */ });
tokio::spawn(async move {
    match handle.await {
        Ok(()) => {}
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            let msg = payload.downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic>".to_string());
            error!(panic_message = %msg, agent_id = %agent, event = "resolution_panicked", ...);
        }
        Err(e) if e.is_cancelled() => { debug!(...); }
        Err(e) => { warn!(...); }
    }
});
```

### 4. CI regression guard

Added `scripts/check-byte-slices.sh` wired as `byte-slice-lint` CI job. Greps for:
- Pattern A: `[..var.len().min(N)]` — always unsafe on `&str`
- Pattern B: `&known_str_var[..LITERAL_INT]` — unsafe on known string variable names

Lines with `// safe-byte-slice: <reason>` are excluded.

## Why This Works

`&str[..N]` in Rust indexes by byte offset. Multi-byte UTF-8 characters (2-4 bytes) mean byte N may fall inside a character, causing a panic. `str::floor_char_boundary(N)` returns the largest byte index ≤ N that is a valid char boundary — this preserves the byte-budget constraint while guaranteeing no panic.

The spawn containment doesn't prevent the panic (the `safe_truncate` fix does that), but it ensures that if any future panic occurs in a spawned KG task, it produces a visible tracing event instead of silently poisoning the task.

## Prevention

- **Use `mika_common::text::safe_truncate(s, N)` for byte-budget truncation.** Never use `&s[..N]` or `&s[..s.len().min(N)]` on `&str`.
- **Use `db::truncate_chars(s, N)` for char-count truncation** (when you want "first N characters + ...").
- **The CI `byte-slice-lint` job catches regressions** — new unsafe patterns fail the build.
- **Always await or wrap `tokio::spawn` JoinHandles** for background tasks that should not fail silently. The outer-spawn pattern is lightweight and doesn't block the caller.

## Related Issues

- GitHub issue: #764
- Prior solution (narrower scope): `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
- Trigger: #757 / PR #759 — schema v26 invalidated all `kg_extractions` markers, causing mass re-extraction that hit multi-byte chars in compound docs
- Related: `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — documents the KG spawn architecture and JoinHandle gap
