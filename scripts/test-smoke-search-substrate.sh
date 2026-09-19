#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/smoke-search-substrate (mika#2407).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# THE ONE ASSERTION THIS FILE EXISTS FOR is `404 search_upstream_not_configured
# must exit non-zero`. That is AC4, and it is the whole reason the smoke is
# worth writing: a smoke that returned 0 in every circumstance would look green
# on the rotation of 2026-09-18 and would reinstall, one layer higher, exactly
# the silence it was written to break. Every other case here exists to keep that
# one honest — a script that exited 1 unconditionally would also pass it.
#
# The gateway is faked, deliberately and completely. The smoke's contract is
# with HTTP statuses and a taxonomy label, not with a Rust handler, so a fake
# that speaks those is a full-fidelity stand-in — and it lets the suite run in
# CI with no gateway, no database, no token and, above all, no upstream request.
# A test suite that spent a real search from a shared free-tier quota on every
# CI run would be a defect of its own.
#
# The battery carries accented content on purpose: this repository writes its
# plans, tickets and logs in French, and a fixture set that is ASCII-only tests
# a population that does not exist here.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SMOKE="$REPO_ROOT/scripts/smoke-search-substrate"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d)"
FAKE_PID=""
cleanup() {
    if [[ -n "$FAKE_PID" ]]; then
        kill "$FAKE_PID" 2>/dev/null || true
        wait "$FAKE_PID" 2>/dev/null || true
    fi
    rm -rf "$TMPROOT"
}
trap cleanup EXIT

# -- the fake gateway ---------------------------------------------------------
#
# One mode per behaviour the smoke must separate, chosen by $FAKE_MODE. It also
# records every request it served, so the suite can assert the "exactly one
# request" property (AC2) rather than trusting the script's own prose about it.

cat >"$TMPROOT/fake_gateway.py" <<'PY'
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

MODE = os.environ["FAKE_MODE"]
HITS = os.environ["FAKE_HITS"]

BODIES = {
    # (status, body) per mode.
    "healthy": (200, {"results": [{"title": "Résultat éphémère",
                                   "url": "https://example.invalid/",
                                   "snippet": "contenu de test — jamais affiché"}],
                      "upstream_latency_ms": 12}),
    "healthy_empty": (200, {"results": [], "upstream_latency_ms": 7}),
    # The 2026-09-18 failure, byte for byte as egress_search::handle_internal_search
    # writes it when `search_egress_client` is None.
    "not_configured": (404, {"error": "search_upstream_not_configured"}),
    # A 404 that is NOT the substrate's: a base URL pointing at something else.
    "foreign_404": (404, {"error": "not_found"}),
    "unauthorized": (502, {"error": "unauthorized"}),
    "upstream_error": (502, {"error": "upstream_error"}),
    "transport": (502, {"error": "transport"}),
    "not_implemented": (501, {"error": "not_implemented"}),
    # The bearer middleware refusing us: nothing about the substrate was learned.
    "bearer_refused": (401, {"error": "unauthorized"}),
    "garbage_body": (200, "pas du json"),
}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        with open(HITS, "a", encoding="utf-8") as fh:
            fh.write(self.path + "\n")
        length = int(self.headers.get("Content-Length", 0))
        self.rfile.read(length)
        status, body = BODIES[MODE]
        payload = (body if isinstance(body, str) else json.dumps(body)).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_args):
        pass


server = HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
PY

HITS_FILE="$TMPROOT/hits"

# Start the fake in mode $1; echo nothing, set $BASE_URL and reset the hit log.
start_fake() {
    local mode="$1"
    : >"$HITS_FILE"
    # Truncate the port file BEFORE the fork, not by the redirection itself.
    # The redirection truncates at exec time, so between `&` and that exec the
    # file still holds the *previous* run's port — and the wait loop below,
    # which only tests for non-emptiness, would read a port whose server has
    # just been killed. That race produced five failures on the first run of
    # this suite, all of them curl exit 7 against a stale port.
    : >"$TMPROOT/port"
    FAKE_MODE="$mode" FAKE_HITS="$HITS_FILE" \
        python3 "$TMPROOT/fake_gateway.py" >"$TMPROOT/port" 2>"$TMPROOT/fake.err" &
    FAKE_PID=$!
    # Wait for the port line rather than sleeping: a fixed sleep is either slow
    # or flaky, and on a loaded CI runner it is both.
    local waited=0
    while [[ ! -s "$TMPROOT/port" ]]; do
        sleep 0.05
        waited=$((waited + 1))
        if [[ $waited -gt 200 ]]; then
            echo "FATAL: fake gateway did not start (mode=$mode)" >&2
            cat "$TMPROOT/fake.err" >&2
            exit 1
        fi
    done
    BASE_URL="http://127.0.0.1:$(cat "$TMPROOT/port")"
}

stop_fake() {
    if [[ -n "$FAKE_PID" ]]; then
        kill "$FAKE_PID" 2>/dev/null || true
        wait "$FAKE_PID" 2>/dev/null || true
        FAKE_PID=""
    fi
}

ok() {
    echo "PASS: $1"
    PASS=$((PASS + 1))
}

ko() {
    echo "FAIL: $1"
    FAIL=$((FAIL + 1))
}

# Run the smoke against a fake in mode $1; assert exit == $2. $3 = case name.
assert_mode_exit() {
    local mode="$1" want="$2" name="$3"
    start_fake "$mode"
    local got=0
    MIKA_INTERNAL_TOKEN="$(printf 'a%.0s' {1..64})" \
        bash "$SMOKE" "$BASE_URL" >"$TMPROOT/out" 2>"$TMPROOT/err" || got=$?
    stop_fake
    if [[ "$got" -eq "$want" ]]; then
        ok "$name (exit $got)"
    else
        ko "$name (wanted exit $want, got $got)"
        sed 's/^/    /' "$TMPROOT/err" >&2
    fi
}

# Assert the smoke's combined output for mode $1 contains the literal $2.
assert_mode_output_contains() {
    local mode="$1" needle="$2" name="$3"
    start_fake "$mode"
    MIKA_INTERNAL_TOKEN="$(printf 'a%.0s' {1..64})" \
        bash "$SMOKE" "$BASE_URL" >"$TMPROOT/out" 2>"$TMPROOT/err" || true
    stop_fake
    if grep -qF -- "$needle" "$TMPROOT/out" "$TMPROOT/err"; then
        ok "$name"
    else
        ko "$name (output did not contain: $needle)"
    fi
}

# Assert the smoke's combined output for mode $1 does NOT contain the literal $2.
assert_mode_output_lacks() {
    local mode="$1" needle="$2" name="$3"
    start_fake "$mode"
    MIKA_INTERNAL_TOKEN="$(printf 'a%.0s' {1..64})" \
        bash "$SMOKE" "$BASE_URL" >"$TMPROOT/out" 2>"$TMPROOT/err" || true
    stop_fake
    if grep -qF -- "$needle" "$TMPROOT/out" "$TMPROOT/err"; then
        ko "$name (output leaked: $needle)"
    else
        ok "$name"
    fi
}

echo "== mika#2407 — smoke-search-substrate =="

# -- AC4: the assertion this file exists for ---------------------------------
#
# A smoke returning 0 here would pass a gateway with no search substrate, which
# is the exact state six tenants were served for twenty hours.
assert_mode_exit not_configured 1 \
    "AC4 — 404 search_upstream_not_configured fails the deployment"

assert_mode_output_contains not_configured "MIKA_SEARCH_UPSTREAM" \
    "AC4 — the failure names the selector an operator must set"
assert_mode_output_contains not_configured "MIKA_BRAVE_API_KEY" \
    "AC4 — and the key, because one without the other activates nothing"

# -- AC2: 404 and 200 are separated, and one request is spent ----------------

assert_mode_exit healthy 0 "AC2 — 200 is a pass"
assert_mode_exit healthy_empty 0 \
    "AC2 — 200 with zero results is still a pass (we assert on status, not content)"

start_fake healthy
MIKA_INTERNAL_TOKEN="$(printf 'a%.0s' {1..64})" bash "$SMOKE" "$BASE_URL" >/dev/null 2>&1
stop_fake
hits="$(wc -l <"$HITS_FILE" | tr -d ' ')"
if [[ "$hits" -eq 1 ]]; then
    ok "AC2 — exactly one request reaches the gateway (quota is shared and small)"
else
    ko "AC2 — expected exactly 1 request, the gateway saw $hits"
fi

# -- The third verdict: indeterminate is neither pass nor substrate failure ---

assert_mode_exit unauthorized 1 \
    "502 unauthorized is a substrate failure — the key is present and refused"
assert_mode_output_contains unauthorized "rotation" \
    "502 unauthorized names key rotation, not the missing selector"

assert_mode_exit upstream_error 2 \
    "502 upstream_error is indeterminate — wired substrate, bad upstream moment"
assert_mode_exit transport 2 "502 transport is indeterminate"
assert_mode_exit not_implemented 2 "501 not_implemented is indeterminate"
assert_mode_exit bearer_refused 2 \
    "401 is indeterminate — we could not authenticate, so we verified nothing"
assert_mode_exit foreign_404 2 \
    "a 404 that is not the substrate's own is indeterminate, not a broken substrate"
assert_mode_exit garbage_body 0 \
    "an unparseable 200 body is still a 200 — the status is the verdict"

# -- Nothing verified is never a pass ----------------------------------------

start_fake healthy
got=0
env -u MIKA_INTERNAL_TOKEN bash "$SMOKE" "$BASE_URL" >"$TMPROOT/out" 2>"$TMPROOT/err" || got=$?
stop_fake
if [[ "$got" -eq 2 ]]; then
    ok "no token ⇒ exit 2, never 0 (an unauthenticated smoke verified nothing)"
else
    ko "no token ⇒ wanted exit 2, got $got"
fi
# And it must not even have tried: spending a request it cannot authenticate
# would burn shared quota for nothing.
if [[ ! -s "$HITS_FILE" ]]; then
    ok "no token ⇒ no request spent"
else
    ko "no token ⇒ a request was sent anyway"
fi

got=0
MIKA_INTERNAL_TOKEN="   " bash "$SMOKE" "http://127.0.0.1:1" >/dev/null 2>&1 || got=$?
[[ "$got" -eq 2 ]] && ok "whitespace-only token ⇒ exit 2" || ko "whitespace-only token ⇒ got $got"

got=0
MIKA_INTERNAL_TOKEN="x" bash "$SMOKE" >/dev/null 2>&1 || got=$?
[[ "$got" -eq 2 ]] && ok "no URL ⇒ exit 2" || ko "no URL ⇒ got $got"

# An unreachable host: transport failure says nothing about the substrate.
got=0
MIKA_INTERNAL_TOKEN="$(printf 'a%.0s' {1..64})" \
    bash "$SMOKE" "http://127.0.0.1:1" >/dev/null 2>&1 || got=$?
[[ "$got" -eq 2 ]] && ok "unreachable gateway ⇒ exit 2" || ko "unreachable gateway ⇒ got $got"

# -- What it must never print -------------------------------------------------
#
# The token is the operator's shared bearer; the results are somebody's search.
# Both assertions are cheap and both would be discovered late.
start_fake healthy
TOKEN_SENTINEL="sentinelle-du-jeton-a8c12d0c"
MIKA_INTERNAL_TOKEN="$TOKEN_SENTINEL" bash "$SMOKE" "$BASE_URL" >"$TMPROOT/out" 2>"$TMPROOT/err"
stop_fake
if grep -qF -- "$TOKEN_SENTINEL" "$TMPROOT/out" "$TMPROOT/err"; then
    ko "the token must never be printed"
else
    ok "the token is never printed"
fi

assert_mode_output_lacks healthy "Résultat éphémère" \
    "search result titles are never printed (a deployment check is not a search log)"
assert_mode_output_lacks healthy "example.invalid" \
    "search result URLs are never printed"

# -- Report -------------------------------------------------------------------

echo ""
echo "passed: $PASS   failed: $FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
exit 0
