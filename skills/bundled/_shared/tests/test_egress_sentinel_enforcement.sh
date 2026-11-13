#!/bin/bash
# mika#2724 (vol 2 de mika#2716): the egress addon REQUIRES the pilot sentinel
# on api.github.com, refuses with three distinct reasons, and never refuses the
# legitimate path.
#
# WHY THIS SUITE EXISTS, next to the canary. The canary reads the INSIDE of the
# sandbox and runs post-deploy, by hand. The refusal lives in the addon, host
# side, and the failure mode the parent ticket names as the only one able to
# stop the loop — "un bug d'injection ferait silencieusement 403 tous les appels
# légitimes" — would land at `make deploy`, after the merge. A guard the
# pipeline does not run refuses nothing (mika#2578, invoked twice already by
# vol 1). This suite is the pipeline's half: `make test-egress-sentinel-enforcement`,
# called by ci.yml in the very commit that creates it.
#
# WHY IT DRIVES `requestheaders` AND NOT ONLY THE PURE FUNCTIONS. A green,
# unplugged verdict function is class mika#2205 — two states rendering the same
# bytes. Every case below goes through `requestheaders` itself, so a verdict
# function that is perfect and never called reddens here. The price is a ~30
# line mitmproxy stub (Headers / Request / Response.make / HTTPFlow), paid once.
#
# WHAT IS NOT HERE, deliberately. mitmproxy is not a test dependency of this
# repo (measured: `import mitmproxy` fails on this host), so the real proxy is
# never launched. The contract under test is the addon's DECISION and its
# rewrite of the request, both observable on the stubbed flow. The real-proxy
# half is the canary's three `egress_sentinel_*` probes (§ 3.3 of the plan).
#
# Run: bash skills/bundled/_shared/tests/test_egress_sentinel_enforcement.sh
# Expected: all assertions pass, exit 0. Needs no bwrap, no gh, no mitmproxy
# and no network. Runs as root as well as unprivileged: the unreadable-registry
# cases point the registry AT A REGULAR FILE or at an absent path rather than
# chmod'ing a directory, so they do not depend on permissions root ignores.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
GH_ADDON="$REPO_ROOT/scripts/mika-pilot-github-auth-addon.py"

PASS=0
FAIL=0
assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"; echo "    actual:   '$actual'"
    fi
}

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2724-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT
export HOME="$TMPROOT/home"
mkdir -p "$HOME/.mika"

# The host-side token the addon injects. Deliberately recognisable: case 10
# asserts it appears in NO refusal body and in NO DENY log line.
printf 'ghp_hostTOKEN2724hostTOKEN2724hostTOK\n' > "$HOME/.mika/pilot-gh-token"
chmod 0600 "$HOME/.mika/pilot-gh-token"

# The addon's env fallback must not shadow the file, and an inherited GH_TOKEN
# from the surrounding shell would make the token unrecognisable.
unset GH_TOKEN MIKA_GITHUB_TOKEN MIKA_PILOT_EGRESS_SENTINEL _PILOT_EGRESS_SENTINEL_DIR

echo ""
echo "The addon requires the egress sentinel on api.github.com (mika#2724)"
echo "---------------------------------------------------------------------"

PY_OUT="$TMPROOT/python.out"
python3 - "$GH_ADDON" "$TMPROOT" >"$PY_OUT" 2>&1 <<'PY'
import base64
import importlib.util
import io
import os
import sys
import time
import types
from contextlib import redirect_stderr
from pathlib import Path

ADDON_PATH, TMPROOT = sys.argv[1], sys.argv[2]

HOST_TOKEN = "ghp_hostTOKEN2724hostTOKEN2724hostTOK"
BARE = "proxy-managed-no-secret"
NONCE = "a1b2c3d4e5f60718293a4b5c6d7e8f90"
SENTINEL = f"{BARE}-{NONCE}"
BOGUS = "not-a-sentinel-canary-2724"
API = "api.github.com"
GIT = "github.com"

failures = 0


def check(label, expected, actual):
    global failures
    ok = expected == actual
    if not ok:
        failures += 1
    print(f"RESULT {'PASS' if ok else 'FAIL'} {label}")
    if not ok:
        print(f"      expected: {expected!r}")
        print(f"      actual:   {actual!r}")


def check_true(label, condition):
    check(label, True, bool(condition))


# ---------------------------------------------------------------- mitmproxy stub
class Headers:
    """The slice of mitmproxy.http.Headers the addon touches, case-insensitive.

    Case-insensitivity is not decoration: the addon pops lowercase names and
    writes `Authorization`, and `gh` sends `Authorization` while a proxy may
    normalise. A case-sensitive stub would pass an addon that strips nothing.
    """

    def __init__(self):
        self._d = {}

    def get(self, name, default=None):
        hit = self._d.get(name.lower())
        return hit if hit is not None else default

    def __getitem__(self, name):
        hit = self._d.get(name.lower())
        if hit is None:
            raise KeyError(name)
        return hit

    def __setitem__(self, name, value):
        self._d[name.lower()] = value

    def pop(self, name, default=None):
        return self._d.pop(name.lower(), default)

    def __contains__(self, name):
        return name.lower() in self._d


class Request:
    def __init__(self, host, method, path, headers):
        self.host, self.method, self.path, self.headers = host, method, path, headers


class Response:
    def __init__(self, status_code, content, headers):
        self.status_code, self.content, self.headers = status_code, content, headers

    @classmethod
    def make(cls, status_code=200, content=b"", headers=None):
        if isinstance(content, str):
            content = content.encode("utf-8")
        return cls(status_code, content, headers or {})


class HTTPFlow:
    def __init__(self, request):
        self.request, self.response = request, None


_mitm = types.ModuleType("mitmproxy")
_http = types.ModuleType("mitmproxy.http")
_http.Response = Response
_http.HTTPFlow = HTTPFlow
_mitm.http = _http
sys.modules["mitmproxy"] = _mitm
sys.modules["mitmproxy.http"] = _http

_spec = importlib.util.spec_from_file_location("mika_addon_2724", ADDON_PATH)
addon = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(addon)

# ------------------------------------------------------------------- fixtures
REGISTRY = Path(TMPROOT) / "registry"
REGISTRY.mkdir(parents=True, exist_ok=True)
(REGISTRY / NONCE).write_text("", encoding="utf-8")
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(REGISTRY)

NOT_A_DIR = Path(TMPROOT) / "registry-is-a-file"
NOT_A_DIR.write_text("", encoding="utf-8")
ABSENT = Path(TMPROOT) / "registry-absent"


def run(host, cred=None, header="Authorization", method="GET", path="/rate_limit"):
    """Drive the real `requestheaders`; return (flow, captured stderr)."""
    headers = Headers()
    if cred is not None:
        headers[header] = cred
    flow = HTTPFlow(Request(host, method, path, headers))
    err = io.StringIO()
    with redirect_stderr(err):
        addon.requestheaders(flow)
    return flow, err.getvalue()


def body_of(flow):
    return flow.response.content.decode("utf-8", "replace") if flow.response else ""


def status_of(flow):
    return flow.response.status_code if flow.response else 200


def ctype_of(flow):
    # Never dereference a missing response: a red case must stay ONE red line,
    # not a traceback that takes every later case with it.
    return flow.response.headers.get("Content-Type") if flow.response else None


print("")
print("CASE 1 — a registered sentinel presented as Bearer is let through")
flow, _ = run(API, f"Bearer {SENTINEL}")
check("1.1 no refusal", 200, status_of(flow))
check("1.2 the HOST credential goes out, not the presented one",
      f"Bearer {HOST_TOKEN}", flow.request.headers.get("Authorization"))
check_true("1.3 the sentinel does not reach GitHub",
           SENTINEL not in (flow.request.headers.get("Authorization") or ""))

print("")
print("CASE 2 — `token <s>`, the scheme gh really emits (captured on the wire,")
print("         gh v2.92.0: `Authorization: token <tok>`, never Bearer)")
flow, _ = run(API, f"token {SENTINEL}")
check("2.1 accepted", 200, status_of(flow))
check("2.2 host credential injected", f"Bearer {HOST_TOKEN}",
      flow.request.headers.get("Authorization"))
flow, _ = run(API, f"TOKEN {SENTINEL}")
check("2.3 the scheme comparison is case-insensitive (RFC 7235)", 200, status_of(flow))
flow, _ = run(API, f"token {SENTINEL}", header="Proxy-Authorization")
check("2.4 Proxy-Authorization is read with the same precedence", 200, status_of(flow))
check_true("2.5 and is stripped before forwarding",
           flow.request.headers.get("Proxy-Authorization") is None)

print("")
print("CASE 3 — Basic <base64> is decoded, and a malformed one denies, never raises")
basic = base64.b64encode(f"x-access-token:{SENTINEL}".encode()).decode("ascii")
flow, _ = run(API, f"Basic {basic}")
check("3.1 Basic carrying the sentinel as password is accepted", 200, status_of(flow))
for label, broken in (
    ("3.2 Basic with non-base64 payload", "Basic !!!not-base64!!!"),
    ("3.3 Basic with no colon once decoded",
     "Basic " + base64.b64encode(b"nocolonhere").decode("ascii")),
    ("3.4 Basic with an empty payload", "Basic "),
    ("3.5 a scheme with no value at all", "Bearer"),
):
    try:
        flow, _ = run(API, broken)
        got = status_of(flow)
    except Exception as exc:  # a crash here would be a 500 for every call
        got = f"raised {type(exc).__name__}: {exc}"
    check(f"{label} denies as absent (never raises, never allows)", 403, got)

print("")
print("CASE 4 — nothing presented: 403 sentinel-absent, with a DENY line")
flow, err = run(API, None, method="POST", path="/graphql")
check("4.1 refused", 403, status_of(flow))
check_true("4.2 the body names the reason in clear", "sentinel-absent" in body_of(flow))
check_true("4.3 the body names the ticket that explains it", "mika#2716" in body_of(flow))
check_true("4.4 the body carries a remedy, not just a token",
           len(body_of(flow)) >= 80)
check("4.5 the refusal is text/plain", "text/plain; charset=utf-8", ctype_of(flow))
check_true("4.6 NO credential is injected on a refusal",
           flow.request.headers.get("Authorization") is None)
check("4.7 one greppable DENY line on stderr, mirroring ALLOW",
      "[github-proxy] DENY sentinel-absent POST /graphql", err.strip())
absent_body = body_of(flow)

print("")
print("CASE 5 — a value unknown to the registry: 403 sentinel-unknown")
flow, err = run(API, f"Bearer {BOGUS}")
check("5.1 refused", 403, status_of(flow))
check_true("5.2 the body names THIS reason", "sentinel-unknown" in body_of(flow))
check_true("5.3 and not the other one", "sentinel-absent" not in body_of(flow))
check("5.4 the DENY line names this reason",
      "[github-proxy] DENY sentinel-unknown GET /rate_limit", err.strip())
# The whole point of two reasons (mika#2205): absent and unknown have opposite
# causes — a cargo test that overwrote GH_TOKEN vs a registry that is not the
# one we think. Identical bytes would make the difference undiagnosable at the
# moment it decides the remedy.
check_true("5.5 the two refusal bodies differ byte for byte",
           body_of(flow) != absent_body)
unknown_body = body_of(flow)

print("")
print("CASE 6 — the bare placeholder is accepted WITHOUT touching the registry")
# MIKA_PILOT_EGRESS_SENTINEL=0 launcher-side presents the bare prefix. If the
# addon refused it, the rollback vol 1 shipped would become a guard that
# refuses everything (mika#2165). The registry is made unreadable in the SAME
# case on purpose: the acceptance must not depend on a readable registry, or
# the rollback would not survive the very breakage it is there to work around.
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(NOT_A_DIR)
flow, err = run(API, f"Bearer {BARE}")
check("6.1 accepted on a BROKEN registry", 200, status_of(flow))
check("6.2 and the host credential still goes out", f"Bearer {HOST_TOKEN}",
      flow.request.headers.get("Authorization"))
check("6.3 nothing is logged as a refusal", "", err.strip())
flow, _ = run(API, f"token {BARE}")
check("6.4 same through the `token` scheme", 200, status_of(flow))

print("")
print("CASE 7 — an unreadable registry REFUSES (fail-closed, § 3.2)")
# Deliberately the opposite of the launcher's fail-open. Launcher-side an
# unreadable registry must not kill a dispatch; addon-side it must not
# authorise everyone. Written here, in the test, because it is a DECISION and
# not an implementation detail.
for label, path in (("a regular file", NOT_A_DIR), ("an absent directory", ABSENT)):
    os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(path)
    flow, err = run(API, f"Bearer {SENTINEL}")
    check(f"7.x refused when the registry is {label}", 403, status_of(flow))
    check_true(f"7.x the body names registry-unreadable ({label})",
               "registry-unreadable" in body_of(flow))
    check(f"7.x the DENY line names it ({label})",
          "[github-proxy] DENY registry-unreadable GET /rate_limit", err.strip())
check_true("7.9 the third body differs from the other two",
           body_of(flow) not in (absent_body, unknown_body))
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(REGISTRY)

print("")
print("CASE 8 — github.com is strictly unchanged (§ 2.1–2.3: scope reduction)")
# `git` presents NO credential in the pilot sandbox (measured at grooming), so
# requiring one on github.com would refuse every push for nothing. The reduced
# scope is the decision; this case is what makes a later widening visible.
expected_basic = "Basic " + base64.b64encode(
    f"x-access-token:{HOST_TOKEN}".encode()).decode("ascii")
for label, cred in (("no credential", None),
                    ("an unknown value", f"Bearer {BOGUS}"),
                    ("the bare placeholder", f"Bearer {BARE}"),
                    ("a registered sentinel", f"Bearer {SENTINEL}")):
    flow, err = run(GIT, cred, method="POST", path="/senara-solutions/mika.git/git-upload-pack")
    check(f"8.x github.com with {label}: never a 403", 200, status_of(flow))
    check(f"8.x github.com with {label}: Basic injection unchanged",
          expected_basic, flow.request.headers.get("Authorization"))
    check(f"8.x github.com with {label}: no DENY line", "", err.strip())

print("")
print("CASE 9 — TTL purge: an aged entry goes, a fresh one stays")
PURGED = "0000000000000000000000000000dead"
KEPT_YOUNG = "1111111111111111111111111111beef"
for name in (PURGED, KEPT_YOUNG):
    (REGISTRY / name).write_text("", encoding="utf-8")
now = time.time()
os.utime(REGISTRY / PURGED, (now - 73 * 3600, now - 73 * 3600))     # past 72 h
os.utime(REGISTRY / KEPT_YOUNG, (now - 71 * 3600, now - 71 * 3600))  # inside 72 h
addon._last_purge = {}  # the 10-minute throttle must not hide the purge here
flow, _ = run(API, f"Bearer {SENTINEL}")
check("9.1 the entry validated in this very call is never purged", 200, status_of(flow))
check_true("9.2 the entry past the TTL is gone", not (REGISTRY / PURGED).exists())
check_true("9.3 the entry inside the TTL survives", (REGISTRY / KEPT_YOUNG).exists())
check_true("9.4 the entry being validated survives", (REGISTRY / NONCE).exists())
flow, _ = run(API, f"Bearer {BARE}-{KEPT_YOUNG}")
check("9.5 and it is still a valid credential", 200, status_of(flow))
# The throttle: at most one pass every 10 minutes, so a busy proxy does not
# scandir on every request. Observable, hence asserted.
THROTTLED = "2222222222222222222222222222cafe"
(REGISTRY / THROTTLED).write_text("", encoding="utf-8")
os.utime(REGISTRY / THROTTLED, (now - 99 * 3600, now - 99 * 3600))
flow, _ = run(API, f"Bearer {SENTINEL}")
check("9.6 a second pass inside 10 minutes does not re-scan", 200, status_of(flow))
check_true("9.7 so the newly-aged entry is still there (throttle honoured)",
           (REGISTRY / THROTTLED).exists())
addon._last_purge = {}

print("")
print("CASE 10 — the DENY log and the 403 bodies name, and never disclose")
# mika#1949: nommer, pas divulguer. A fingerprint is deferred disclosure, so
# the assertion is on the raw values AND on any hex digest of them.
import hashlib  # noqa: E402 — local to this case, as is the doctrine it serves
leaky = f"{BARE}-{'f' * 32}"
flow, err = run(API, f"Bearer {leaky}")
combined = err + body_of(flow)
check("10.1 refused, so there IS something to inspect", 403, status_of(flow))
check_true("10.2 the capture is non-vacuous (the DENY line is really here)",
           "[github-proxy] DENY sentinel-unknown" in err)
check_true("10.3 the presented value appears nowhere", leaky not in combined)
check_true("10.4 nor its tail alone", ("f" * 32) not in combined)
check_true("10.5 the host token appears nowhere", HOST_TOKEN not in combined)
for algo in ("md5", "sha1", "sha256"):
    for label, secret in (("presented value", leaky), ("host token", HOST_TOKEN)):
        digest = hashlib.new(algo, secret.encode()).hexdigest()
        check_true(f"10.x no {algo} fingerprint of the {label}",
                   digest not in combined and digest[:12] not in combined)

print("")
print("CASE 11 — the addon honours the operator switch (second reader, mika#2165)")
os.environ["MIKA_PILOT_EGRESS_SENTINEL"] = "0"
flow, err = run(API, None)
check("11.1 disarmed: nothing presented is no longer a refusal", 200, status_of(flow))
check("11.2 disarmed: the host credential is injected as on origin/main",
      f"Bearer {HOST_TOKEN}", flow.request.headers.get("Authorization"))
check("11.3 disarmed: no DENY line", "", err.strip())
for value in ("FALSE", "no", "Off", "disabled"):
    os.environ["MIKA_PILOT_EGRESS_SENTINEL"] = value
    flow, _ = run(API, None)
    check(f"11.x `{value}` disarms too (same vocabulary as the shell half)",
          200, status_of(flow))
for value in ("1", "", "yes", "garbage"):
    os.environ["MIKA_PILOT_EGRESS_SENTINEL"] = value
    flow, _ = run(API, None)
    check(f"11.x `{value}` leaves the guard ARMED (default is armed)",
          403, status_of(flow))
del os.environ["MIKA_PILOT_EGRESS_SENTINEL"]

print("")
print("CASE 12 — the registry path is resolved per call, not at module load")
# mitmdump is long-lived: a path frozen at import would make the override
# untestable and a relocated registry unreadable until a proxy restart.
del os.environ["_PILOT_EGRESS_SENTINEL_DIR"]
late_home = Path(TMPROOT) / "late-home"
(late_home / ".mika" / "pilot-egress-sentinels").mkdir(parents=True, exist_ok=True)
(late_home / ".mika" / "pilot-egress-sentinels" / NONCE).write_text("", encoding="utf-8")
os.environ["HOME"] = str(late_home)
flow, _ = run(API, f"Bearer {SENTINEL}")
check("12.1 the default ~/.mika/pilot-egress-sentinels is read at call time",
      200, status_of(flow))
check("12.2 and the token file stays the one staged host-side",
      f"Bearer {HOST_TOKEN}", flow.request.headers.get("Authorization"))

print("")
print("CASE 13 — the purge throttle is per directory, not per process (mika#2751)")
# `_purge_registry` takes its directory as an ARGUMENT while the throttle used
# to be a single module-level float. So the first directory purged in a
# ten-minute window starved every other one for the rest of it, and which one
# got starved depended only on request order. One registry today, two as soon
# as a second one exists (the push-scope perimeter of mika#2751, read on the
# same request path) — and a registry left stale is one whose expired entries
# still answer.
PURGE_A = Path(TMPROOT) / "purge-a"
PURGE_B = Path(TMPROOT) / "purge-b"
AGED = "3333333333333333333333333333f00d"
stale = time.time() - 99 * 3600
for directory in (PURGE_A, PURGE_B):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / NONCE).write_text("", encoding="utf-8")   # so the call validates
    (directory / AGED).write_text("", encoding="utf-8")
    os.utime(directory / AGED, (stale, stale))
addon._last_purge = {}

os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(PURGE_A)
flow, _ = run(API, f"Bearer {SENTINEL}")
check("13.1 the call against the first registry is served", 200, status_of(flow))
check_true("13.2 its aged entry is purged", not (PURGE_A / AGED).exists())

# Same ten-minute window, a DIFFERENT directory. With a global throttle this is
# where the second registry stayed stale.
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(PURGE_B)
flow, _ = run(API, f"Bearer {SENTINEL}")
check("13.3 the call against the second registry is served", 200, status_of(flow))
check_true("13.4 the second registry is purged too, in the same window",
           not (PURGE_B / AGED).exists())

# And the throttle is still a throttle: a second pass on a directory already
# purged must NOT re-scan. Without this the fix could have been "purge always",
# which is a scandir per request and no longer the thing being asserted.
LATE = "4444444444444444444444444444beef"
(PURGE_B / LATE).write_text("", encoding="utf-8")
os.utime(PURGE_B / LATE, (stale, stale))
flow, _ = run(API, f"Bearer {SENTINEL}")
check("13.5 a second call on the same registry is served", 200, status_of(flow))
check_true("13.6 and its newly-aged entry survives (the throttle still throttles)",
           (PURGE_B / LATE).exists())
check_true("13.7 the two directories are throttled independently, by key",
           {str(PURGE_A), str(PURGE_B)} <= set(addon._last_purge))
addon._last_purge = {}
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(REGISTRY)

print("")
print("CASE 14 — the 72 h TTL of THIS registry is unchanged (mika#2766)")
# mika#2766 gave the perimeter registry of mika#2751 its own, much shorter TTL,
# and `_purge_registry` now takes the number as an argument instead of holding
# it. This is the neighbouring registry's suite, so this is the right place to
# assert nobody moved ITS number in passing: the realistic regression is not a
# rewritten constant but a swapped argument at one of the two call sites, and
# that reads as 72 h becoming 12 h here.
#
# 13 h is the age that separates the two: past the perimeter's 12 h, well inside
# this registry's 72 h. A sentinel of that age must still be here — a flight is
# re-read hours after it ran, and a nonce purged under it is an `api.github.com`
# 403 on a pilot that is simply slow.
BETWEEN_TTLS = "6666666666666666666666666666ab1e"
(REGISTRY / BETWEEN_TTLS).write_text("", encoding="utf-8")
aged = time.time() - 13 * 3600
os.utime(REGISTRY / BETWEEN_TTLS, (aged, aged))
addon._last_purge = {}
flow, _ = run(API, f"Bearer {SENTINEL}")
check("14.1 the call is served", 200, status_of(flow))
check_true("14.2 a 13 h sentinel is NOT purged — 72 h still governs here",
           (REGISTRY / BETWEEN_TTLS).exists())
flow, _ = run(API, f"Bearer {BARE}-{BETWEEN_TTLS}")
check("14.3 and it is still a valid credential", 200, status_of(flow))
check("14.4 the constant itself is unmoved", 72 * 3600, addon._REGISTRY_TTL_SECONDS)
# The purge still takes the TTL from its CALLER, so a reader of this suite can
# see that the two registries cannot share one number by construction.
check_true("14.5 and it is strictly longer than the perimeter registry's",
           addon._REGISTRY_TTL_SECONDS > addon._PUSH_SCOPE_TTL_SECONDS)
check_true("14.6 `_purge_registry` takes the TTL in, it does not choose it",
           "ttl_seconds" in addon._purge_registry.__code__.co_varnames[
               : addon._purge_registry.__code__.co_argcount])
(REGISTRY / BETWEEN_TTLS).unlink()
addon._last_purge = {}

print("")
print(f"TOTAL failures: {failures}")
sys.exit(1 if failures else 0)
PY
py_rc=$?
cat "$PY_OUT"

# Anti-vacuity, on the harness itself: a python block that dies on import, or
# whose case list silently shrank, either exits non-zero (caught by the rc
# assertion) or exits 0 having asserted almost nothing (caught only by this
# floor). Both must be caught — an `exit 0` after three assertions reads
# exactly like a clean tree (class mika#2205).
asserted=$(grep -c '^RESULT ' "$PY_OUT" || true)
rc=1; [ "${asserted:-0}" -ge 86 ] && rc=0
assert_eq "anti-vacuity: the python half really asserted ($asserted assertions >= 86)" "0" "$rc"
assert_eq "every addon-level assertion passes" "0" "$py_rc"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
