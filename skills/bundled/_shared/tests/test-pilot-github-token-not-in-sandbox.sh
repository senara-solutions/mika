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
#     real sandbox environment (whose GH_TOKEN is only the non-secret
#     placeholder since mika#2572) AND from the sandbox filesystem (no
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

# mika#2578 — PART D needs substring assertions. Same shape and same `grep -qF`
# as the sibling suite `skills/bundled/_shared/test-dispatch-lib.sh`. Without
# them the PART D scan calls undefined functions: under this suite's `set -uo`
# (no `-e`), bash prints "command not found", FAIL stays 0, and five structural
# assertions read as green while asserting nothing. Measured while writing them.
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to contain: $needle"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if ! grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected NOT to contain: $needle"
    fi
}

FAKE_TOKEN="ghp_2056abcdef2056abcdef2056abcdef2056abcd"

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2056-XXXXXX")
# mika#2049 — the fabricated relay's listener dies with the temp tree. Guarded:
# the helper is sourced later and may never be, if bwrap is absent.
trap 'if declare -F stop_fake_egress_relay >/dev/null 2>&1; then stop_fake_egress_relay; fi; rm -rf "$TMPROOT"' EXIT
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

# --- mika#2578 (D4): a harness can REDIRECT the staging path ---------------
# The three assertions above are the default half's negative control: with no
# redirection in scope, _stage_pilot_gh_token writes $HOME/.mika/pilot-gh-token
# — the ONLY behaviour the tree had before mika#2578, and the one that let
# `scripts/canary-pilot-containment` overwrite the operator's live credential
# with its decoy on every run.
#
# The redirection is posed BEFORE the `source`, in a subshell, because that is
# the real mechanism: the canary exports the variable and dispatch-lib's
# declaration has to KEEP it. Posing it after the source would only prove that
# _stage_pilot_gh_token reads its variable — true on both sides of U1 — so the
# assertions would stay green on the very tree that carries the defect. Measured
# while writing them: the post-source form passed on a reverted dispatch-lib.
rm -f "$staged_file"
redirect_file="$TMPROOT/redirected/pilot-gh-token"

surviving_path=$(
    export _PILOT_GH_TOKEN_FILE="$redirect_file"
    # shellcheck source=/dev/null
    source "$DISPATCH_LIB" >/dev/null 2>&1
    GH_TOKEN="$FAKE_TOKEN" _stage_pilot_gh_token
    printf '%s' "$_PILOT_GH_TOKEN_FILE"
)
assert_eq "mika#2578: a path posed BEFORE the source survives it" "$redirect_file" "$surviving_path"

rc=1; [ -f "$redirect_file" ] && rc=0
assert_eq "mika#2578: the harness path receives the staged token" "0" "$rc"
assert_eq "mika#2578: the redirected file carries the token" "$FAKE_TOKEN" "$(cat "$redirect_file" 2>/dev/null)"
assert_eq "mika#2578: the redirected file is mode 0600" "600" "$(stat -c '%a' "$redirect_file" 2>/dev/null)"
# THE assertion of the ticket, and it is an ABSENCE: the host default was not
# created. The three positives above are what keep it from being vacuous — a
# _stage_pilot_gh_token turned no-op would satisfy it too.
rc=1; [ ! -e "$staged_file" ] && rc=0
assert_eq "mika#2578: the host DEFAULT path is not written when redirected" "0" "$rc"

# And nothing was frozen: with nothing posed, staging still targets the host
# default. Without this, every assertion above is compatible with the staging
# having broken outright.
GH_TOKEN="$FAKE_TOKEN" _stage_pilot_gh_token
rc=1; [ -f "$staged_file" ] && rc=0
assert_eq "mika#2578: with nothing posed, staging still targets the host default" "0" "$rc"

# ============================================================================
# PART C — SANDBOX-ABSENT: same token, real sandbox, cannot be read inside
# ============================================================================
echo ""
echo "PART C — the staged/host token is absent inside the real sandbox"
echo "----------------------------------------------------------------"

if ! command -v bwrap >/dev/null 2>&1; then
    echo "  ⊘ real-sandbox checks skipped — bwrap not installed on PATH"
else
    # mika#2049 — a SERVING relay, fabricated. This used to stub the launcher to
    # `return 1`, which exercised the Phase 2a fallback (fs cut, network open).
    # That fallback no longer exists: the posture is fail-closed, so `return 1`
    # now makes `_run_pilot_sandboxed` refuse and there would be no sandbox left
    # to inspect. The credential-absence invariant (mika#2056) is unchanged;
    # only the preparation moved. See lib-fake-egress-relay.sh.
    # shellcheck source=skills/bundled/_shared/tests/lib-fake-egress-relay.sh
    source "$SCRIPT_DIR/lib-fake-egress-relay.sh"
    stub_serving_egress_relay "$TMPROOT"
    _ensure_pilot_helper() { return 1; }

    # Production shape: empty secret allowlist (the mika#2056 state).
    _PILOT_SANDBOX_SECRET_ALLOWLIST=()

    got_env=$(GH_TOKEN="$FAKE_TOKEN" _run_pilot_sandboxed \
        /bin/sh -c 'printf %s "${GH_TOKEN:-<absent>}"' 2>/dev/null)
    # mika#2572: the NAME travels, set to the non-secret placeholder so gh does
    # not stop on `gh auth login`; the token VALUE never does.
    assert_eq "GH_TOKEN in the real sandbox is the placeholder (mika#2572)" "proxy-managed-no-secret" "$got_env"
    rc=1; [ "$got_env" = "$FAKE_TOKEN" ] && rc=0
    assert_eq "the staged/parent token value never reaches the sandbox env" "1" "$rc"

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

# ============================================================================
# PART D — mika#2578: the canary never writes the host credential path
# ============================================================================
# Structural, not behavioural, and deliberately so: running the canary here
# would exercise the very write this guard exists to forbid — reproducing the
# defect to prove we detect it would cost exactly the damage mika#2578 closes.
echo ""
echo "PART D — the canary redirects its staging (mika#2578)"
echo "-----------------------------------------------------"

CANARY="$REPO_ROOT/scripts/canary-pilot-containment"

# --- Anti-vacuity FIRST (mika#2205): a scan whose path rots, or whose subject
# shrank, passes by looking at nothing and reads exactly like a clean tree.
rc=1; [ -f "$CANARY" ] && rc=0
assert_eq "D3 anti-vacuity: the canary exists at scripts/canary-pilot-containment" "0" "$rc"
canary_bytes=$(wc -c < "$CANARY" 2>/dev/null || echo 0)
rc=1; [ "${canary_bytes:-0}" -gt 15000 ] && rc=0
assert_eq "D3 anti-vacuity: the canary is the real file ($canary_bytes bytes > 15000)" "0" "$rc"
canary_sandbox_sites=$(grep -c '_run_pilot_sandboxed' "$CANARY" || true)
rc=1; [ "${canary_sandbox_sites:-0}" -ge 1 ] && rc=0
assert_eq "D3 anti-vacuity: the canary still calls the sandbox ($canary_sandbox_sites mentions)" "0" "$rc"

# --- Ordering: the redirection is in scope before anything can stage.
canary_pose_line=$(grep -n '^export _PILOT_GH_TOKEN_FILE=' "$CANARY" | head -1 | cut -d: -f1)
canary_source_line=$(grep -n '^source "\$DISPATCH_LIB"' "$CANARY" | head -1 | cut -d: -f1)
# First NON-COMMENT mention of the sandbox helper — the canary's own prose names
# it several times, and a comment is not a call (class mika#2050). Composed from
# the two idioms this file already uses (`grep -vE '^[[:space:]]*#'` to strip
# comments, `head -1 | cut -d: -f1` to take a line number) rather than a
# hand-rolled loop; the `^[0-9]+:` prefix is what `grep -n` prepends.
canary_first_call_line=$(grep -n '_run_pilot_sandboxed' "$CANARY" \
    | grep -vE '^[0-9]+:[[:space:]]*#' | head -1 | cut -d: -f1)

rc=1
[ -n "$canary_pose_line" ] && [ -n "$canary_source_line" ] \
    && [ "$canary_pose_line" -lt "$canary_source_line" ] && rc=0
assert_eq "D3: the redirection is posed BEFORE the dispatch-lib source (line $canary_pose_line < $canary_source_line)" "0" "$rc"

rc=1
[ -n "$canary_source_line" ] && [ -n "$canary_first_call_line" ] \
    && [ "$canary_source_line" -lt "$canary_first_call_line" ] && rc=0
assert_eq "D3: the source precedes the first sandbox call (line $canary_source_line < $canary_first_call_line)" "0" "$rc"

# --- Exactly one pose site, and it does not point back at the host default.
# The allowlist is an INPUT to the count, never a statement standing beside it:
# an entry added here removes its line and drops the count below 1, so the
# assertion goes red at the same time as the emptiness pin below. That coupling
# is what makes "shipped empty, pinned empty" mean something — a pin on a
# variable nothing reads is a PASS that measures nothing (doctrine mika#2201).
# When this scan fires, REDIRECT the offending site; do not exempt it.
CANARY_HOST_WRITE_ALLOWLIST=""

canary_pose_lines=$(grep -vE '^[[:space:]]*#' "$CANARY" \
    | grep -E '(^|[[:space:]])(export[[:space:]]+)?_PILOT_GH_TOKEN_FILE:?=' || true)
# `grep -vxF -e ""` drops only empty lines, so an empty allowlist filters
# nothing but the trailing newline `printf` adds.
canary_pose_kept=$(printf '%s\n' "$canary_pose_lines" \
    | grep -vxF -e "$CANARY_HOST_WRITE_ALLOWLIST" || true)
canary_pose_count=$(printf '%s' "$canary_pose_kept" | grep -c . || true)
assert_eq "D3: exactly one non-allowlisted site poses _PILOT_GH_TOKEN_FILE" "1" "${canary_pose_count:-0}"
assert_eq "D3: and the allowlist that count consumes is empty" "" "$CANARY_HOST_WRITE_ALLOWLIST"

canary_pose_stmt=$(printf '%s' "$canary_pose_kept" | head -1)
assert_not_contains "D3: the redirection does not point back at the host default" \
    '.mika/pilot-gh-token' "$canary_pose_stmt"
assert_contains "D3: the redirection points at the canary's own temp dir" \
    '_CANARY_TOKEN_STAGE_DIR' "$canary_pose_stmt"

# --- R6: the self-check must observe the host DEFAULT, never the redirection.
# Observing $_PILOT_GH_TOKEN_FILE would make the guard tautologically green —
# it would compare the redirected file to itself and always report "untouched".
canary_exit_fn=$(sed -n '/^_canary_on_exit() {$/,/^}$/p' "$CANARY")
rc=1; [ -n "$canary_exit_fn" ] && rc=0
assert_eq "D3 anti-vacuity: the _canary_on_exit body was extracted" "0" "$rc"
# Comments are stripped BEFORE the scan: the function's prose explains why it
# must not read $_PILOT_GH_TOKEN_FILE, and naming a variable in order to forbid
# it is not a read. Same rule the mika#2425 single-reader scan states, and the
# same false positive mika#2050 measured on the Signal S grep.
canary_exit_code=$(printf '%s\n' "$canary_exit_fn" | grep -vE '^[[:space:]]*#' || true)
rc=1; [ -n "$canary_exit_code" ] && rc=0
assert_eq "D3 anti-vacuity: the comment-stripped body is non-empty" "0" "$rc"
assert_contains "D3 (R6): the self-check observes the host default path as a literal" \
    '.mika/pilot-gh-token' "$canary_exit_code"
assert_not_contains "D3 (R6): the self-check never observes the redirection variable" \
    '_PILOT_GH_TOKEN_FILE' "$canary_exit_code"

# --- KTD7: one EXIT trap, so the token self-check and the mika#2141 victim-ref
# cleanup cannot displace each other.
canary_exit_traps=$(grep -vE '^[[:space:]]*#' "$CANARY" \
    | grep -cE '^[[:space:]]*trap[[:space:]].*[[:space:]]EXIT[[:space:]]*$' || true)
assert_eq "D3 (KTD7): the canary installs exactly one EXIT trap" "1" "${canary_exit_traps:-0}"
assert_contains "D3 (KTD7): and that trap also cleans the mika#2141 victim ref" \
    'CANARY_VICTIM_REF' "$canary_exit_fn"

# --- D3, fourth term (R1/R13): no OTHER non-comment line WRITES the host default.
# The three scans above constrain where the REDIRECTION is posed; not one of them
# would see a site that bypasses `_PILOT_GH_TOKEN_FILE` entirely and names the
# host path directly. That is exactly how the ticket's rejected "voie 2" would be
# written — back the file up, restore it in a trap — and it is a write to the live
# credential either way.
#
# Two terms, and the second is what makes this fail-closed:
#   A. a MUTATING form on a line that names the host path — a redirect into it, or
#      a mutating command anywhere on the line;
#   B. a line that names the host path and matches NONE of the declared read
#      forms — an unrecognised shape is REFUSED, never assumed harmless.
#
# Neither term subsumes the other. Term B alone would pass
# `echo x > "$HOME/.mika/pilot-gh-token"`, which matches the `echo` read form;
# term A alone is a denylist of command names, which the next write idiom walks
# straight past. The asymmetry that settles the direction: a wrongful red costs
# one CI run plus a one-line declaration below, a wrongful green costs the
# operator's live GitHub credential and every in-flight pilot's authentication,
# irreversibly. Same rule mika#2520 states, running the opposite way — there an
# unreadable signal must KEEP because the action destroyed work; here it must
# REFUSE because the action IS the write.
#
# Shipped EMPTY and pinned empty. When this fires: on a WRITE, redirect the
# offending site; on a legitimate new READ, declare its form in
# canary_host_read_forms. Never add an exemption — doctrine mika#2201, "on
# déclare, on n'allowliste pas": a guard you can exempt is a guard with an
# off-switch nobody announces.
CANARY_HOST_MUTATION_ALLOWLIST=""

# The allowlist is consumed BEFORE both violation terms, so an exemption really
# does silence them — and the emptiness pin below is then the ONLY assertion that
# reddens, naming the exempted line verbatim. That is the point: exempting is a
# visible act, never a quiet bypass. A pin standing beside a variable nothing
# reads is a PASS that measures nothing — the trap the pose-count block one screen
# up already names — and this one was measured: with the allowlist carrying one
# write, both terms went green and the pin went red alone.
#
# The floor is a different guard, against a different failure: it catches the path
# being RENAMED out from under the scan, not an exemption (one entry takes the
# count from 6 to 5, which clears 4).
canary_host_hits=$(grep -vE '^[[:space:]]*#' "$CANARY" \
    | grep -F -- '.mika/pilot-gh-token' || true)
canary_host_hits=$(printf '%s\n' "$canary_host_hits" \
    | grep -vxF -e "$CANARY_HOST_MUTATION_ALLOWLIST" || true)
canary_host_hit_count=$(printf '%s' "$canary_host_hits" | grep -c . || true)

rc=1; [ "${canary_host_hit_count:-0}" -ge 4 ] && rc=0
assert_eq "D3 anti-vacuity: the host default is still named on non-comment lines ($canary_host_hit_count)" "0" "$rc"
assert_eq "D3: and the allowlist that filter consumes is empty" "" "$CANARY_HOST_MUTATION_ALLOWLIST"

# Term A. `>>?` covers truncate and append; `2>/dev/null` cannot match it because
# the target has to be the host path itself. The command list is the mutating
# half of what a shell can do to a file that already exists, deletion included —
# R1 forbids writing, truncating AND removing it.
canary_host_writes=$(printf '%s\n' "$canary_host_hits" | grep -E \
    -e '>>?[[:space:]]*"?\$\{?HOME\}?/\.mika/pilot-gh-token' \
    -e '(^|[[:space:]]|\||;|&)(cp|mv|rm|ln|dd|tee|touch|truncate|install|shred|chmod|chown)([[:space:]]|$)' \
    -e 'sed[[:space:]]+-i' \
    || true)
assert_eq "D3: no non-comment line writes, truncates or removes the host default" "" "$canary_host_writes"

# Term B. The declared read forms — the vocabulary of the predicate, NOT an
# exemption list: each one is a shape the canary legitimately needs to observe
# its own blast radius (the BEFORE fingerprint, the _canary_on_exit comparison,
# and the R9 attribution line). Reformat one of those sites and this goes red
# with the line printed; that is the intended cost of a fail-closed predicate.
canary_host_read_forms=(
    '_canary_digest_of[[:space:]]+"\$HOME/\.mika/pilot-gh-token"'
    '^[[:space:]]*(local[[:space:]]+)?_[A-Za-z0-9_]+="\$HOME/\.mika/pilot-gh-token"$'
    '\[[[:space:]]+-[efsr][[:space:]]+"\$HOME/\.mika/pilot-gh-token"[[:space:]]+\]'
    'stat[[:space:]]+-c[[:space:]]'
    '^[[:space:]]*echo[[:space:]]'
)
canary_host_unknown=$(
    while IFS= read -r _line; do
        [ -n "$_line" ] || continue
        _matched=1
        for _form in "${canary_host_read_forms[@]}"; do
            if grep -qE -- "$_form" <<<"$_line"; then _matched=0; break; fi
        done
        [ "$_matched" -eq 0 ] || printf '%s\n' "$_line"
    done <<<"$canary_host_hits"
)
assert_eq "D3: every non-comment mention of the host default is a declared read form" "" "$canary_host_unknown"

# --- The canary still parses.
canary_rc=0
bash -n "$CANARY" 2>/dev/null || canary_rc=$?
assert_eq "D3: scripts/canary-pilot-containment passes bash -n" "0" "$canary_rc"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
