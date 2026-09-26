#!/bin/bash
# mika#2049 — a SERVING egress relay, fabricated for the containment suites.
#
# WHY THIS FILE EXISTS
#
# Four containment suites launch a real bwrap through the real
# `_run_pilot_sandboxed`, and until mika#2049 each of them stubbed
# `_ensure_pilot_egress_proxy` to `return 1` in order to exercise the Phase 2a
# fallback — filesystem cut only, network open. The fail-closed posture deletes
# that fallback: `return 1` now makes `_run_pilot_sandboxed` REFUSE, and a suite
# stubbing it that way has no launch left to inspect.
#
# Those four suites guard the gitdir binds (mika#2141), the GitHub credential's
# absence (mika#2056), the session-log bind (mika#2165) and the secret-free argv
# (mika#2039). Leaving them broken would weaken containment in the name of a
# containment fix, so the stub is migrated rather than removed — and it is
# migrated ONCE, here, because four hand-written copies of the same fabrication
# is the shape that drifts.
#
# WHAT THE PHASE 2B BRANCH ACTUALLY REQUIRES, and why a bare `return 0` is not
# enough:
#
#   * `--bind $_PILOT_EGRESS_SOCK …` — bwrap refuses to bind a source path that
#     does not exist, so the launch would fail for a reason unrelated to the
#     suite. A real unix socket is bound here rather than a plain file: that is
#     what production binds, and `_pilot_egress_sock_connectable` (used by the
#     canary and by anything that probes after us) answers truthfully on it.
#   * `--ro-bind $_PILOT_EGRESS_PROXY_BIN …` — same requirement, a real file.
#
# WHAT IT DOES NOT NEED, and this is what makes the fabrication cheap: the
# in-sandbox shim's wait loop is BOUNDED (~1 s) and non-blocking. If the shim
# never listens, the loop simply expires and the pilot is exec'd anyway. So the
# fake proxy binary does not have to proxy anything — it only has to exist. The
# cost is about one second per sandbox launch, paid once per suite.
#
# WHAT IS FORBIDDEN in these suites, and it is worth naming because each of the
# three looks like a shortcut (mika#2049 § Fire-Disposition D5):
#   (i)   stubbing `_run_pilot_sandboxed` itself — the suite would stop
#         inspecting a real launch, hence stop testing the invariant;
#   (ii)  relaxing or deleting an assertion to make the scenario pass;
#   (iii) arming `MIKA_PILOT_SANDBOX=0` in the harness, which would take the
#         suite out of the sandbox branch ENTIRELY and hollow out all four
#         invariants at once.
#
# Usage, after sourcing dispatch-lib.sh:
#
#     source "$SCRIPT_DIR/lib-fake-egress-relay.sh"
#     trap 'stop_fake_egress_relay; rm -rf "$TMPROOT"' EXIT
#     stub_serving_egress_relay "$TMPROOT"
#
# THE TRAP IS THE CALLER'S, deliberately. Installing one here would REPLACE the
# `trap 'rm -rf "$TMPROOT"' EXIT` every one of these suites already sets —
# bash keeps one handler per signal — and the suite would leak its temp tree on
# every run while looking perfectly healthy. Composing the two is the caller's
# call because only the caller knows what else its own handler must do.

# Fabricate a serving relay under $1 and point dispatch-lib at it.
#
# Overrides `_PILOT_EGRESS_SOCK` and `_PILOT_EGRESS_PROXY_BIN` (never the real
# host paths — a test must not bind /tmp/mika-pilot-egress.sock, which a live
# dispatch may be using) and replaces `_ensure_pilot_egress_proxy` with one that
# affirms the fabrication.
stub_serving_egress_relay() {
    local root="$1"
    mkdir -p "$root/egress"

    _PILOT_EGRESS_SOCK="$root/egress/relay.sock"
    _PILOT_EGRESS_PROXY_BIN="$root/egress/fake-egress-proxy"

    # A no-op proxy. It is `--ro-bind`ed into the sandbox and invoked by the
    # entrypoint shim; sleeping rather than exiting keeps it from looking like a
    # crash in the pilot's stderr, and the shim's EXIT trap kills it.
    cat > "$_PILOT_EGRESS_PROXY_BIN" <<'FAKE_EGRESS_PROXY'
#!/usr/bin/env python3
# mika#2049 test fixture — exists to be bound and executed, proxies nothing.
# The sandbox shim's wait loop is bounded, so never listening is harmless.
import time
# Bounded rather than endless: if a suite dies before its trap runs, the orphan
# is gone in ten minutes instead of living until the next reboot.
time.sleep(600)
FAKE_EGRESS_PROXY
    chmod +x "$_PILOT_EGRESS_PROXY_BIN"

    # A real listening unix socket, held open for the duration of the suite.
    python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(8)
time.sleep(600)
' "$_PILOT_EGRESS_SOCK" &
    _FAKE_EGRESS_LISTENER_PID=$!

    # Bounded wait for the bind, so the caller never races it.
    local i=0
    while [ $i -lt 50 ] && [ ! -S "$_PILOT_EGRESS_SOCK" ]; do
        sleep 0.1
        i=$((i + 1))
    done

    # The relay serves. `_run_pilot_sandboxed` therefore builds the full Phase 2b
    # shape — which is now the ONLY shape it builds.
    _ensure_pilot_egress_proxy() { _PILOT_EGRESS_ABORT=""; return 0; }

    # And the stamp side effects must not touch the operator's real state dir.
    # `_pilot_egress_mark_up` would `rm -f` a stamp a live outage had just
    # written; `_pilot_egress_mark_down` is unreachable here (the relay serves)
    # but is neutralised too, so a future edit to this harness cannot start
    # escalating to Telegram from `make test`.
    _pilot_egress_mark_up() { :; }
    _pilot_egress_mark_down() { :; }
}

# Tear the fabrication down. Idempotent; safe to call when the stub never ran.
stop_fake_egress_relay() {
    [ -n "${_FAKE_EGRESS_LISTENER_PID:-}" ] || return 0
    kill "$_FAKE_EGRESS_LISTENER_PID" 2>/dev/null || true
    unset _FAKE_EGRESS_LISTENER_PID
}
