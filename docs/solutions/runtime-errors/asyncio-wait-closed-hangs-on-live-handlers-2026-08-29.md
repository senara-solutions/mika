---
module: scripts/mika-pilot-egress-proxy
tags: [asyncio, graceful-shutdown, sigterm, unix-socket, wait-closed, cpython-3.12, egress]
problem_type: bug
category: runtime-errors
date: 2026-08-29
---

# `async with server` turns a graceful shutdown into an unbounded hang

## Problem

Adding a `SIGTERM` handler to the egress proxy so it would unlink its socket on the way out made the proxy **stop terminating on `SIGTERM` at all** whenever a client was connected. The shape looked unremarkable:

```python
    try:
        async with server:      # <-- the defect
            await stop
    finally:
        ...
```

Measured on this host (Python 3.14.6), with one established CONNECT tunnel to an allowlisted host: `SIGTERM` left the proxy running past 15s, and repeated `SIGTERM`s were swallowed by the stop future's own `if not stop.done()` guard. Before the change, `SIGTERM` had default disposition and killed the process instantly.

This is worse than the bug it was fixing. An operator whose `kill` appears to do nothing escalates to `kill -9` — and `kill -9` is exactly how the orphaned socket that motivated the whole change gets made. The remedy reproduced the disease.

## Root cause

Two independent mistakes, both invisible from reading the diff alone.

**1. `Server.__aexit__` awaits `wait_closed()`, which since CPython 3.12.1 waits for every live client handler.** It is no longer "the listening sockets are closed"; it is "close() was called *and* every connection handler has finished". A CONNECT tunnel in this proxy has no lifetime bound — `pipe_bytes` runs until one side hangs up — and a claude-pilot session holds one open for its entire duration. So the shutdown waited on the thing it was shutting down.

Even with no tunnel, an idle connected client holds a handler for the 10s header-read timeout, which is what makes this reproducible in CI without network.

**2. The hand-rolled `finally` unlink was not a backstop but a strictly worse duplicate.** It guarded with `sock_path.is_socket()`, which is equally true of a *different* proxy's live socket. asyncio's own unix cleanup already runs inside `close()` and guards correctly, by inode:

```python
# CPython asyncio/unix_events.py, _UnixSelectorEventLoop._stop_serving
if os.stat(path).st_ino == prev_ino:
    os.unlink(path)
```

Combined with defect 1 the two compose into a concrete failure: proxy A wedges in `wait_closed`, the launcher starts proxy B, B binds a fresh socket at the same path, A's tunnel eventually closes, and A's `finally` deletes B's live socket — leaving a running proxy with no path and a launcher that spawns another one on every dispatch.

## Solution

`close()` alone. No context manager, no `wait_closed()`, no manual unlink:

```python
    try:
        await stop
    finally:
        server.close()
```

`close()` runs asyncio's inode-guarded unix cleanup synchronously, so the socket is unlinked and a successor's is never touched. Returning from the coroutine lets `asyncio.run()` cancel the remaining handler tasks. Measured after the fix: the same established-tunnel scenario terminates in 1s with no residual socket, and the suite's own runtime dropped from 1.6s to 0.8s.

## Prevention

- **`async with server:` is not free.** On any server whose handlers are long-lived — tunnels, streams, websockets, SSE — it converts shutdown into "wait for every client to leave". Use it only where handlers are short and bounded.
- **Test shutdown with a connection open.** A shutdown test against an idle server exercises the one path that was never in question. The regression test here connects a client and asserts termination within 4s; it fails against the defective shape and passes after.
- **Before hand-rolling cleanup next to a library's own, read the library's.** The manual unlink was written from the assumption that asyncio might not cover the case. Reading `_stop_serving` would have shown it covers the case *better*, with a check the hand-rolled version could not express.
- Both defects were found by code review, not by the tests written alongside the change — the tests asserted the socket was gone, which was true, while the process stayed alive. **"The artifact is in the expected state" is not "the operation completed."**
