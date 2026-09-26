---
module: skills/bundled/_shared
tags: [guards, liveness-probe, unix-socket, egress, anti-vacuity, test-design, dispatch-lib]
problem_type: process
category: best-practices
date: 2026-08-29
---

# A guard must observe, not assert — and its test must be seen failing

## Problem

`dispatch-lib.sh` decided whether the pilot's egress proxy was alive with `[ -S "$sock" ]`. That predicate answers "is there a file of type socket here". The question it was standing in for is "is anyone listening".

Those come apart in exactly one situation, and it is the situation that matters. A `kill` does not unlink a unix socket: the path outlives its owner as an orphan that satisfies `[ -S ]` and refuses `connect()`. On 2026-08-29 (mika#2041) that meant the launcher's wait loop exited on its first iteration, the `fs-only` fallback became unreachable in precisely the scenario it exists for, and `dispatch-lib` reported a successful launch over a proxy that had already died. The pilot went out with no egress and nothing in 1569 log lines said so.

The correct probe was already in the same file, ten lines above, in the liveness check: a real `socket.connect()`. **Two divergent definitions of "alive" in one function is the defect** — not the weaker of the two on its own.

## Root cause

A guard that asserts a proxy for the property instead of observing the property. The proxy and the property agree in the common case and diverge exactly in the failure case, so the guard is silently correct until the moment it is load-bearing.

The failure is absorbing and silent in combination: the only signal designed to reveal it (`fs-only`) was gated behind the same broken predicate. A check and its own alarm sharing a defect means the alarm can never ring.

## Solution

Factor **one** probe and use it at every site that asks the question:

```sh
# True when something is actually listening on the unix socket at $1.
_pilot_egress_sock_connectable() {
    [ -S "$1" ] || return 1
    python3 -c '...s.connect(sys.argv[1])...' "$1" "${2:-1}" 2>/dev/null
}
```

Both the liveness probe and the wait loop call it. Shipped in `skills/bundled/_shared/dispatch-lib.sh` on the mika#2041 branch (PR pending at time of writing).

Three secondary rules fell out of the same work, each of which was a live trap in this session rather than a hypothetical:

**1. An assertion you have not seen fail proves nothing.** The fallback test was run against the original `[ -S ]` guard and observed failing with the incident's exact shape (`rc=0 launched=yes msg=launched-ok`). A structural test — grepping the source for the new predicate — would have passed over dead code, which is the same class of mistake as the bug being fixed. Two further vacuity holes surfaced in the same session and were only visible because they were checked for: a *silent* fake binary made a log-pollution assertion incapable of ever failing, and a sandbox test that never asserted its subprocess started was satisfied by a subprocess that died at launch.

**2. Never assert on the size of a shared, live-written file.** Comparing `/var/log/mika/pilot-egress-proxy.log` before and after the suite failed whenever the real proxy wrote unrelated traffic during the run — flaky by construction, and only on machines where the thing under test is actually running. Assert on the fixture's own string instead. Guard readability with `-r`, not `-f`: a log left by a root-run deploy is present but unreadable, and `grep -c` then exits 2 printing nothing, so the comparison fails for a reason unrelated to the test.

**3. A test must not write into the evidence surface.** The launcher hardcoded `/var/log/mika`, and the suite calls the real launcher — so it would have appended fake-proxy output into the operational log, the same file every diagnosis of this incident was read from. The log directory is now overridable (`MIKA_PILOT_EGRESS_LOG_DIR`), default unchanged, and an assertion checks the fixture's line never reaches the real log.

## Prevention

When writing or reviewing a guard, name the property in a sentence and then read the predicate back as a question. If the two sentences are not the same sentence, the guard asserts rather than observes. `[ -S ]`, `[ -f ]`, `pgrep`, "the PID file exists", "the container is listed" are all proxies; `connect()`, a health request, and a real read are observations.

Two corollaries that generalise past this file:

- **A process being present is not the same as a process being able to serve.** During remediation of this incident, the operator's own process-existence check caught the doomed ephemeral proxies and reported the substrate healthy — the identical mistake to the one in the code.
- **Carry the ticket's intent; route its diagnosis.** mika#2041's body asserted that `bind()` failed on the occupied path. Measurement contradicted it: the proxy already unlinks a stale path before binding (`scripts/mika-pilot-egress-proxy:742-750`), and `diff` against `~/.local/bin/` proved that code deployed during the incident. The fix's intent held; the causal story did not. Why the proxies died before `bind()` is still unknown and was recorded as unknown rather than filled in with something plausible — the guard fix is what makes the next occurrence leave a trace instead of a silence.
