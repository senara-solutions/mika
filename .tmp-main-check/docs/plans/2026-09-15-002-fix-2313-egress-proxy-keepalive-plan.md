---
issue: 2313
type: fix
module: mika-pilot-egress-proxy
tags: [substrate, egress-proxy, keep-alive, prompt-cache, loop-breaker]
problem_type: loop-breaker
---

# fix(2313): egress proxy forwards client keep-alive → 360 s hang/turn → cache TTL blown

## Problem (P0 loop-breaker — the REAL cause)
Every sandboxed pilot turn hung ~300–360 s, and `cache_read=0` (100–250k re-created
each turn). The earlier hypothesis (missing `~/.claude.json`) was **refuted**: a
window-cleared proof-groom showed cache_read=0 across 6 turns WITH that fix active.

## Root cause (timed tee, samidarko, 2026-09-15 06:19–06:25)
`handle_anthropic_reverse_proxy` dropped only host/authorization/x-api-key/
proxy-authorization from client headers, so the client's `Connection: keep-alive`
was **forwarded upstream** alongside the relay's injected `Connection: close` (two
Connection headers). Upstream honoured keep-alive and held the TLS open. The relay
is one-request-per-connection: it looped on `up_reader.read()` until upstream's
~360 s idle-timeout EOF, never closing the client. Meanwhile the CLI (undici,
keep-alive) had already sent its 2nd request on the same connection — it sat unread
for ~358 s, then the client EOF'd, retried the identical request on a NEW connection
(answered in ~1.7 s). Proof: connection c11 REQ1 06:19:44 → resp 06:19:46 → REQ2
06:19:46.9 → client EOF 06:25:45 (358 s) → new conn c17 REQ3 (=REQ2) in 1.7 s. The
>5-min hang exceeded the prompt-cache 5-min TTL → every turn re-created the context.

## Fix (surgical)
`_CLIENT_HEADER_DROP_RE` (hoisted to module level, testable) also drops
`connection|keep-alive|proxy-connection`. Upstream then receives ONLY the relay's
`Connection: close` → closes right after the response → `_relay_response_with_status_tap`
hits EOF promptly → the relay returns, `handle_host_client` closes the client, and
the relayed response carries `Connection: close` so the client does not reuse the
connection. This delivers samidarko's point-2 intent (prompt client close + response
carries close) without an invasive response-head rewrite — the negative test + the
proof-groom confirm it; an explicit response rewrite is a follow-up only if a gap shows.

## Deployment
The proxy is a standalone script in `~/.local/bin` (NOT embedded like dispatch-lib).
`make install` ships it; the running relay must be restarted (operator, at an empty
slot; the diagnostic tap is removed then).

## Acceptance criteria
- AC1: `scripts/test-pilot-egress-keepalive.py` passes — `_CLIENT_HEADER_DROP_RE`
  drops Connection/Keep-Alive/Proxy-Connection (+ host/auth/x-api-key/proxy-auth),
  preserves Content-Type/Content-Length/anthropic-version/anthropic-beta/etc.
- AC2 (negative, live): a request with `Connection: keep-alive` followed by a 2nd
  request on the SAME connection gets an EOF/response promptly — never > 2 s silence.
- AC3 (positive proof-groom, window-cleared): a sandboxed pilot at ≥2 turns shows
  `cache_read > 0` at the 2nd turn in its own transcript AND turns < 60 s.
- AC4: upstream receives exactly one `Connection` header (`close`), never the
  client's keep-alive.

## Rollback
Revert this commit; the relay returns to forwarding client keep-alive (the hang).
Also consider reverting mika#2314 (the ~/.claude.json emitter): inert re: this bug.
