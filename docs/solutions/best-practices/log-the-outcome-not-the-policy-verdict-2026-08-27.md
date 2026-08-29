---
module: scripts, pilot-egress
date: 2026-08-27
problem_type: best_practice
category: best-practices
component: tooling
severity: high
applies_when: "A proxy, gateway, middleware, or guard logs a line after deciding to let a request through"
tags:
  - observability
  - logging
  - proxy
  - rate-limit
  - anthropic
  - diagnosability
  - claude-pilot
related_components:
  - scripts/mika-pilot-egress-proxy
  - scripts/mika-pilot-anthropic-auth-addon.py
---

# Log the outcome, not the policy verdict — and log it when you learn it

## Context

The pilot egress proxy sits between the sandboxed claude-pilot and
`api.anthropic.com`. For every forwarded request it printed one line:

```
[anthropic-proxy] ALLOW POST /v1/messages?beta=true
```

`ALLOW` was true and accurate. It meant *the egress allowlist permitted this
call* — a policy verdict about the request. It said nothing about what the
upstream did with it.

On 2026-08-06 a pilot session stalled for 606s and the guardrail killed it as
`idle_timeout: No meaningful progress for 300s`. The proxy log showed the line
above. Anthropic had in fact returned **HTTP 429**. Establishing that took most
of a day: a suspect component was cleared with a predicate test, an A/B
pre-install run reproduced the hang without it, an instrumented copy of the
proxy was written to add `STREAM_START`/`STREAM_END` markers, and finally a
host-side `curl` produced the actual verdict (`rate_limit_error`, request
`req_011CdmbTL5FH62zfwP7ieMhu`). The proxy had that fact in its hands the whole
time and printed a line that read like success.

Fixed in senara-solutions/mika#1901.

## Guidance

### 1. A line that names your decision is not a line that names the result

`ALLOW`, `permitted`, `forwarded`, `dispatched`, `accepted` all describe what
*you* chose. Under diagnosis, nobody is asking what you chose — they are asking
what happened. When the two are logged as one line, every upstream failure class
(401, 429, 5xx, no answer at all) collapses into the shape of success.

Keep the verdict, append the outcome:

```
[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 429
```

Existing greps for `ALLOW` keep working, and the class that matters is now
visible without one.

### 2. Give the failure class you care about its own greppable token

A status code buried in a line is findable only by someone who already suspects
it. The condition worth naming gets its own first token:

```
[anthropic-proxy] RATE_LIMITED POST /v1/messages?beta=true Retry-After=37 request-id=req_...
```

One `grep RATE_LIMITED` now ends the investigation that previously needed a
probe. The same repo learned this once already for Anthropic HTTP 400 billing
rejections, which were classified as a generic error and produced 1086
indistinguishable WARN lines over four hours — see
`docs/solutions/runtime-errors/anthropic-billing-400-classified-as-generic-httperror-2026-05-12.md`.

### 3. "Answered nothing" is a third outcome, not a kind of success

An upstream that closes without sending a status line is neither allowed-and-fine
nor an error status. It needs its own line (`UPSTREAM_NO_RESPONSE`), because
that is precisely the shape the ad-hoc probe had been written to detect —
`STREAM_START` with no `STREAM_END`.

### 4. Log at the moment you learn the outcome, not at the end of the stream

**This is where the first fix attempt reproduced the original blindness**, and
it is the least obvious of the four.

Reading the status off a streamed response and reporting it after the relay loop
finishes looks correct and passes every happy-path test. It fails in exactly the
incident shape:

1. upstream answers 429;
2. the client stalls waiting on the retry;
3. the guardrail kills it at 300s;
4. the client socket closes, the relay's `drain()` raises;
5. the exception escapes before the reporting line is ever reached.

Result: **nothing at all is logged** — worse than the misleading line it
replaced. Verified against the real handler over loopback before the fix: the
throttled-then-killed case emitted zero output.

Report as soon as the response head is complete, and put the fallback report in
a `finally` so exactly one line is emitted whatever happens to the body:

```python
tap = _ResponseHeadTap()
reported = False
try:
    while True:
        chunk = await up_reader.read(BUFFER_SIZE)
        if not chunk:
            break
        writer.write(chunk)          # client first — always
        await writer.drain()
        if reported:
            continue
        tap.feed(chunk)              # then a copy for us
        if tap.complete:
            _log_upstream_outcome(method, path, ...)
            reported = True
finally:
    if not reported:
        _log_upstream_outcome(method, path, ...)
```

A stalled stream is then diagnosed *while it is still stalling*, which is the
only time the information is worth anything.

### 5. Read the stream without owning it

Adding an observer to a relay must not change what the relayed bytes look like
or when they arrive. Two rules make that structural rather than careful:

- **Write first, tap second.** The accumulator only ever sees a copy of bytes
  already sent and drained. It cannot delay them because it runs after them.
- **Keep only the head, and cap it.** Stop accumulating at the head terminator
  so a long body costs nothing, and give up past a fixed cap so a malformed
  upstream cannot grow memory per connection.

Assert byte-identity directly in a test — a multi-chunk SSE response in, the
same bytes out — rather than trusting that the tap is passive.

### 6. Allowlist what you log; never filter it

An observer that logs "the interesting headers" by excluding known-sensitive
names will eventually print a secret, because the exclusion list is written
against today's upstream. Name what may be logged instead:

```python
_QUOTA_HEADER_NAMES = frozenset({"retry-after", "request-id"})
_QUOTA_HEADER_PREFIX = "anthropic-ratelimit-"
```

A header the selector does not name cannot reach a log line, whatever the
upstream starts returning. Test it with an `authorization` header in the
fixture, so the property is proven rather than promised.

### 7. Instrument every door, or you have instrumented none

Traffic reached Anthropic two ways: a reverse-proxy path
(`ANTHROPIC_BASE_URL`/`CLAUDE_CODE_API_BASE_URL` point at it) and a
CONNECT-tunnel path chained to a local mitmdump for the internal `claude` calls
that ignore those overrides. On the tunnel the proxy sees only ciphertext, so
the mitmproxy addon is the only place a status is readable there.

Instrumenting one path would have left the original silence fully intact for
anything that took the other — the failure reproduces, just through the other
half of the code. Enumerate the paths to the dependency before deciding the
instrumentation is done, and emit identical line shapes on each so one grep
covers all of them.

### 8. Prefer `responseheaders` to `response` in a mitmproxy addon

`responseheaders` fires before the body is read, so a read-only status
observation leaves the streaming behaviour untouched. A `response` hook forces
the body into memory. Make the test's fake response raise on `.content` and
`.text`, so a future edit that reaches for the body fails the suite instead of
silently changing streaming semantics in production.

## Applicability

The specific case is an HTTP proxy, but the shape is general: any component that
decides whether to let something proceed and then logs that decision. Guards,
middleware, dispatchers, permission classifiers, and queue admitters all have the
same failure mode — the log answers "did we allow it?" when the operator is
asking "did it work?".

The tell is a log line whose vocabulary comes from your policy rather than from
the thing you called.
