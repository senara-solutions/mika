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
      MIKA_PILOT_EGRESS_PUSH_BOUND _PILOT_EGRESS_SENTINEL_DIR \
      MIKA_PILOT_EGRESS_PUSH_SCOPE _PILOT_EGRESS_PUSH_SCOPE_DIR

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
import time
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

# The declared perimeter of mika#2751. Deliberately NOT exported yet: the
# twelve cases of vol 1 run with no perimeter at all, which is the degradation
# path, and CASE 13 asserts that this is what keeps them meaning what they
# meant. Each later case arms the override itself.
SCOPES = Path(TMPROOT) / "push-scopes"
SCOPE_DIR_ENV = "_PILOT_EGRESS_PUSH_SCOPE_DIR"
SCOPE_SWITCH = "MIKA_PILOT_EGRESS_PUSH_SCOPE"
SCOPE_ENTRY_A = "1" * 32
SCOPE_ENTRY_B = "2" * 32
SCOPE_ENTRY_C = "3" * 32

PUSH_PATH = "/senara-solutions/mika.git/git-receive-pack"
SIBLING_PATH = "/senara-solutions/mika-platform.git/git-receive-pack"
THIRD_PATH = "/senara-solutions/mika-cloud.git/git-receive-pack"
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


def declare_scopes(*entries):
    """Arm the perimeter registry with exactly these `(name, text)` entries.

    Rebuilt from scratch each time: a residual entry can only ever WIDEN the
    union, so a leak between cases would make a later refusal unreachable and
    read as a pass. The override is set here rather than once at the top so the
    absent-registry state stays testable (CASE 13).
    """
    SCOPES.mkdir(parents=True, exist_ok=True)
    for child in SCOPES.iterdir():
        child.chmod(0o600)
        child.unlink()
    for name, text in entries:
        (SCOPES / name).write_text(text, encoding="utf-8")
    os.environ[SCOPE_DIR_ENV] = str(SCOPES)


def declare_repos(*repos):
    """One entry per repository, the shape the launcher writes."""
    names = (SCOPE_ENTRY_A, SCOPE_ENTRY_B, SCOPE_ENTRY_C)
    declare_scopes(*[
        (name, f"repo=senara-solutions/{repo}\n")
        for name, repo in zip(names, repos)
    ])


def forget_scopes():
    """Back to no perimeter at all — the state the twelve vol-1 cases run in."""
    os.environ.pop(SCOPE_DIR_ENV, None)


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

# ===========================================================================
# mika#2751 — the DECLARED perimeter, a tightening strictly UNDER the bound
#
# The seven cases below are numbered 13 to 19. The plan calls them 14 to 20,
# having counted thirteen existing cases where the file carries twelve; the
# labels are renumbered rather than a gap left at 13, because sequential case
# numbers are the property a reader relies on.
#
# They are the FLOOR of this flight (§ 6): the tightening is a refusal on the
# push path of the autonomous loop, and the only failure mode able to STOP that
# loop — 403 on the 4-byte authentication probe, hence on every push large
# enough to be streamed — is pinned by CASE 18 as a POSITIVE case.
# ===========================================================================

print("")
print("CASE 13 — no perimeter declared: exactly mika#2736, and it says which")
# Point 2 of the ticket, asserted rather than promised. This is also WHY the
# twelve cases above still mean what they meant: none of them declares a
# perimeter, so all of them degrade.
forget_scopes()
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("13.1 the nominal push still passes", 200, status_of(flow))
check("13.2 and still carries the HOST credential", EXPECTED_BASIC,
      flow.request.headers.get("Authorization"))
check("13.3 nothing is refused", [], denies_in(err))
check("13.4 one degradation line, naming the absent perimeter",
      [f"[github-proxy] DEGRADE scope-absent POST {PUSH_PATH}"],
      degrades_in(err))
# An ABSENT directory is `scope-absent` and not `scope-unreadable`: the
# launcher creates it with the first perimeter it stages, so its absence means
# nothing was declared. Calling it unreadable would send an operator looking at
# permissions for a launcher that never ran (mika#2205).
check_true("13.5 the operator probe of § 3.6 really greps",
           "scope-absent" in err)
# And the static bound still BITES underneath — the whole point of the
# subordination. No path turns a mika#2736 refusal into an allow.
flow, err = run(GIT, PUSH_PATH, body(cmd(SHA_A, SHA_B, b"refs/heads/main")))
check("13.6 refs/heads/main is still refused with no perimeter",
      403, status_of(flow))
check_true("13.7 by the STATIC motif", "push-ref-protected" in body_of(flow))
check("13.8 and the refusal is logged, not the degradation",
      [], degrades_in(err))
flow, _ = run(GIT, FOREIGN_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("13.9 a foreign organisation is still refused with no perimeter",
      403, status_of(flow))
check_true("13.10 by the STATIC motif", "push-repo-foreign" in body_of(flow))

print("")
print("CASE 14 — a declared perimeter lets ITS repository through")
# The anti-vacuity of every case below: a tightening that refused everything
# would read green on 15, 16 and 17 and redden only here.
declare_repos("mika")
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("14.1 the declared repository passes", 200, status_of(flow))
check("14.2 and the outgoing Authorization is the HOST credential",
      EXPECTED_BASIC, flow.request.headers.get("Authorization"))
check("14.3 nothing is refused", [], denies_in(err))
check("14.4 and nothing degrades — the tightening really applied",
      [], degrades_in(err))
# GitHub is case-insensitive on `owner/repo`; a case-sensitive comparison here
# would refuse a legitimate push on one capital letter.
flow, _ = run(GIT, "/Senara-Solutions/MIKA.git/git-receive-pack",
              body(cmd(ZERO, SHA_A, BRANCH)))
check("14.5 the comparison is case-insensitive, as GitHub is",
      200, status_of(flow))
flow, _ = run(GIT, "/senara-solutions/mika/git-receive-pack",
              body(cmd(ZERO, SHA_A, BRANCH)))
check("14.6 the path without the .git suffix is the same repository",
      200, status_of(flow))
# A declared perimeter does not loosen the static bound either.
flow, _ = run(GIT, PUSH_PATH, body(cmd(SHA_A, SHA_B, b"refs/heads/main")))
check("14.7 in-perimeter, refs/heads/main is STILL refused", 403, status_of(flow))
check_true("14.8 by the static motif", "push-ref-protected" in body_of(flow))

print("")
print("CASE 15 — a sibling repository of the organisation: 403 push-scope-repo")
# The second bullet of the ticket, closed: mika#2736 let this through, because
# `senara-solutions/mika-platform` is inside the organisation.
declare_repos("mika")
flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("15.1 refused", 403, status_of(flow))
scope_body = body_of(flow)
check_true("15.2 the body names the motif in clear",
           "push-scope-repo" in scope_body)
check_true("15.3 it names the repository AIMED AT (client data, not a secret)",
           "senara-solutions/mika-platform" in scope_body)
check_true("15.4 it names the flight that decided the refusal",
           "mika#2751" in scope_body)
check_true("15.5 it names the rollback switch, and the one it does NOT disarm",
           SCOPE_SWITCH in scope_body and "MIKA_PILOT_EGRESS_PUSH_BOUND" in scope_body)
check_true("15.6 it names the registry to list, host-side",
           "pilot-push-scopes" in scope_body)
check("15.7 exactly one DENY line, naming motif, method and path",
      [f"[github-proxy] DENY push-scope-repo POST {SIBLING_PATH}"],
      denies_in(err))
check("15.8 and no degradation line alongside it", [], degrades_in(err))
check("15.9 the refusal is a text/plain body like the five others",
      "text/plain; charset=utf-8", ctype_of(flow))
check_true("15.10 the host credential is stripped from the refused request",
           flow.request.headers.get("Authorization") is None)
# The sixth body, distinct from the five of mika#2736 and naming no other
# motif (class mika#2205 — a shared body makes the difference undiagnosable at
# the moment it decides the remedy). Asserted here rather than in CASE 11,
# which runs before this body exists.
for other_name, other in (("push-repo-foreign", foreign_body),
                          ("push-ref-delete", delete_body),
                          ("push-ref-protected", protected_body),
                          ("push-ref-exotic", exotic_body),
                          ("push-body-unreadable", opaque_body)):
    check_true(f"15.x push-scope-repo != {other_name}", scope_body != other)
    check_true(f"15.x push-scope-repo does not name {other_name}",
               other_name not in scope_body)
    check_true(f"15.x {other_name} does not name push-scope-repo",
               "push-scope-repo" not in other)
check_true("15.11 it carries a remedy, not just a label", len(scope_body) >= 80)
# mika#1949: naming the TARGET is not disclosure; naming the presented
# justification, or any digest of it, would be.
flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)),
                cred=f"Bearer {SENTINEL}")
combined = err + body_of(flow)
check_true("15.12 the host token appears nowhere", HOST_TOKEN not in combined)
check_true("15.13 the presented value appears nowhere", SENTINEL not in combined)
check_true("15.14 nor its nonce alone", NONCE not in combined)
for algo in ("md5", "sha1", "sha256"):
    for label, secret in (("host token", HOST_TOKEN), ("presented value", SENTINEL)):
        digest = hashlib.new(algo, secret.encode()).hexdigest()
        check_true(f"15.x no {algo} fingerprint of the {label}",
                   digest not in combined and digest[:12] not in combined)

print("")
print("CASE 16 — two live entries: each of the two passes, a third is refused")
# The UNION of § 2.2, and it is imposed rather than chosen: one shared unix
# socket and no credential on github.com make a request unattributable to a
# spawn, so the verdict is "does SOME live entry declare this repository?".
# Without this case two concurrent pilots would be a regression found in
# flight.
declare_repos("mika", "mika-platform")
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("16.1 the first declared repository passes", 200, status_of(flow))
check("16.2 with nothing refused and nothing degraded",
      ([], []), (denies_in(err), degrades_in(err)))
flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("16.3 the second declared repository passes too", 200, status_of(flow))
check("16.4 with nothing refused either", [], denies_in(err))
flow, err = run(GIT, THIRD_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("16.5 a third, undeclared one is refused", 403, status_of(flow))
check_true("16.6 with the scope motif, naming IT",
           "push-scope-repo" in body_of(flow)
           and "senara-solutions/mika-cloud" in body_of(flow))
# One entry may carry several keys, and a future flight adds `ref=`: unknown
# keys are IGNORED on read, which is what makes that addition pure.
declare_scopes((SCOPE_ENTRY_A,
                "ref=refs/heads/feat/2751/whatever\n"
                "repo=senara-solutions/mika\n"
                "future-key=whatever the next flight writes\n"))
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("16.7 unknown keys are ignored, the repo= line still authorises",
      200, status_of(flow))
check("16.8 and the entry is not treated as unreadable", [], degrades_in(err))

print("")
print("CASE 17 — the two degradation causes, and the entry ignored in isolation")
# Readable registry, no `repo=` anywhere: nothing was DECLARED. Different cause
# from a registry that cannot be read, hence a different token and a different
# remedy (mika#2205).
declare_scopes((SCOPE_ENTRY_A, "ref=refs/heads/feat/2751/only-a-ref\n"))
flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("17.1 a registry declaring no repository degrades, never refuses",
      200, status_of(flow))
check("17.2 and names scope-absent",
      [f"[github-proxy] DEGRADE scope-absent POST {SIBLING_PATH}"],
      degrades_in(err))
flow, _ = run(GIT, PUSH_PATH, body(cmd(SHA_A, SHA_B, b"refs/heads/main")))
check("17.3 and the static bound still bites underneath", 403, status_of(flow))
check_true("17.4 with the static motif", "push-ref-protected" in body_of(flow))
# A path that exists and cannot be scanned IS host bookkeeping: a regular file
# where a directory belongs. Chosen over a chmod so the case means the same
# thing as root.
NOT_A_SCOPE_DIR = Path(TMPROOT) / "push-scopes-is-a-file"
NOT_A_SCOPE_DIR.write_text("", encoding="utf-8")
os.environ[SCOPE_DIR_ENV] = str(NOT_A_SCOPE_DIR)
flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("17.5 an unreadable registry degrades, never refuses", 200, status_of(flow))
check("17.6 and names scope-unreadable — the OTHER cause, the other remedy",
      [f"[github-proxy] DEGRADE scope-unreadable POST {SIBLING_PATH}"],
      degrades_in(err))
check_true("17.7 the two tokens are really distinct",
           addon._DEGRADE_SCOPE_ABSENT != addon._DEGRADE_SCOPE_UNREADABLE)
# An entry that is individually unreadable is SKIPPED: one spawn's bookkeeping
# going bad must neither refuse its neighbours nor widen the union.
declare_repos("mika")
(SCOPES / SCOPE_ENTRY_B).write_text("repo=senara-solutions/mika-platform\n",
                                    encoding="utf-8")
(SCOPES / SCOPE_ENTRY_B).chmod(0o000)
if os.geteuid() == 0:
    print("RESULT PASS 17.8 skipped as root (mode 000 is not enforced)")
    print("RESULT PASS 17.9 skipped as root (mode 000 is not enforced)")
    print("RESULT PASS 17.10 skipped as root (mode 000 is not enforced)")
else:
    flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
    check("17.8 a healthy neighbour entry still authorises", 200, status_of(flow))
    check("17.9 and the unreadable entry triggers no refusal", [], denies_in(err))
    flow, _ = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
    check("17.10 while what only the unreadable entry declared is NOT allowed",
          403, status_of(flow))
(SCOPES / SCOPE_ENTRY_B).chmod(0o600)

print("")
print("CASE 18 — the 4-byte probe and the chunked push stay POSITIVE, armed")
# Point 4 of the ticket, and the only failure mode able to stop the autonomous
# loop: a guard that 403s the probe POST refuses every push large enough to be
# streamed, and nothing would say so before the first dispatch. Structurally
# out of reach — the tightening judges the REPOSITORY, which comes from the
# URL and not from the body, so it cannot tell a probe from a push — but
# "structurally impossible" is a claim of the plan, not a property of the code
# (risk R2).
declare_repos("mika")
flow, err = run(GIT, PUSH_PATH, FLUSH)
check("18.1 the bare flush-pkt passes with the tightening armed",
      200, status_of(flow))
check("18.2 and carries the host credential", EXPECTED_BASIC,
      flow.request.headers.get("Authorization"))
check("18.3 with no refusal and no degradation",
      ([], []), (denies_in(err), degrades_in(err)))
flow, err = run(GIT, PUSH_PATH, b"")
check("18.4 a zero-length body is not a refusal either", 200, status_of(flow))
check("18.5 and logs no refusal", [], denies_in(err))
big_armed = body(cmd(ZERO, SHA_B, b"refs/heads/feat/2751/big"),
                 pack=b"PACK\x00\x00\x00\x02" + os.urandom(400_000))
flow, err = run(GIT, PUSH_PATH, big_armed)
check("18.6 a 400 KiB chunked push passes with the tightening armed",
      200, status_of(flow))
check("18.7 with no refusal", [], denies_in(err))
check_true("18.8 the case is non-vacuous (the body really is large)",
           len(big_armed) > 300_000)
# And the probe to an UNDECLARED repository is still refused: the positive
# cases above must not be the tightening giving up on probes altogether.
flow, _ = run(GIT, SIBLING_PATH, FLUSH)
check("18.9 the same probe to an undeclared repository IS refused",
      403, status_of(flow))
check_true("18.10 with the scope motif", "push-scope-repo" in body_of(flow))

print("")
print("CASE 19 — the switch matrix, and the subordination AS A DECISION")
# | PUSH_BOUND | PUSH_SCOPE | expected                                     |
# |------------|------------|----------------------------------------------|
# |     1      |     1      | static + tightening                          |
# |     1      |     0      | EXACTLY mika#2736 — the static bound alone   |
# |     0      |     1      | nothing: PUSH_BOUND is the master breaker    |
# |     0      |     0      | nothing                                      |
#
# THE (0, 1) ROW IS A DECISION, not a defect of orthogonality. Point 3 of the
# ticket asks that the TIGHTENING be disarmable without disarming the static
# bound; the converse would make no sense — an operator who cuts "the bound"
# and keeps a refusal has a remedy bigger than the problem. Asserted WITH this
# comment so a naive reading does not "fix" it. The orthogonality mika#2165
# bears on is asserted below, across guards, and it holds.
declare_repos("mika")
BOUND_SWITCH = "MIKA_PILOT_EGRESS_PUSH_BOUND"
for bound, scope, want, label in (
    ("1", "1", 403, "both armed: the tightening refuses the sibling"),
    ("1", "0", 200, "tightening off: exactly mika#2736, the sibling passes"),
    ("0", "1", 200, "bound off: the tightening lives UNDER it, so nothing"),
    ("0", "0", 200, "both off: nothing"),
):
    os.environ[BOUND_SWITCH] = bound
    os.environ[SCOPE_SWITCH] = scope
    flow, err = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
    check(f"19.x ({bound}, {scope}) {label}", want, status_of(flow))
    if want == 200:
        check(f"19.x ({bound}, {scope}) and the credential is injected",
              EXPECTED_BASIC, flow.request.headers.get("Authorization"))
# `PUSH_SCOPE=0` must give back mika#2736 EXACTLY, which means the four static
# motifs still fire. A rollback that also dropped the static bound would be a
# remedy bigger than the problem.
os.environ[BOUND_SWITCH] = "1"
os.environ[SCOPE_SWITCH] = "0"
for label, path, command, motif in (
    ("a foreign organisation", FOREIGN_PATH, cmd(ZERO, SHA_A, BRANCH),
     "push-repo-foreign"),
    ("a ref deletion", PUSH_PATH, cmd(SHA_A, ZERO, BRANCH), "push-ref-delete"),
    ("an exotic ref", PUSH_PATH, cmd(ZERO, SHA_A, b"refs/tags/v1"),
     "push-ref-exotic"),
    ("a protected branch", PUSH_PATH, cmd(SHA_A, SHA_B, b"refs/heads/main"),
     "push-ref-protected"),
):
    flow, _ = run(GIT, path, body(command))
    check(f"19.y tightening off, {label} is still refused", 403, status_of(flow))
    check_true(f"19.y and by its own static motif ({motif})",
               motif in body_of(flow))
for value in ("FALSE", "no", "Off", "disabled"):
    os.environ[SCOPE_SWITCH] = value
    flow, _ = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
    check(f"19.z `{value}` disarms the tightening too (same vocabulary)",
          200, status_of(flow))
for value in ("1", "", "yes", "garbage"):
    os.environ[SCOPE_SWITCH] = value
    flow, _ = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
    check(f"19.w `{value}` leaves the tightening ARMED (default is armed)",
          403, status_of(flow))
# ORTHOGONALITY ACROSS GUARDS (class mika#2165), which is where it is real.
os.environ[SCOPE_SWITCH] = "0"
flow, err = run(API, "/graphql", method="POST")
check("19.1 PUSH_SCOPE=0 leaves the api.github.com cut of mika#2724 intact",
      403, status_of(flow))
check_true("19.2 with ITS own motif, not a push one",
           "sentinel-absent" in body_of(flow))
del os.environ[SCOPE_SWITCH]
os.environ["MIKA_PILOT_EGRESS_SENTINEL"] = "0"
flow, _ = run(GIT, SIBLING_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("19.3 and MIKA_PILOT_EGRESS_SENTINEL=0 leaves the tightening intact",
      403, status_of(flow))
check_true("19.4 still with the scope motif",
           "push-scope-repo" in body_of(flow))
del os.environ["MIKA_PILOT_EGRESS_SENTINEL"]
del os.environ[BOUND_SWITCH]
forget_scopes()

print("")
print("CASE 20 — a perimeter entry stops answering on its own (TTL, mika#2766)")
# The addon half of the entry lifecycle. Unregistration at spawn end covers the
# ordinary exit; a SIGKILLed dispatch runs NO trap, so an entry nothing removed
# has to stop answering by itself. 12 h and not the sentinels' 72 h: that number
# is calibrated on "can a flight still be read three days later?", this registry
# answers "is this perimeter still in flight?", and a three-day-old entry never
# is.
declare_repos("mika")
stale = time.time() - 13 * 3600
os.utime(SCOPES / SCOPE_ENTRY_A, (stale, stale))
addon._last_purge = {}  # the ten-minute throttle must not hide the purge here
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check_true("20.1 the entry past the TTL is gone",
           not (SCOPES / SCOPE_ENTRY_A).exists())
check("20.2 and the verdict falls back on the static bound, never a refusal",
      200, status_of(flow))
check("20.3 naming scope-absent — it was the only entry, so nothing is declared",
      [f"[github-proxy] DEGRADE scope-absent POST {PUSH_PATH}"], degrades_in(err))
# The other side of the boundary, which is the side that stops the loop: an
# entry still INSIDE the TTL must keep authorising its repository. A TTL cut
# "to look tidy" would 403 a live pilot, and this is where that reddens.
declare_repos("mika")
fresh = time.time() - 11 * 3600
os.utime(SCOPES / SCOPE_ENTRY_A, (fresh, fresh))
addon._last_purge = {}
flow, err = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check_true("20.4 an entry INSIDE the TTL survives", (SCOPES / SCOPE_ENTRY_A).exists())
check("20.5 and still authorises its repository", 200, status_of(flow))
check("20.6 with nothing degraded — it really is still declared",
      [], degrades_in(err))
# The value itself, asserted so moving it is a deliberate act and not a drift.
# 12 h is 2.7x the longest spawn measured over the 718 pilot transcripts on the
# development host (max 4.40 h); 4 h would already be under it.
check("20.7 the TTL is the value the derivation landed on, in seconds",
      12 * 3600, addon._PUSH_SCOPE_TTL_SECONDS)
check_true("20.8 and it is strictly shorter than the sentinels' — not one number",
           addon._PUSH_SCOPE_TTL_SECONDS < addon._REGISTRY_TTL_SECONDS)

print("")
print("CASE 21 — the two registries age at their OWN rates, one request path")
# The regression this reddens on is a SINGLE TTL. `_purge_registry` serves two
# registries and used to hold the number itself, exactly as it used to hold one
# global throttle (CASE 13 of the sentinel suite). One entry of the SAME age in
# each, picked between the two TTLs — 13 h, past the perimeter's 12 h and well
# inside the sentinels' 72 h — so neither a shared 12 h nor a shared 72 h can
# read green here.
declare_repos("mika")
AGED_SENTINEL = "5555555555555555555555555555d00d"
(REGISTRY / AGED_SENTINEL).write_text("", encoding="utf-8")
same_age = time.time() - 13 * 3600
os.utime(SCOPES / SCOPE_ENTRY_A, (same_age, same_age))
os.utime(REGISTRY / AGED_SENTINEL, (same_age, same_age))
addon._last_purge = {}
# Two requests, because the two registries are read on two different paths: the
# perimeter on a push to github.com, the sentinel registry on api.github.com.
flow, _ = run(GIT, PUSH_PATH, body(cmd(ZERO, SHA_A, BRANCH)))
check("21.1 the push is served", 200, status_of(flow))
flow, _ = run(API, "/rate_limit", cred=f"Bearer {SENTINEL}", method="GET")
check("21.2 the api.github.com call with an attested sentinel is served too",
      200, status_of(flow))
check_true("21.3 the 13 h PERIMETER entry is purged (past its own 12 h)",
           not (SCOPES / SCOPE_ENTRY_A).exists())
check_true("21.4 the 13 h SENTINEL entry survives (well inside its own 72 h)",
           (REGISTRY / AGED_SENTINEL).exists())
check_true("21.5 and the nonce this whole suite runs on is untouched",
           (REGISTRY / NONCE).exists())
check_true("21.6 the two directories were throttled independently, by key",
           {str(SCOPES), str(REGISTRY)} <= set(addon._last_purge))
(REGISTRY / AGED_SENTINEL).unlink()
addon._last_purge = {}
forget_scopes()

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
rc=1; [ "${asserted:-0}" -ge 254 ] && rc=0
assert_eq "anti-vacuity: the python half really asserted ($asserted assertions >= 254)" "0" "$rc"
assert_eq "every addon-level assertion passes" "0" "$py_rc"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
