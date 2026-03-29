---
status: complete
priority: p2
issue_id: 737
tags: [code-review, security]
---

# Shell handler env scrub: replace explicit unset list with wildcard

## Problem Statement

The `shell-exec/handlers/run.sh` script had a hardcoded list of env vars to `unset`, which was inherently fragile — every new `MIKA_*` secret required manually updating the list. The list was missing 3 provider API keys (MIKA_MINIMAX_API_KEY, MIKA_KIMI_API_KEY, MIKA_QWEN_API_KEY) and other secrets (MIKA_DASHBOARD_TOKEN, MIKA_OTLP_AUTH_HEADER).

## Resolution

Replaced the explicit `unset` list with a wildcard loop that mirrors the Rust executor's `scrub_mika_env_vars()` approach:

```sh
for _mika_var in $(env | grep '^MIKA_' | cut -d= -f1); do unset "$_mika_var"; done
```

This eliminates the maintenance burden and covers all current and future `MIKA_*` secrets automatically.

## Work Log

- 2026-03-29: Found during code review of #317. Fixed in the same PR.
