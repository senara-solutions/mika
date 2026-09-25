#!/usr/bin/env bash
# Test fixture for scripts/smoke-webhook-chain (mika#2135).
#
# WHAT IT PINS
#   The probe's decision table, which is the whole of its value, plus the one
#   property the founding incident turned on: SILENCE on a healthy chain.
#
#   A probe that warns on a healthy system is the defect mika#2135 repairs, in
#   the other direction. A probe made unconditionally silent would not have
#   repaired anything either — it would have deleted a signal instead of making
#   it true (mika#2135 AC3, in its own words). So both halves are asserted:
#   every failure mode produces its verdict AND its wording, and the healthy case
#   produces an empty stream and exit 0.
#
#   The traversal verdict is asserted on a FAMILY of statuses, never on the 422
#   the incident happened to measure. Pinning 422 here would re-create, in the
#   test, the brittleness the probe deliberately refuses (see the probe's header,
#   "WHAT IT TESTS IS A FAMILY, NEVER A CODE"). The gateway side of that family is
#   pinned by `mika2135_*` in crates/mika-gateway/src/routes.rs.
#
# HOW
#   Two disposable local HTTP servers built with `python3 -m http.server`'s
#   machinery: one stands in for the local gateway's /health, one for the public
#   URL at the far end of the chain. Each is told which status to answer, so the
#   probe is exercised through its real curl calls and its real exit codes — no
#   function is stubbed and nothing is sourced.
#
# Usage: bash scripts/test-smoke-webhook-chain.sh
# Exit codes:
#   0 — all checks passed
#   1 — one or more checks failed
#   2 — setup error
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROBE="$REPO_ROOT/scripts/smoke-webhook-chain"

[ -r "$PROBE" ] || { echo "ERROR: $PROBE not readable" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "ERROR: python3 required" >&2; exit 2; }
command -v curl >/dev/null 2>&1 || { echo "ERROR: curl required" >&2; exit 2; }

PASS=0
FAIL=0
TMPDIR_TEST="$(mktemp -d)"
SERVER_PIDS=()

cleanup() {
  for pid in ${SERVER_PIDS+"${SERVER_PIDS[@]}"}; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  rm -rf "$TMPDIR_TEST"
}
trap cleanup EXIT

ok()   { PASS=$((PASS + 1)); echo "  ok   — $1"; }
bad()  { FAIL=$((FAIL + 1)); echo "  FAIL — $1" >&2; }

# ---------------------------------------------------------------------------
# A disposable server that answers one fixed status to everything.
# Writes its chosen port to a file so we never guess or race on a fixed port.
# ---------------------------------------------------------------------------
cat >"$TMPDIR_TEST/server.py" <<'PY'
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

status = int(sys.argv[1])
port_file = sys.argv[2]


class H(BaseHTTPRequestHandler):
    def _answer(self):
        self.send_response(status)
        self.send_header("Content-Length", "0")
        self.end_headers()

    do_GET = _answer
    do_POST = _answer

    def log_message(self, *_args):
        pass


srv = HTTPServer(("127.0.0.1", 0), H)
with open(port_file, "w") as fh:
    fh.write(str(srv.server_address[1]))
srv.serve_forever()
PY

# start_server <status> -> sets the global PORT. Call it as a STATEMENT, then
# read $PORT:
#
#     start_server 200; HEALTH_OK_PORT="$PORT"
#
# Never `HEALTH_OK_PORT="$(start_server 200)"`. A command substitution runs in a
# subshell, so both of this function's outputs die with it: `PORT` is assigned in
# the child and the parent never sees it, and `$!` is the child's notion of the
# background job, so `SERVER_PIDS` comes back empty and the cleanup trap has
# nothing to kill. The failure is silent and it is not a hang — the server is
# started, the substitution returns the empty string, and every call site then
# holds a port that is `""`. A probe handed an empty port falls back to the
# *real* gateway on the default 8080 for the health check and builds
# `http://127.0.0.1:/webhook/telegram` for the traversal, so the suite passes a
# check it never made and then fails 22 assertions on a curl exit 7. This comment
# already said "deliberately NOT" while every call site did exactly that
# (mika#2135, caught by QA on PR #2435) — the shape is pinned below by
# `mika2135_no_call_site_uses_command_substitution`.
start_server() {
  local status="$1"
  local port_file
  port_file="$(mktemp -p "$TMPDIR_TEST")"
  python3 -B "$TMPDIR_TEST/server.py" "$status" "$port_file" >/dev/null 2>&1 &
  SERVER_PIDS+=($!)
  # Wait for the port file to be written and the socket to accept.
  local waited=0
  while [ ! -s "$port_file" ]; do
    sleep 0.05
    waited=$((waited + 1))
    [ "$waited" -gt 100 ] && { echo "ERROR: server did not start" >&2; exit 2; }
  done
  PORT="$(cat "$port_file")"
}

# A port nothing listens on: bind one, read it, then let it go.
dead_port() {
  python3 -B -c '
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
'
}

# run_probe <health-port> <chain-url> -> sets RC, OUT (stdout+stderr merged)
run_probe() {
  local health_port="$1" chain_url="$2"
  OUT="$(
    MIKA_GATEWAY_PORT="$health_port" \
    MIKA_WEBHOOK_CHAIN_URL="$chain_url" \
    MIKA_WEBHOOK_CHAIN_LOCAL_TIMEOUT_SECS=2 \
    MIKA_WEBHOOK_CHAIN_TIMEOUT_SECS=5 \
    bash "$PROBE" 2>&1
  )"
  RC=$?
}

assert_rc() {
  local want="$1" label="$2"
  if [ "$RC" -eq "$want" ]; then ok "$label (exit $RC)"; else bad "$label — expected exit $want, got $RC. Output: $OUT"; fi
}

assert_contains() {
  local needle="$1" label="$2"
  case "$OUT" in
    *"$needle"*) ok "$label" ;;
    *) bad "$label — output does not contain '$needle'. Output: $OUT" ;;
  esac
}

assert_not_contains() {
  local needle="$1" label="$2"
  case "$OUT" in
    *"$needle"*) bad "$label — output unexpectedly contains '$needle'. Output: $OUT" ;;
    *) ok "$label" ;;
  esac
}

echo "== smoke-webhook-chain =="

start_server 200; HEALTH_OK_PORT="$PORT"
start_server 503; HEALTH_503_PORT="$PORT"
DEAD_PORT="$(dead_port)"   # `dead_port` genuinely echoes; a substitution is right here.

# ---------------------------------------------------------------------------
# 1. THE NEGATIVE CONTROL (AC3). Healthy chain ⇒ empty output, exit 0.
#    This is the assertion the founding incident is about. Every status of the
#    traversal family must produce it — not just the 422 that was measured.
# ---------------------------------------------------------------------------
for status in 400 401 404 415 422; do
  start_server "$status"; port="$PORT"
  run_probe "$HEALTH_OK_PORT" "http://127.0.0.1:$port/webhook/telegram"
  assert_rc 0 "traversal $status ⇒ chain alive"
  if [ -z "$OUT" ]; then
    ok "traversal $status ⇒ silence (AC3 negative control)"
  else
    bad "traversal $status ⇒ expected empty output, got: $OUT"
  fi
done

# ---------------------------------------------------------------------------
# 2. Reverse proxy that cannot reach its backend ⇒ chain broken, and the message
#    blames the hops in front of the gateway rather than the gateway.
# ---------------------------------------------------------------------------
for status in 502 504; do
  start_server "$status"; port="$PORT"
  run_probe "$HEALTH_OK_PORT" "http://127.0.0.1:$port/webhook/telegram"
  assert_rc 1 "traversal $status ⇒ chain broken"
  assert_contains "WARNING" "traversal $status ⇒ warns"
  assert_contains "does not reach it" "traversal $status ⇒ names the public path, not the gateway"
done

# ---------------------------------------------------------------------------
# 3. 503 is NOT "broken" — it is ambiguous between the gateway shedding load and
#    a Synology in trouble, and a warning that names the wrong culprit is the
#    defect being repaired. NOTE, never WARNING.
# ---------------------------------------------------------------------------
start_server 503; port="$PORT"
run_probe "$HEALTH_OK_PORT" "http://127.0.0.1:$port/webhook/telegram"
assert_rc 2 "traversal 503 ⇒ nothing verified"
assert_contains "NOTE:" "traversal 503 ⇒ a NOTE"
assert_not_contains "WARNING" "traversal 503 ⇒ never a warning"

# ---------------------------------------------------------------------------
# 4. An unexpected 2xx is not a pass: no handler of ours answers `{}` with a
#    success, so this is an answer the probe cannot attribute.
# ---------------------------------------------------------------------------
start_server 200; port="$PORT"
run_probe "$HEALTH_OK_PORT" "http://127.0.0.1:$port/webhook/telegram"
assert_rc 2 "traversal 200 ⇒ nothing verified (a 2xx is not evidence of traversal)"
assert_not_contains "WARNING" "traversal 200 ⇒ never a warning"

# ---------------------------------------------------------------------------
# 5. Unreachable public host ⇒ chain broken, and the message points at the hops
#    in front, having established the gateway itself is healthy.
# ---------------------------------------------------------------------------
run_probe "$HEALTH_OK_PORT" "http://127.0.0.1:$DEAD_PORT/webhook/telegram"
assert_rc 1 "unreachable public URL ⇒ chain broken"
assert_contains "did not come back" "unreachable public URL ⇒ names the transport failure"
assert_contains "Freebox" "unreachable public URL ⇒ names the real chain"

# ---------------------------------------------------------------------------
# 6. Local terminus down ⇒ chain broken at the LAST hop, and the probe says so
#    without ever crossing the public URL. The public server here answers 422,
#    so a probe that skipped the local check would wrongly report success.
# ---------------------------------------------------------------------------
start_server 422; port="$PORT"
run_probe "$DEAD_PORT" "http://127.0.0.1:$port/webhook/telegram"
assert_rc 1 "local gateway absent ⇒ chain broken"
assert_contains "not answering on localhost" "local gateway absent ⇒ names the gateway"
assert_contains "rc-service mika-gateway" "local gateway absent ⇒ gives the operator gesture"

# ---------------------------------------------------------------------------
# 7. Local terminus present but never ready ⇒ still broken, with a DIFFERENT
#    message. "Nothing is listening" and "it listens and will not come up" send
#    an operator to two different places.
#    The bounded poll (2s here) is also what stops a gateway still warming up
#    after `restart` from producing a warning on a healthy system.
# ---------------------------------------------------------------------------
start_server 422; port="$PORT"
run_probe "$HEALTH_503_PORT" "http://127.0.0.1:$port/webhook/telegram"
assert_rc 1 "local gateway not ready ⇒ chain broken"
assert_contains "is not ready" "local gateway not ready ⇒ distinguished from absent"

# ---------------------------------------------------------------------------
# 8. No declared URL ⇒ nothing verified. Never a warning, and never a pass:
#    the probe cannot derive the chain (mika#2135 D2), so it says what it needs.
# ---------------------------------------------------------------------------
OUT="$(
  env -u MIKA_WEBHOOK_CHAIN_URL -u MIKA_TELEGRAM_WEBHOOK_URL \
    MIKA_GATEWAY_PORT="$HEALTH_OK_PORT" \
    bash "$PROBE" 2>&1
)"
RC=$?
assert_rc 2 "no declared URL ⇒ nothing verified"
assert_contains "NOTE:" "no declared URL ⇒ a NOTE"
assert_not_contains "WARNING" "no declared URL ⇒ never a warning"
assert_contains "MIKA_WEBHOOK_CHAIN_URL" "no declared URL ⇒ names the variable to set"
assert_contains "docs/operator/local-webhook-topology.md" "no declared URL ⇒ names the topology doc"

# ---------------------------------------------------------------------------
# 9. The fallback declaration: the gateway's own variable is accepted when the
#    probe-specific override is absent.
# ---------------------------------------------------------------------------
start_server 422; port="$PORT"
OUT="$(
  env -u MIKA_WEBHOOK_CHAIN_URL \
    MIKA_TELEGRAM_WEBHOOK_URL="http://127.0.0.1:$port/webhook/telegram" \
    MIKA_GATEWAY_PORT="$HEALTH_OK_PORT" \
    MIKA_WEBHOOK_CHAIN_LOCAL_TIMEOUT_SECS=2 \
    MIKA_WEBHOOK_CHAIN_TIMEOUT_SECS=5 \
    bash "$PROBE" 2>&1
)"
RC=$?
assert_rc 0 "MIKA_TELEGRAM_WEBHOOK_URL is accepted as the declaration"
if [ -z "$OUT" ]; then ok "fallback declaration ⇒ silence"; else bad "fallback declaration ⇒ expected empty output, got: $OUT"; fi

# ---------------------------------------------------------------------------
# 10. AC2, asserted rather than assumed: no message the probe can emit names
#     ngrok or prescribes `ngrok http 8080`. Source-level, because a message
#     only some branch prints would escape a behavioural check.
# ---------------------------------------------------------------------------
if grep -nE '^[^#]*ngrok' "$PROBE" >/dev/null 2>&1; then
  bad "AC2 — the probe names ngrok outside a comment: $(grep -nE '^[^#]*ngrok' "$PROBE")"
else
  ok "AC2 — no executable line of the probe names ngrok"
fi

# ---------------------------------------------------------------------------
# 11. mika2135_no_call_site_uses_command_substitution — a source scan over this
#     very file, because the class it guards is invisible to every assertion
#     above. `port="$(start_server 422)"` makes no decision wrong: the server is
#     started, `$port` is the empty string, the probe silently falls back to the
#     real gateway on 8080 for its health check, and the suite then fails on a
#     curl exit 7 — twenty-two assertions blaming the probe for a defect in its
#     harness. That is what QA measured on PR #2435, under a comment that already
#     forbade the shape. A comment is not a guard.
#
#     The anti-vacuity half is load-bearing: a scan that finds no call site at
#     all reads exactly like a clean one, so the count is asserted rather than
#     assumed (class mika#2205).
# ---------------------------------------------------------------------------
SELF="$SCRIPT_DIR/$(basename "$0")"
substituted="$(grep -nE '\$\([[:space:]]*start_server\b' "$SELF" | grep -v '^[0-9]*:#' || true)"
if [ -n "$substituted" ]; then
  bad "mika2135 — start_server is called in a command substitution, so \$PORT is lost: $substituted"
else
  ok "mika2135 — no call site of start_server uses a command substitution"
fi

call_sites="$(grep -cE '^[[:space:]]*start_server[[:space:]]' "$SELF" || true)"
if [ "${call_sites:-0}" -ge 9 ]; then
  ok "mika2135 — the scan has a population ($call_sites statement call sites)"
else
  bad "mika2135 — only $call_sites statement call sites found; the scan above is looking at nothing"
fi

echo
echo "  $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
exit 0
