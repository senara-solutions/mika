#!/usr/bin/env bash
#
# smoke-webhook-flood.sh — post-deploy regression tripwire for the gateway↔server
# 429 self-amplification class (mika#1710, AC6).
#
# WHAT IT CHECKS
#   After a restart, fires a small sequence of mock `/message` POSTs at a
#   dedicated, guaranteed-idle `smoke-test` agent and asserts each is accepted
#   (HTTP 200/202) with no HTTP 429 "agent busy" rejection. A sustained 429 here
#   would reproduce the incident's self-amplifying flood on the delivery path.
#
# WHY SEQUENTIAL + PRECONDITIONED-IDLE (mika#1710 F4)
#   mika-spirit's `/message` is a per-agent concurrency-1 lock: while an agent is
#   mid-turn, every other inbound message gets an instant 429 BY DESIGN. So a
#   concurrent burst would 429 legitimately and tell us nothing. This script
#   therefore (a) targets a dedicated `smoke-test` agent — never `mika-dev`, which
#   may be mid-dispatch or draining DLQ events — and (b) waits for the agent lock
#   to clear between sends (bounded poll). If the agent never reaches idle it
#   SKIPS fail-open rather than emit a false regression signal.
#
# FAIL-OPEN POSTURE (matches `check-ngrok`)
#   Skips cleanly (exit 0) when: MIKA_INTERNAL_TOKEN is unavailable, mika-spirit
#   is unreachable, or the `smoke-test` agent is not provisioned (404). This is a
#   tripwire, not a gate — `make deploy` wires it non-fatal so a benign warmup
#   blip never blocks a deploy. On a real regression it prints a loud, actionable
#   warning and exits non-zero (the Makefile downgrades that to a warning).
#
# CONFIG (env)
#   MIKA_SPIRIT_URL     base URL of mika-spirit        (default http://localhost:8080)
#   MIKA_INTERNAL_TOKEN bearer token for /message      (required; skip if unset)
#   MIKA_SMOKE_AGENT    target agent name              (default smoke-test)
#   MIKA_SMOKE_COUNT    number of messages to fire     (default 10)
#   MIKA_SMOKE_IDLE_TIMEOUT_SECS  max wait per send for idle (default 15)

set -u

SPIRIT_URL="${MIKA_SPIRIT_URL:-http://localhost:8080}"
AGENT="${MIKA_SMOKE_AGENT:-smoke-test}"
COUNT="${MIKA_SMOKE_COUNT:-10}"
IDLE_TIMEOUT="${MIKA_SMOKE_IDLE_TIMEOUT_SECS:-15}"

skip() {
  echo ""
  echo "  ⏭  smoke-webhook-flood: SKIP — $1"
  echo ""
  exit 0
}

fail() {
  echo ""
  echo "  ⚠  smoke-webhook-flood: REGRESSION SUSPECTED — $1"
  echo "     This is the mika#1710 429-flood class. Inspect gateway.log / server.log"
  echo "     for 'status':429 storms and the 'gateway_target_paused' / 'rate_limit_trip' events."
  echo ""
  exit 1
}

command -v curl >/dev/null 2>&1 || skip "curl not found"
[ -n "${MIKA_INTERNAL_TOKEN:-}" ] || skip "MIKA_INTERNAL_TOKEN not set (cannot authenticate /message)"

# 1. mika-spirit reachable and ready?
health="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$SPIRIT_URL/health" 2>/dev/null || echo 000)"
[ "$health" = "200" ] || skip "mika-spirit not ready at $SPIRIT_URL/health (got '$health')"

# Fire one message; echoes the HTTP status code. Trivial text keeps the turn short.
send_one() {
  local rid="$1"
  curl -s -o /dev/null -w '%{http_code}' --max-time 10 \
    -X POST "$SPIRIT_URL/message" \
    -H "authorization: Bearer ${MIKA_INTERNAL_TOKEN}" \
    -H 'content-type: application/json' \
    -d "{\"text\":\"ping\",\"channel\":\"smoke\",\"request_id\":\"${rid}\",\"agent\":\"${AGENT}\"}" \
    2>/dev/null || echo 000
}

# 2. Readiness probe: confirm the agent exists and is currently idle.
probe="$(send_one "smoke-probe-0")"
case "$probe" in
  404) skip "agent '$AGENT' is not provisioned (404) — nothing to smoke-test" ;;
  000) skip "could not reach $SPIRIT_URL/message (connection error)" ;;
esac

echo "smoke-webhook-flood: firing $COUNT sequential messages at idle agent '$AGENT' (no-429 expected)"

accepted=0
rejected=0
for i in $(seq 1 "$COUNT"); do
  # Wait for the agent lock to clear (poll on a probe) before the real send, so a
  # still-running prior turn doesn't produce a by-design busy-lock 429.
  waited=0
  while :; do
    code="$(send_one "smoke-$i")"
    case "$code" in
      200|202)
        accepted=$((accepted + 1))
        break
        ;;
      429)
        if [ "$waited" -ge "$IDLE_TIMEOUT" ]; then
          rejected=$((rejected + 1))
          echo "  ✗ msg $i: still 429 after ${IDLE_TIMEOUT}s — agent not draining"
          break
        fi
        sleep 1
        waited=$((waited + 1))
        ;;
      404) skip "agent '$AGENT' disappeared mid-run (404)" ;;
      *)
        rejected=$((rejected + 1))
        echo "  ✗ msg $i: unexpected status $code"
        break
        ;;
    esac
  done
done

echo "smoke-webhook-flood: accepted=$accepted rejected=$rejected of $COUNT"
if [ "$rejected" -gt 0 ]; then
  fail "$rejected/$COUNT messages were not accepted (sustained 429 or error)"
fi

echo "  ✓ smoke-webhook-flood: PASS — all $COUNT messages accepted, no sustained 429"
exit 0
