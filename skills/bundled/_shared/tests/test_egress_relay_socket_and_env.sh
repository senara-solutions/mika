#!/bin/bash
# The pilot egress relay: owner-only socket, minimal launch environment.
#
# WHAT THIS HOLDS, against the REAL relay launched by the REAL launcher.
#
#   1. The socket `_ensure_pilot_egress_proxy` brings up is mode 0600.
#   2. The relay process does not inherit the caller's environment: a sentinel
#      variable exported by the caller is absent from /proc/<pid>/environ, while
#      the variables the relay actually reads (PATH, HOME, MIKA_EGRESS_DEBUG)
#      are present.
#   3. A pilot inside a real bwrap sandbox, same uid, still connects to that
#      0600 socket through the bind-mount `_run_pilot_sandboxed` sets up.
#
# Point 3 is the one the canary cannot give: `scripts/canary-pilot-containment`
# talks to the relay already serving /tmp/mika-pilot-egress.sock, i.e. to
# whatever code that relay was started from. This suite starts the branch's
# relay on a temporary socket instead, so it proves the new mode is reachable
# without touching the live relay or its socket.
#
# NEGATIVE CONTROL, built in. The same probe runs against a copy of
# dispatch-lib.sh whose launcher line has `env -i "${relay_env[@]}"` removed;
# the sentinel must then be PRESENT in the relay's environment. Without that
# half, an absent sentinel could mean "filtered" or "never exported", and the
# suite could not tell them apart. The precondition asserts the copy differs
# from the real library by exactly the line under test.
#
# ISOLATION. HOME is exported BEFORE dispatch-lib is sourced: the library fixes
# `$HOME/.mika/pilot-gh-token` and the mitmdump paths at source time, so a HOME
# switched afterwards would let `_run_pilot_sandboxed` stage a GitHub token into
# the operator's real state (the mika#2578 class). GH_TOKEN is unset, the
# mitmdump helper is stubbed out, and the egress-down/up stamps are neutralised.
# `_ensure_pilot_egress_proxy` itself stays real: it is the code under test.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
RELAY="$REPO_ROOT/scripts/mika-pilot-egress-proxy"
SENTINEL_NAME="MIKA_EGRESS_LAUNCH_SENTINEL"
SENTINEL_VALUE="sentinel-must-not-reach-the-relay"

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

if ! command -v bwrap >/dev/null 2>&1; then
    echo "⊘ skipped — bwrap not installed on PATH (this suite needs a real sandbox)"
    exit 0
fi
if [ ! -x "$RELAY" ]; then
    echo "✗ relay script not executable at $RELAY"
    exit 1
fi

# Short root: a unix socket path is capped at 108 bytes, and the default TMPDIR
# of some harnesses is long enough to cross it once `egress/relay.sock` is added.
TMPROOT=$(mktemp -d /tmp/mika-egress-env-XXXXXX)
trap 'rm -rf "$TMPROOT"' EXIT

MUTATED_LIB="$TMPROOT/dispatch-lib-no-env-i.sh"
sed 's/nohup env -i "\${relay_env\[@\]}" /nohup /' "$DISPATCH_LIB" > "$MUTATED_LIB"
changed=$(diff "$DISPATCH_LIB" "$MUTATED_LIB" | grep -c '^<' || true)

echo ""
echo "PRECONDITION — the negative control differs by exactly the line under test"
echo "--------------------------------------------------------------------------"
assert_eq "the mutated library drops env -i on exactly one line" "1" "$changed"

# probe <lib> <run-root>
# Launches the relay through the given library on a temporary socket, then
# prints `key=value` lines: the socket mode, the relay's environment, and
# whether a sandboxed pilot reached the socket.
probe() {
    local lib="$1" root="$2"
    mkdir -p "$root/home/.mika/data/pilot-transcripts" "$root/worktree" \
        "$root/egress" "$root/logs"
    bash -c '
        set -uo pipefail
        lib="$1"; root="$2"; relay="$3"; sname="$4"; svalue="$5"
        export HOME="$root/home"
        unset GH_TOKEN
        WORKTREE_DIR="$root/worktree"
        # shellcheck disable=SC1090
        source "$lib"
        _ensure_pilot_helper() { return 1; }
        _pilot_egress_mark_up() { :; }
        _pilot_egress_mark_down() { :; }
        _PILOT_SANDBOX_SECRET_ALLOWLIST=()
        _PILOT_EGRESS_SOCK="$root/egress/relay.sock"
        _PILOT_EGRESS_PROXY_BIN="$relay"
        export MIKA_PILOT_EGRESS_LOG_DIR="$root/logs"
        export MIKA_EGRESS_DEBUG=1
        export "$sname=$svalue"

        launch=$(_ensure_pilot_egress_proxy 2>&1 >/dev/null)
        echo "launch_rc=$?"
        pid=$(printf "%s\n" "$launch" | sed -n "s/.*pilot-egress-proxy launched (pid \([0-9]*\).*/\1/p" | head -1)
        echo "pid_found=$([ -n "$pid" ] && echo yes || echo no)"
        [ -n "$pid" ] || exit 0

        echo "sock_mode=$(stat -c %a "$_PILOT_EGRESS_SOCK" 2>/dev/null)"
        env_dump=$(tr "\0" "\n" < "/proc/$pid/environ" 2>/dev/null)
        has() { printf "%s\n" "$env_dump" | grep -q "^$1=" && echo yes || echo no; }
        echo "env_sentinel=$(has "$sname")"
        echo "env_path=$(has PATH)"
        echo "env_home=$(has HOME)"
        echo "env_debug=$(printf "%s\n" "$env_dump" | sed -n "s/^MIKA_EGRESS_DEBUG=//p")"
        # Names outside the launcher allowlist. Measured: `env -i` + exec adds
        # none of its own, so the comparison is exact.
        allowed=" ${_PILOT_EGRESS_RELAY_ENV_ALLOWLIST[*]:-} "
        extra=""
        for n in $(printf "%s\n" "$env_dump" | cut -d= -f1); do
            case "$allowed" in *" $n "*) ;; *) extra="$extra $n" ;; esac
        done
        echo "env_outside_allowlist=${extra:-none}"

        sandboxed=$(_run_pilot_sandboxed python3 -c "
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
try:
    s.connect(sys.argv[1])
    print(\"connect=ok\")
except OSError as exc:
    print(\"connect=refused:\" + type(exc).__name__)
" "$_PILOT_EGRESS_SOCK" 2>/dev/null)
        printf "%s\n" "$sandboxed" | grep "^connect=" | head -1

        kill -TERM "$pid" 2>/dev/null
        i=0
        while [ $i -lt 50 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.1; i=$((i + 1)); done
        echo "sock_after=$([ -e "$_PILOT_EGRESS_SOCK" ] && echo present || echo absent)"
    ' _ "$lib" "$root" "$RELAY" "$SENTINEL_NAME" "$SENTINEL_VALUE"
}

field() { printf '%s\n' "$2" | sed -n "s/^$1=//p" | head -1; }

echo ""
echo "POSITIVE — the real launcher, the real relay, a real sandbox"
echo "------------------------------------------------------------"
OUT=$(probe "$DISPATCH_LIB" "$TMPROOT/pos")
assert_eq "the launcher reports the relay up" "0" "$(field launch_rc "$OUT")"
assert_eq "the launcher names the relay pid" "yes" "$(field pid_found "$OUT")"
assert_eq "the relay socket is mode 600" "600" "$(field sock_mode "$OUT")"
assert_eq "the caller's sentinel does NOT reach the relay" "no" "$(field env_sentinel "$OUT")"
assert_eq "PATH reaches the relay (its shebang resolves python3)" "yes" "$(field env_path "$OUT")"
assert_eq "HOME reaches the relay (credentials path)" "yes" "$(field env_home "$OUT")"
assert_eq "MIKA_EGRESS_DEBUG reaches the relay unchanged" "1" "$(field env_debug "$OUT")"
assert_eq "a bwrap pilot, same uid, connects to the 0600 socket" "ok" "$(field connect "$OUT")"
assert_eq "SIGTERM leaves no socket behind" "absent" "$(field sock_after "$OUT")"
assert_eq "no variable outside the launcher allowlist reaches the relay" "none" "$(field env_outside_allowlist "$OUT")"

echo ""
echo "NEGATIVE CONTROL — without env -i, the sentinel reaches the relay"
echo "------------------------------------------------------------------"
OUT_NEG=$(probe "$MUTATED_LIB" "$TMPROOT/neg")
assert_eq "the mutated launcher still brings the relay up" "yes" "$(field pid_found "$OUT_NEG")"
assert_eq "the caller's sentinel DOES reach the relay" "yes" "$(field env_sentinel "$OUT_NEG")"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
