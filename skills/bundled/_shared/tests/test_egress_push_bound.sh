#!/bin/bash
# mika#2736: on github.com the egress addon BOUNDS what a `git-receive-pack`
# may touch — the repository, ref deletions, exotic refs, protected branches —
# and it never refuses the legitimate push of a pilot.
#
# WHY A BOUND AND NOT A SENTINEL. The grooming measured that no channel can
# carry a justification that attests the legitimate PATH rather than the
# sandbox: `http.extraHeader` via GIT_CONFIG_KEY_n works, and a second process
# in the same environment presents the very same valid header while pushing
# whatever it wants (§ 2.5 of the plan). But the same measurement showed the
# proxy does not need to be told anything — the repository is in the URL and
# the ref commands are in CLEAR at the head of the body (§ 2.2). A bound on
# what the proxy already SEES asks nothing of the client, so no process of the
# sandbox can get around it by presenting itself well.
#
# WHY THIS SUITE IS THE FLOOR OF THE TICKET (§ 6, commit 2). The bound is
# fail-closed and sits on the path of the autonomous loop. The two failure
# modes able to 403 every legitimate push were MEASURED at grooming and are
# pinned here by POSITIVE cases, not by prose:
#   * CASE 2 — git sends a 4-byte authentication probe (a bare flush-pkt, no
#     command) before streaming a large push. A guard demanding "at least one
#     command" would refuse it, hence refuse every push big enough to be
#     streamed. Nothing would say so before the first dispatch.
#   * CASE 3 — past the threshold the body goes out chunked, with the commands
#     still at its head. The verdict therefore lives in `request` (body
#     buffered) and not in `requestheaders` (body not there yet).
#
# WHY IT DRIVES THE REAL HOOKS AND NOT ONLY THE PURE FUNCTIONS. A green,
# unplugged verdict function is class mika#2205 — two states rendering the same
# bytes. Every case below runs `requestheaders` THEN `request`, in the order
# mitmproxy fires them, so a perfect `_push_verdict` that is never called
# reddens here, and CASE 1 additionally asserts that an ALLOWED push still
# carries the host credential (risk R6: the injection and the verdict live in
# two different hooks).
#
# WHAT IS NOT HERE, deliberately. mitmproxy is not a test dependency of this
# repo (measured: `import mitmproxy` fails on this host), so the real proxy is
# never launched and no bytes reach the forge. The contract under test is the
# addon's DECISION and its rewrite of the request, both observable on the
# stubbed flow. The in-flight half is the canary's three `push_bound_*` probes.
#
# Run: bash skills/bundled/_shared/tests/test_egress_push_bound.sh
# Expected: all assertions pass, exit 0. Needs no bwrap, no git, no mitmproxy
# and no network.

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

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2736-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT
export HOME="$TMPROOT/home"
mkdir -p "$HOME/.mika"

# The host-side token the addon injects. Deliberately recognisable: the
# disclosure case asserts it appears in NO refusal body and in NO DENY line.
printf 'ghp_hostTOKEN2736hostTOKEN2736hostTOK\n' > "$HOME/.mika/pilot-gh-token"
chmod 0600 "$HOME/.mika/pilot-gh-token"

# An inherited value would shadow the file and make the token unrecognisable;
# an inherited switch would silently disarm half the suite.
unset GH_TOKEN MIKA_GITHUB_TOKEN MIKA_PILOT_EGRESS_SENTINEL \
      MIKA_PILOT_EGRESS_PUSH_BOUND _PILOT_EGRESS_SENTINEL_DIR

echo ""
echo "The addon bounds what a git-receive-pack may touch (mika#2736)"
echo "---------------------------------------------------------------"

PY_OUT="$TMPROOT/python.out"
python3 - "$GH_ADDON" "$TMPROOT" >"$PY_OUT" 2>&1 <<'PY'
import base64
import importlib.util
import io
import os
import sys
import types
from contextlib import redirect_stderr
from pathlib import Path

ADDON_PATH, TMPROOT = sys.argv[1], sys.argv[2]

HOST_TOKEN = "ghp_hostTOKEN2736hostTOKEN2736hostTOK"
BARE = "proxy-managed-no-secret"
NONCE = "a1b2c3d4e5f60718293a4b5c6d7e8f90"
SENTINEL = f"{BARE}-{NONCE}"
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
    writes `Authorization`. A case-sensitive stub would pass an addon that
    strips nothing.
    """

    def __init__(self):
        self._d = {}

    def get(self, name, default=None):
        hit = self._d.get(name.lower())
        return hit if hit is not None else default

    def __setitem__(self, name, value):
        self._d[name.lower()] = value

    def pop(self, name, default=None):
        return self._d.pop(name.lower(), default)

    def __contains__(self, name):
        return name.lower() in self._d


class Request:
    """A request whose BODY is observable — the addition this suite needs.

    `content` is a plain attribute so a case can set it to None, which is what
    mitmproxy really yields when `stream_large_bodies` made it stream the body
    (§ 2.3 c of the plan). `RaisingRequest` below covers the other shape of the
    same unavailability.
    """

    def __init__(self, host, method, path, headers, content=b""):
        self.host, self.method, self.path = host, method, path
        self.headers, self.content = headers, content


class RaisingRequest(Request):
    """`.content` raises, as mitmproxy's decoder can on a broken encoding.

    A reader that does not catch it would be a 500 on the loop's push path —
    louder than the 403 we chose, and undiagnosable.
    """

    @property
    def content(self):
        raise ValueError("content not available (streamed)")

    @content.setter
    def content(self, _value):
        pass


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

_spec = importlib.util.spec_from_file_location("mika_addon_2736", ADDON_PATH)
addon = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(addon)

# ------------------------------------------------------------------- fixtures
# The sentinel registry exists and attests one nonce, so nothing in this suite
# is ever refused for a vol-2 reason by accident: a `sentinel-*` motif showing
# up on an api.github.com case would be a different defect from a `push-*` one.
REGISTRY = Path(TMPROOT) / "registry"
REGISTRY.mkdir(parents=True, exist_ok=True)
(REGISTRY / NONCE).write_text("", encoding="utf-8")
os.environ["_PILOT_EGRESS_SENTINEL_DIR"] = str(REGISTRY)

ZERO = b"0" * 40
SHA_A = b"232d30b1" + b"a" * 32
SHA_B = b"e7d978a8" + b"b" * 32
FLUSH = b"0000"

PUSH_PATH = "/senara-solutions/mika.git/git-receive-pack"
FOREIGN_PATH = "/not-senara-solutions/mika.git/git-receive-pack"
FETCH_PATH = "/senara-solutions/mika.git/git-upload-pack"
INFO_REFS = "/senara-solutions/mika.git/info/refs?service=git-receive-pack"
BRANCH = b"refs/heads/feat/2736/egress-push-bound"

EXPECTED_BASIC = "Basic " + base64.b64encode(
    f"x-access-token:{HOST_TOKEN}".encode()).decode("ascii")


def pkt(payload: bytes) -> bytes:
    """One pkt-line: 4 hex digits of TOTAL length, then the payload."""
    return b"%04x" % (len(payload) + 4) + payload


def cmd(old: bytes, new: bytes, ref: bytes, caps: bytes = b"report-status") -> bytes:
    """A ref command exactly as `git push` lays it out (measured, § 2.2)."""
    return pkt(old + b" " + new + b" " + ref + b"\x00" + caps + b"\n")


def body(*commands: bytes, pack: bytes = b"PACK\x00\x00\x00\x02") -> bytes:
    return b"".join(commands) + FLUSH + pack


def run(host, path, content=b"", cred=None, method="POST", raising=False):
    """Drive BOTH real hooks, in the order mitmproxy fires them.

    `requestheaders` injects, `request` judges. Running only the second would
    pass an addon whose verdict is right and whose injection vanished (R6), and
    running only the first would pass an addon whose bound is never wired in.
    """
    headers = Headers()
    if cred is not None:
        headers["Authorization"] = cred
    cls = RaisingRequest if raising else Request
    flow = HTTPFlow(cls(host, method, path, headers, content))
    err = io.StringIO()
    with redirect_stderr(err):
        addon.requestheaders(flow)
        addon.request(flow)
    return flow, err.getvalue()


def status_of(flow):
    return flow.response.status_code if flow.response else 200


def body_of(flow):
    return flow.response.content.decode("utf-8", "replace") if flow.response else ""


def ctype_of(flow):
    # Never dereference a missing response: a red case must stay ONE red line,
    # not a traceback that takes every later case with it.
    return flow.response.headers.get("Content-Type") if flow.response else None


def denies_in(err):
    """The refusal lines of a capture, degradation lines excluded.

    Until mika#2751 "nothing was refused" was asserted as `err.strip() == ""`,
    because a refusal was the only thing the addon ever wrote. The declared-
    perimeter tightening legitimately prints ONE `DEGRADE scope-…` line on an
    allowed push when no perimeter is declared -- it is the operator probe of
    § 3.6, and the canary asserts it in flight -- so the proxy stops standing
    in for the predicate and the predicate is written out: a refusal is a DENY
    line, and nothing else is. The degradation line itself is asserted by the
    cases that own it, never merely tolerated here.
    """
    return [line for line in err.splitlines() if " DENY " in line]


def degrades_in(err):
    """The degradation lines of a capture — the other half of `denies_in`."""
    return [line for line in err.splitlines() if " DEGRADE " in line]


def noise_in(err):
    """Lines that are neither a refusal nor a degradation.

    Keeps the three assertions below as strong as the `err.strip() == ""` they
    replace: an unexpected third kind of line still reddens.
    """
    return [
        line for line in err.splitlines()
        if line.strip() and " DENY " not in line and " DEGRADE " not in line
    ]


print("")
print("CASE 1 — the nominal push of a pilot passes, credential injected")
# The anti-vacuity of every other case: a bound that refused EVERYTHING would
# read green on cases 4-9 and redden only here.
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("1.1 not refused", 200, status_of(flow))
check("1.2 the HOST credential goes out, Basic, as before this ticket",
      EXPECTED_BASIC, flow.request.headers.get("Authorization"))
check("1.3 no DENY line", [], denies_in(err))
check("1.3b and no line of a third kind either", [], noise_in(err))
# A non-create (old-sha real) is the ordinary case after the first push.
flow, err = run(GIT, PUSH_PATH, body(cmd(SHA_A, SHA_B, BRANCH)))
check("1.4 an update (both shas real) passes too", 200, status_of(flow))
check("1.5 and still carries the host credential", EXPECTED_BASIC,
      flow.request.headers.get("Authorization"))
# The capability list varies (`side-band-64k`, `agent=…`, `atomic`); the bound
# must read the refname and not the capabilities.
flow, _ = run(GIT, PUSH_PATH,
              body(cmd(ZERO, SHA_A, BRANCH,
                       b"report-status side-band-64k agent=git/2.43.0")))
check("1.6 a longer capability list changes nothing", 200, status_of(flow))
# The `.git` suffix is optional on the smart-HTTP path.
flow, _ = run(GIT, "/senara-solutions/mika/git-receive-pack",
              body(cmd(ZERO, SHA_A, BRANCH)))
check("1.7 the repo path without the .git suffix is the same repo",
      200, status_of(flow))

print("")
print("CASE 2 — the 4-byte authentication probe passes (bare flush-pkt)")
# MEASURED: a large push emits TWO POSTs and the first carries NO command —
# 4 bytes, a naked flush-pkt. Confusing "no command" with "unreadable" would
# 403 every streamed push (§ 2.3 a). An empty list and None are two states.
flow, err = run(GIT, PUSH_PATH, FLUSH)
check("2.1 the bare flush-pkt is allowed", 200, status_of(flow))
check("2.2 and the credential is injected on it", EXPECTED_BASIC,
      flow.request.headers.get("Authorization"))
check("2.3 no DENY line for the probe", [], denies_in(err))
check("2.3b and no line of a third kind either", [], noise_in(err))
flow, _ = run(GIT, PUSH_PATH, b"")
check("2.4 a zero-length body is not a refusal either", 200, status_of(flow))

print("")
print("CASE 3 — a large chunked body passes, commands read at its head")
# Past the streaming threshold git switches to Transfer-Encoding: chunked but
# the commands stay in front. This is why the verdict lives in `request`.
big = body(cmd(ZERO, SHA_B, b"refs/heads/feat/9999/big"),
           pack=b"PACK\x00\x00\x00\x02" + os.urandom(400_000))
flow, err = run(GIT, PUSH_PATH, big)
check("3.1 a 400 KiB body with valid commands passes", 200, status_of(flow))
check("3.2 no DENY line", [], denies_in(err))
check("3.2b and no line of a third kind either", [], noise_in(err))
check_true("3.3 the case is non-vacuous (the body really is large)",
           len(big) > 300_000)

print("")
print("CASE 4 — a repository outside the organisation: 403 push-repo-foreign")
flow, err = run(GIT, FOREIGN_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("4.1 refused", 403, status_of(flow))
check_true("4.2 the body names the reason in clear",
           "push-repo-foreign" in body_of(flow))
check_true("4.3 the body names the repository in cause (client data, not a secret)",
           "not-senara-solutions/mika" in body_of(flow))
check_true("4.4 the body names the ticket", "mika#2736" in body_of(flow))
check_true("4.5 the body names the operator switch",
           "MIKA_PILOT_EGRESS_PUSH_BOUND" in body_of(flow))
check("4.6 the refusal is text/plain", "text/plain; charset=utf-8", ctype_of(flow))
check("4.7 one greppable DENY line, vol-2 vocabulary",
      f"[github-proxy] DENY push-repo-foreign POST {FOREIGN_PATH}", err.strip())
check_true("4.8 NO host credential reaches a refused request",
           flow.request.headers.get("Authorization") is None)
foreign_body = body_of(flow)

print("")
print("CASE 5 — a ref deletion: 403 push-ref-delete")
flow, err = run(GIT, PUSH_PATH, body(cmd(SHA_A, ZERO, b"refs/heads/main")))
check("5.1 refused", 403, status_of(flow))
check_true("5.2 the body names THIS reason", "push-ref-delete" in body_of(flow))
check_true("5.3 and not the protected-branch one",
           "push-ref-protected" not in body_of(flow))
check_true("5.4 the body names the ref in cause", "refs/heads/main" in body_of(flow))
check("5.5 the DENY line names this reason",
      f"[github-proxy] DENY push-ref-delete POST {PUSH_PATH}", err.strip())
delete_body = body_of(flow)
# A deletion is read off `new-sha` all zeros — the form the protocol imposes,
# no heuristic. A 64-char all-zero sha (sha256 repositories) is the same state.
flow, _ = run(GIT, PUSH_PATH, body(cmd(b"1" * 64, b"0" * 64, BRANCH)))
check("5.6 a sha256 all-zero new-sha is a deletion too", 403, status_of(flow))
check_true("5.7 and names the deletion reason", "push-ref-delete" in body_of(flow))

print("")
print("CASE 6 — a protected integration branch: 403 push-ref-protected")
for branch in (b"refs/heads/main", b"refs/heads/master"):
    flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, branch)))
    check(f"6.x {branch.decode()} refused", 403, status_of(flow))
    check_true(f"6.x {branch.decode()} names push-ref-protected",
               "push-ref-protected" in body_of(flow))
    check_true(f"6.x {branch.decode()} is named in the body",
               branch.decode() in body_of(flow))
protected_body = body_of(flow)
# Near misses must NOT be caught: a dispatch branch may legitimately contain
# the word. A bound that refused these would refuse real pilot work.
for branch in (b"refs/heads/feat/2736/main-bound", b"refs/heads/mainline",
               b"refs/heads/main/sub"):
    flow, _ = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, branch)))
    check(f"6.y {branch.decode()} is NOT protected (whole-name match)",
          200, status_of(flow))

print("")
print("CASE 7 — a refname outside refs/heads/: 403 push-ref-exotic")
for ref in (b"refs/tags/v1", b"refs/dispatch-gate/feat/2736/x",
            b"refs/notes/commits", b"HEAD"):
    flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, ref)))
    check(f"7.x {ref.decode()} refused", 403, status_of(flow))
    check_true(f"7.x {ref.decode()} names push-ref-exotic",
               "push-ref-exotic" in body_of(flow))
exotic_body = body_of(flow)

print("")
print("CASE 8 — several commands, ONE violating: refused, and the body names it")
# A bound that judged only the first command is defeated by a harmless one in
# front. The faulty ref — and not the harmless one — must be the one named.
flow, err = run(GIT, PUSH_PATH, body(
    cmd(ZERO, SHA_A, BRANCH),
    cmd(ZERO, SHA_B, b"refs/heads/feat/2736/second-ok"),
    cmd(ZERO, SHA_B, b"refs/tags/sneaky-2736"),
))
check("8.1 refused although the first two commands are fine", 403, status_of(flow))
check_true("8.2 the faulty ref is named", "refs/tags/sneaky-2736" in body_of(flow))
check_true("8.3 the harmless leading ref is NOT named",
           BRANCH.decode() not in body_of(flow))
check("8.4 the DENY line names the exotic reason",
      f"[github-proxy] DENY push-ref-exotic POST {PUSH_PATH}", err.strip())
# Several commands, all legitimate: the loop must not refuse on count.
flow, _ = run(GIT, PUSH_PATH, body(
    cmd(ZERO, SHA_A, BRANCH),
    cmd(ZERO, SHA_B, b"refs/heads/feat/2736/second-ok"),
))
check("8.5 several legitimate commands pass", 200, status_of(flow))

print("")
print("CASE 9 — an unreadable body refuses, and the dedicated switch disarms it")
# The arbitrage of § 3.3, and it leans the OTHER way from vol 2's: allowing on
# unreadability hands anyone a trivial bypass (grow the body past 10 MB), and a
# pilot pack is of the order of a kilobyte. Made survivable by a switch of its
# own, because the two guards have different failure modes.
for label, kwargs in (
    ("content is None (mitmproxy streamed it)", {"content": None}),
    ("content access raises", {"raising": True}),
):
    flow, err = run(GIT, PUSH_PATH, **kwargs)
    check(f"9.x refused when {label}", 403, status_of(flow))
    check_true(f"9.x names push-body-unreadable ({label})",
               "push-body-unreadable" in body_of(flow))
    check("9.x the DENY line names it",
          f"[github-proxy] DENY push-body-unreadable POST {PUSH_PATH}", err.strip())
# A malformed command section is the same motif: refuse with a named reason
# rather than allow by default (risk R2).
for label, malformed in (
    ("a non-hex length prefix", b"zzzz" + b"junk" + FLUSH),
    ("a length that runs past the body", b"00ff" + b"short"),
    ("a command with two fields instead of three",
     pkt(b"shallow " + SHA_A) + FLUSH),
    ("a command section with no flush-pkt", cmd(ZERO, SHA_A, BRANCH)),
):
    flow, _ = run(GIT, PUSH_PATH, malformed)
    check(f"9.y {label} refuses as unreadable", 403, status_of(flow))
    check_true(f"9.y and names the motif ({label})",
               "push-body-unreadable" in body_of(flow))
opaque_body = body_of(flow)
# The rollback: this bound ALONE, without touching vol 2's cut.
for value in ("0", "FALSE", "no", "Off", "disabled"):
    os.environ["MIKA_PILOT_EGRESS_PUSH_BOUND"] = value
    flow, err = run(GIT, PUSH_PATH, content=None)
    check(f"9.z `{value}` disarms the bound (unreadable body passes)",
          200, status_of(flow))
    check(f"9.z `{value}`: the credential is injected as on origin/main",
          EXPECTED_BASIC, flow.request.headers.get("Authorization"))
    check(f"9.z `{value}`: no DENY line", "", err.strip())
os.environ["MIKA_PILOT_EGRESS_PUSH_BOUND"] = "0"
flow, _ = run(GIT, FOREIGN_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("9.4 disarmed, a foreign repo passes too (the whole bound is off)",
      200, status_of(flow))
flow, _ = run(API, "/graphql", cred=f"Bearer {SENTINEL}", method="POST")
check("9.5 the push switch does NOT disarm vol 2 on api.github.com",
      f"Bearer {HOST_TOKEN}", flow.request.headers.get("Authorization"))
flow, _ = run(API, "/graphql", method="POST")
check("9.6 and vol 2 still refuses an absent sentinel while push is disarmed",
      403, status_of(flow))
check_true("9.7 with ITS own motif, not a push one",
           "sentinel-absent" in body_of(flow))
for value in ("1", "", "yes", "garbage"):
    os.environ["MIKA_PILOT_EGRESS_PUSH_BOUND"] = value
    flow, _ = run(GIT, PUSH_PATH, content=None)
    check(f"9.w `{value}` leaves the bound ARMED (default is armed)",
          403, status_of(flow))
del os.environ["MIKA_PILOT_EGRESS_PUSH_BOUND"]

print("")
print("CASE 10 — the two halves already shipped are strictly unchanged")
# Non-regression. The bound bears on the only verb that WRITES: a fetch reads
# public data, destroys nothing and exfiltrates nothing.
for label, path, content in (
    ("git-upload-pack (the fetch)", FETCH_PATH, b"0032want " + SHA_A + b"\n" + FLUSH),
    ("info/refs?service=git-receive-pack", INFO_REFS, b""),
    ("a non-git path", "/senara-solutions/mika/pull/2736", b""),
):
    flow, err = run(GIT, path, content, method="GET")
    check(f"10.x {label}: never refused", 200, status_of(flow))
    check(f"10.x {label}: the Basic injection is unchanged", EXPECTED_BASIC,
          flow.request.headers.get("Authorization"))
    check(f"10.x {label}: no DENY line", "", err.strip())
# An unreadable body on a NON-push path must not be judged either: the
# fail-closed arbitrage is bounded to git-receive-pack.
flow, err = run(GIT, FETCH_PATH, content=None, method="POST")
check("10.4 an unreadable body on the fetch path is not refused",
      200, status_of(flow))
check("10.5 and logs no refusal", "", err.strip())
# api.github.com keeps vol 2 entirely: the cut lives in `requestheaders` and
# nothing here touches it.
flow, _ = run(API, "/rate_limit", cred=f"token {SENTINEL}", method="GET")
check("10.6 api.github.com with a registered sentinel: still accepted",
      200, status_of(flow))
check("10.7 and the Bearer form of the host credential is injected",
      f"Bearer {HOST_TOKEN}", flow.request.headers.get("Authorization"))
flow, err = run(API, "/graphql", method="POST")
check("10.8 api.github.com with nothing presented: still 403", 403, status_of(flow))
check_true("10.9 with vol 2's own motif", "sentinel-absent" in body_of(flow))
check("10.10 and exactly ONE DENY line — the push hook must not double-log",
      "[github-proxy] DENY sentinel-absent POST /graphql", err.strip())
# Same flow shape, but the path LOOKS like a push. api.github.com has no
# receive-pack endpoint; the bound must not invent one there.
flow, err = run(API, PUSH_PATH, body(cmd(ZERO, SHA_A, b"refs/tags/v1")),
                cred=f"token {SENTINEL}")
check("10.11 a receive-pack-shaped path on api.github.com is not bound",
      200, status_of(flow))
check("10.12 and logs no push refusal", "", err.strip())

print("")
print("CASE 11 — the five refusal bodies are distinct two by two")
# Without this assertion, "five motifs" is a claim of the plan and not a
# property of the code (class mika#2205).
bodies = {
    "push-repo-foreign": foreign_body,
    "push-ref-delete": delete_body,
    "push-ref-protected": protected_body,
    "push-ref-exotic": exotic_body,
    "push-body-unreadable": opaque_body,
}
check("11.1 five bodies were really captured", 5, len(bodies))
names = sorted(bodies)
for i, left in enumerate(names):
    for right in names[i + 1:]:
        check_true(f"11.x {left} != {right}", bodies[left] != bodies[right])
for name, text in bodies.items():
    check_true(f"11.y {name} names itself", name in text)
    check_true(f"11.y {name} carries a remedy, not just a label", len(text) >= 80)
    check_true(f"11.y {name} names no other motif",
               sum(1 for other in bodies if other in text) == 1)

print("")
print("CASE 12 — a refusal names, and never discloses (mika#1949)")
import hashlib  # noqa: E402 — local to this case, as is the doctrine it serves
flow, err = run(GIT, FOREIGN_PATH, body(cmd(ZERO, SHA_A, b"refs/tags/v1")),
                cred=f"Bearer {SENTINEL}")
combined = err + body_of(flow)
check("12.1 refused, so there IS something to inspect", 403, status_of(flow))
check_true("12.2 the capture is non-vacuous (the DENY line is really here)",
           "[github-proxy] DENY push-repo-foreign" in err)
check_true("12.3 the host token appears nowhere", HOST_TOKEN not in combined)
check_true("12.4 the presented value appears nowhere", SENTINEL not in combined)
check_true("12.5 nor its nonce alone", NONCE not in combined)
for algo in ("md5", "sha1", "sha256"):
    for label, secret in (("host token", HOST_TOKEN), ("presented value", SENTINEL)):
        digest = hashlib.new(algo, secret.encode()).hexdigest()
        check_true(f"12.x no {algo} fingerprint of the {label}",
                   digest not in combined and digest[:12] not in combined)

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
rc=1; [ "${asserted:-0}" -ge 110 ] && rc=0
assert_eq "anti-vacuity: the python half really asserted ($asserted assertions >= 110)" "0" "$rc"
assert_eq "every addon-level assertion passes" "0" "$py_rc"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
