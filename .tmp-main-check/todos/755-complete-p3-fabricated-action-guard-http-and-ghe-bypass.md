# Fabricated action guard bypass: HTTP and GitHub Enterprise URLs

**Priority:** P3
**File:** `crates/mika-agent/src/agent.rs`
**Issue:** #308

## Problem

The `GITHUB_RESOURCE_URL_RE` regex anchors on `https://github.com/` which means:

1. **HTTP URLs** (`http://github.com/...`) are not detected. While github.com redirects
   HTTP to HTTPS, an LLM fabricating URLs is not subject to HTTP semantics and could
   emit either scheme.

2. **GitHub Enterprise URLs** (`https://github.enterprise.com/...` or
   `https://github.acme.com/...`) are not detected. If the agent operates against
   a GHE instance, fabricated URLs would use the enterprise hostname.

## Impact

Low. LLMs overwhelmingly generate `https://github.com/` URLs when fabricating. The
HTTP variant is unlikely in practice because LLMs model the real-world convention.
The GHE gap only matters if the agent operates against enterprise GitHub instances,
which would require a configurable hostname pattern.

## Recommendation

For HTTP: optionally match `https?://` instead of `https://`. Minimal regex change.

For GHE: not actionable without a configuration mechanism. If GHE support becomes
relevant, the regex could be parameterized from config. Defer until needed.
