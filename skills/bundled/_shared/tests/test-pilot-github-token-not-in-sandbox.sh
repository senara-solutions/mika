#!/bin/bash
# mika#2056 anti-vacuity suite: the GitHub token is reachable HOST-SIDE but
# absent SANDBOX-SIDE — the two halves proven against the SAME token in the
# SAME run, so neither half is vacuous on its own.
#
# The security claim of mika#2056 is a difference, not an absence: removing the
# token from everywhere would satisfy "sandbox-absent" while breaking the pilot;
# leaving it reachable everywhere would satisfy "host-present" while leaking it.
# This suite asserts both edges at once:
#
#   HOST-REACHABLE — the egress-proxy MITM addon (mika-pilot-github-auth-addon)
#     can read the token host-side and injects the correct Authorization header
#     per GitHub host. The credential is NOT gone; it moved host-side.
#
#   SANDBOX-ABSENT — with the production (empty) secret allowlist, the same
#     token, set in the parent env and staged host-side, is absent from the
#     real sandbox environment AND from the sandbox filesystem (no
#     /run/mika-pilot-secrets/GH_TOKEN, and the host-only staging file is not
#     visible through any bind).
#
# Companion: skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh
# (argv-side + generic mika#2039 channel). Neither subsumes this one.
#
# Run: bash skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
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

FAKE_TOKEN="ghp_2056abcdef2056abcdef2056abcdef2056abcd"

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2056-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT
export HOME="$TMPROOT/home"
WORKTREE_DIR="$TMPROOT/worktree"
mkdir -p "$HOME" "$WORKTREE_DIR" "$HOME/.mika/data/pilot-transcripts"

# ============================================================================
# PART A — HOST-REACHABLE: the addon reads the token + injects the right header
# ============================================================================
# The addon imports `from mitmproxy import http`. mitmproxy is not a test
# dependency, so a stub module satisfies the import; the functions under test
# (`_read_token`, `_auth_header_for`) never touch the real API. This proves the
# credential is reachable and usable host-side — the "not vacuous" half.
echo ""
echo "PART A — token is reachable host-side (addon can read + inject)"
echo "---------------------------------------------------------------"

python3 - "$GH_ADDON" "$FAKE_TOKEN" "$HOME" <<'PY'
import importlib.util
import sys
import types
from pathlib import Path

addon_path, fake_token, home = sys.argv[1], sys.argv[2], sys.argv[3]

# Stub the mitmproxy dependency so the module imports without it installed.
mitm = types.ModuleType("mitmproxy")
http_mod = types.ModuleType("mitmproxy.http")
http_mod.HTTPFlow = object
http_mod.Response = object
mitm.http = http_mod
sys.modules["mitmproxy"] = mitm
sys.modules["mitmproxy.http"] = http_mod

spec = importlib.util.spec_from_file_location("gh_addon", addon_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

results = []

# 1. env fallback (no staged file yet) — the canary/tests path.
for name in ("GH_TOKEN", "MIKA_GITHUB_TOKEN"):
    sys.modules  # noqa
import os
os.environ.pop("MIKA_GITHUB_TOKEN", None)
os.environ["GH_TOKEN"] = fake_token
# Ensure the staged file is absent so we exercise the env fallback.
tok_file = Path(home) / ".mika" / "pilot-gh-token"
if tok_file.exists():
    tok_file.unlink()
mod._token_cache.update({"mtime": None, "value": None})
results.append(("env fallback reads GH_TOKEN", mod._read_token() == fake_token))

# 2. staged host-only file wins over env (rotation-safe path).
tok_file.parent.mkdir(parents=True, exist_ok=True)
tok_file.write_text("ghs_STAGEDINSTALLTOKEN00000000000000000000", encoding="utf-8")
mod._token_cache.update({"mtime": None, "value": None})
results.append(("staged file is preferred over env",
                mod._read_token() == "ghs_STAGEDINSTALLTOKEN00000000000000000000"))

# 3. header shape per host — Bearer for the REST host, Basic for git smart-HTTP.
import base64
api_hdr = mod._auth_header_for("api.github.com", fake_token)
git_hdr = mod._auth_header_for("github.com", fake_token)
expected_basic = "Basic " + base64.b64encode(
    f"x-access-token:{fake_token}".encode()).decode()
results.append(("api.github.com → Bearer", api_hdr == f"Bearer {fake_token}"))
results.append(("github.com → Basic x-access-token", git_hdr == expected_basic))

# 4. no token anywhere → _read_token returns None (fail-closed source).
os.environ.pop("GH_TOKEN", None)
tok_file.unlink()
mod._token_cache.update({"mtime": None, "value": None})
results.append(("no source → None (fail-closed)", mod._read_token() is None))

ok = all(v for _, v in results)
for label, v in results:
    print(f"RESULT {'PASS' if v else 'FAIL'} {label}")
sys.exit(0 if ok else 1)
PY
addon_rc=$?
assert_eq "addon host-side token-read + header-injection all pass" "0" "$addon_rc"

# ============================================================================
# PART B — the dispatcher stages the token host-side, 0600, off every bind
# ============================================================================
echo ""
echo "PART B — dispatcher stages the token host-side (0600, unbound path)"
echo "-------------------------------------------------------------------"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

GH_TOKEN="$FAKE_TOKEN" _stage_pilot_gh_token

staged_file="$HOME/.mika/pilot-gh-token"
rc=1; [ -f "$staged_file" ] && rc=0
assert_eq "staged token file exists" "0" "$rc"
assert_eq "staged token content matches" "$FAKE_TOKEN" "$(cat "$staged_file" 2>/dev/null)"
assert_eq "staged token file is mode 0600" "600" "$(stat -c '%a' "$staged_file" 2>/dev/null)"

# The staging path must NOT be any bwrap bind source. The only ~/.mika bind is
# ~/.mika/data/pilot-transcripts; the token lives at ~/.mika/pilot-gh-token,
# a sibling that is deliberately never bound. Assert dispatch-lib declares no
# --bind / --ro-bind of the token path (a future regression that exposed it
# would trip here).
bound_token=$(grep -nE -- '(--bind|--ro-bind[a-z-]*)[^"]*pilot-gh-token' "$DISPATCH_LIB" || true)
assert_eq "the staged token path is never a bwrap bind source" "" "$bound_token"

# ============================================================================
# PART C — SANDBOX-ABSENT: same token, real sandbox, cannot be read inside
# ============================================================================
echo ""
echo "PART C — the staged/host token is absent inside the real sandbox"
echo "----------------------------------------------------------------"

if ! command -v bwrap >/dev/null 2>&1; then
    echo "  ⊘ real-sandbox checks skipped — bwrap not installed on PATH"
else
    # Neutralise the daemon launchers so no real egress proxy / mitmproxy is
    # spawned by this test; Phase 2a (fs cut) is enough to prove env/fs absence.
    _ensure_pilot_egress_proxy() { return 1; }
    _ensure_pilot_helper() { return 1; }

    # Production shape: empty secret allowlist (the mika#2056 state).
    _PILOT_SANDBOX_SECRET_ALLOWLIST=()

    got_env=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${GH_TOKEN:-<absent>}"' 2>/dev/null)
    assert_eq "GH_TOKEN is absent from the real sandbox environment" "<absent>" "$got_env"

    got_secret_file=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c '[ -e /run/mika-pilot-secrets/GH_TOKEN ] && echo present || echo missing' 2>/dev/null)
    assert_eq "no /run/mika-pilot-secrets/GH_TOKEN inside the sandbox" "missing" "$got_secret_file"

    # The host-only staging file (which DOES hold the token, host-side) must not
    # be visible through any bind: ~/.mika is tmpfs-blanked except the bound
    # data subdir. Same token, proven unreadable from inside.
    got_staged_visible=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c '[ -e "$HOME/.mika/pilot-gh-token" ] && echo present || echo missing' 2>/dev/null)
    assert_eq "the host-only staging file is not visible inside the sandbox" "missing" "$got_staged_visible"

    # And a positive control: the sandbox IS otherwise functional (proves the
    # absence above is real isolation, not a broken launch that fails every
    # command — the vacuity trap this whole suite guards against).
    got_alive=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'echo alive' 2>/dev/null)
    assert_eq "the sandbox itself runs (absence is isolation, not breakage)" "alive" "$got_alive"
fi

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
