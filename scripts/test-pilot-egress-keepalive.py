#!/usr/bin/env python3
"""Unit test for the mika#2313 keep-alive fix in mika-pilot-egress-proxy.

The relay forwarded the client's `Connection: keep-alive` upstream, upstream held
the TLS open, and the relay (one req/connection) hung ~358 s per turn waiting for
an upstream EOF that only came at the idle-timeout — blowing the 5-min prompt-cache
TTL (the real cache_read=0). The fix: `_CLIENT_HEADER_DROP_RE` also strips the
hop-by-hop connection headers so ONLY the relay's injected `Connection: close`
reaches upstream. This asserts the guard, without a live socket.
"""
import os
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
PROXY = os.path.join(HERE, "mika-pilot-egress-proxy")

mod = SourceFileLoader("egress_proxy", PROXY).load_module()
DROP = mod._CLIENT_HEADER_DROP_RE

# Headers that MUST be dropped (never forwarded upstream) — mika#2313.
MUST_DROP = [
    b"Connection: keep-alive",
    b"connection: Keep-Alive",
    b"Keep-Alive: timeout=5, max=1000",
    b"Proxy-Connection: keep-alive",
    b"Host: api.anthropic.com",
    b"Authorization: Bearer sk-xxx",
    b"X-Api-Key: sk-xxx",
    b"Proxy-Authorization: Basic xxx",
]
# Headers that MUST be preserved (forwarded verbatim).
MUST_KEEP = [
    b"Content-Type: application/json",
    b"Content-Length: 1234",
    b"anthropic-version: 2023-06-01",
    b"anthropic-beta: prompt-caching-2024-07-31",
    b"Accept: application/json",
    b"User-Agent: undici",
    b"Transfer-Encoding: chunked",
]

fail = []
for h in MUST_DROP:
    if not DROP.match(h):
        fail.append(f"NOT dropped (leaks upstream): {h!r}")
for h in MUST_KEEP:
    if DROP.match(h):
        fail.append(f"wrongly dropped (breaks request): {h!r}")

if fail:
    print("FAIL:")
    for f in fail:
        print("  -", f)
    raise SystemExit(1)
print(f"ok: {len(MUST_DROP)} hop-by-hop/rewritten headers dropped "
      f"(incl. Connection/Keep-Alive/Proxy-Connection), {len(MUST_KEEP)} preserved")
