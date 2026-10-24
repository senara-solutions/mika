#!/usr/bin/env bash
#
# Offline harness for the major-change guard of install.sh (mika#2897).
#
# From v1.0 Mika follows strict semver: a breaking change ships as a major
# version with a migration guide. install.sh replaces the binary in place, and
# the repository's most common path — `curl … | sh`, no argument — is exactly
# the one that would carry someone from 1.x to 2.x without a word. They would
# learn about the migration when their ~/.mika stopped opening.
#
# WHAT THIS SUITE MEASURES, AND WHY IT MEASURES IT THAT WAY. The property under
# test is a negative one: *nothing was replaced*. Inferring it from a successful
# install would mean serving an archive and a matching checksum, i.e. putting
# the download and the checksum verification under test — two behaviours
# mika#2897 does not touch. So the suite intercepts `curl` on the PATH and reads
# the log of its calls instead. "The guard refused" is then three facts of
# opposite sign: a non-zero exit, no `releases/download` URL ever requested, and
# the sentinel line of the fake binary still in place. "The guard let through"
# is one fact: a `releases/download` URL was requested. The script dies on the
# stubbed curl right after, and that is immaterial — the question asked is
# whether the guard let through, not whether the install completes.
#
# No testability hook enters install.sh for this: no --dry-run, no
# MIKA_INSTALL_GATE_ONLY. A mode that exists only for the test proves nothing
# about the real path.
#
# Usage: test-install-major-gate.sh [path/to/install.sh]
# The optional argument points the suite at another copy of install.sh. It
# exists for the red-before proof — a copy with the guard block removed must
# redden the refusal rows — and defaults to the repository's own install.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="${1:-$REPO_ROOT/install.sh}"

if [[ ! -f "$SCRIPT" ]]; then
    echo "ERROR: install.sh not readable: $SCRIPT" >&2
    exit 2
fi

PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Survives in the fake binary iff install.sh never `mv`-ed over it.
SENTINEL="mika#2897 fixture sentinel"

SANDBOX=""
RC=0
OUT=""

# run_case <installed> <latest-tag> [install.sh args...]
#
# <installed> is "none" (no binary in INSTALL_DIR), "unreadable" (a file that is
# not executable), or a version string the fake binary reports.
# <latest-tag> is what the stubbed releases/latest API answers.
run_case() {
    local installed="$1" latest="$2"
    shift 2

    SANDBOX="$(mktemp -d -p "$WORK")"
    mkdir -p "$SANDBOX/bin" "$SANDBOX/install"
    printf '%s\n' "$latest" > "$SANDBOX/latest"
    : > "$SANDBOX/curl.log"

    # The stub logs every call, answers the releases/latest API, and refuses
    # everything else. install.sh resolves `curl` through the PATH (it never
    # calls it by absolute path), so a PATH-leading stub intercepts all three
    # of its call sites.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'printf "%s\n" "$*" >>"$CURL_LOG"' \
        'for arg in "$@"; do' \
        '    if [[ "$arg" == *api.github.com* ]]; then' \
        '        printf "{\n  \"tag_name\": \"%s\"\n}\n" "$(cat "$LATEST_TAG_FILE")"' \
        '        exit 0' \
        '    fi' \
        'done' \
        'exit 1' \
        > "$SANDBOX/bin/curl"
    chmod +x "$SANDBOX/bin/curl"

    case "$installed" in
        none) ;;
        unreadable)
            # Present but not executable: install.sh cannot read a version from
            # it and must say so rather than stay silent.
            printf '%s\n' "not a binary — $SENTINEL" > "$SANDBOX/install/mika"
            chmod 0644 "$SANDBOX/install/mika"
            ;;
        *)
            # `mika X.Y.Z (sha)` — the shape of build_info.rs's version string
            # prefixed with the binary name by clap.
            printf '%s\n' \
                '#!/bin/sh' \
                "# $SENTINEL" \
                "echo \"mika $installed (deadbee)\"" \
                > "$SANDBOX/install/mika"
            chmod +x "$SANDBOX/install/mika"
            ;;
    esac

    RC=0
    OUT="$(
        PATH="$SANDBOX/bin:$PATH" \
        CURL_LOG="$SANDBOX/curl.log" \
        LATEST_TAG_FILE="$SANDBOX/latest" \
        MIKA_INSTALL_DIR="$SANDBOX/install" \
        sh "$SCRIPT" "$@" 2>&1
    )" || RC=$?
}

report() {
    local name="$1" problems="$2"
    if [[ -z "$problems" ]]; then
        PASS=$((PASS + 1)); echo "ok   $name"
    else
        FAIL=$((FAIL + 1))
        echo "FAIL $name (rc=$RC)$problems"
        echo "       output: $(printf '%s' "$OUT" | tr '\n' '|')"
    fi
}

# assert_passes <name> <installed> <latest-tag> [args...]
#
# The guard let through, and on a path AC1 requires to stay silent it also said
# nothing: no warning, no mention of a major change.
assert_passes() {
    local name="$1"; shift
    run_case "$@"
    local problems=""
    grep -q 'releases/download' "$SANDBOX/curl.log" \
        || problems="$problems; no releases/download was requested — the guard refused (mika#2897)"
    [[ "$OUT" != *major* ]] \
        || problems="$problems; the guard spoke on a path AC1 keeps silent (mika#2897)"
    [[ "$OUT" != *Warning:* ]] \
        || problems="$problems; a warning appeared on a path AC1 keeps silent (mika#2897)"
    report "$name" "$problems"
}

# assert_passes_with_warning <name> <installed> <latest-tag> [args...]
#
# The unreadable-version case: fail-open, but never silently (R7).
assert_passes_with_warning() {
    local name="$1"; shift
    run_case "$@"
    local problems=""
    grep -q 'releases/download' "$SANDBOX/curl.log" \
        || problems="$problems; no releases/download was requested — the guard refused (mika#2897)"
    [[ "$OUT" == *"could not read the version"* ]] \
        || problems="$problems; the unreadable installed version passed without a warning (mika#2897)"
    report "$name" "$problems"
}

# assert_refused <name> <shown-installed> <shown-target> <guide-tag> \
#                <installed> <latest-tag> [args...]
assert_refused() {
    local name="$1" shown_installed="$2" shown_target="$3" guide_tag="$4"
    shift 4
    run_case "$@"
    local problems=""
    [[ "$RC" -ne 0 ]] \
        || problems="$problems; exit code was 0, the refusal must be non-zero (mika#2897)"
    ! grep -q 'releases/download' "$SANDBOX/curl.log" \
        || problems="$problems; an archive was requested after the refusal (mika#2897)"
    grep -q "$SENTINEL" "$SANDBOX/install/mika" \
        || problems="$problems; the installed binary was replaced (mika#2897)"
    [[ "$OUT" == *"$shown_installed"* ]] \
        || problems="$problems; the message does not name the installed version $shown_installed (mika#2897)"
    [[ "$OUT" == *"$shown_target"* ]] \
        || problems="$problems; the message does not name the target version $shown_target (mika#2897)"
    [[ "$OUT" == *"docs/releases/$guide_tag.md"* ]] \
        || problems="$problems; the message does not name the migration guide of $guide_tag (mika#2897)"
    report "$name" "$problems"
}

# ── The three cases of AC4 ───────────────────────────────────────────────────
assert_passes  "no installation: nothing to compare, nothing changes" \
    none v1.2.0 v1.2.0
assert_passes  "same major: nothing changes" \
    1.0.0 v1.2.0 v1.2.0
assert_refused "major change: refused before any archive is requested" \
    1.4.0 v2.0.0 v2.0.0 \
    1.4.0 v2.0.0 v2.0.0

# ── The trajectories where a badly placed guard still passes the three above ─
assert_passes  "--allow-major-upgrade lifts the guard (AC3)" \
    1.4.0 v2.0.0 v2.0.0 --allow-major-upgrade
assert_refused "a major going backwards is a major change too (D6)" \
    2.1.0 v1.0.0 v1.0.0 \
    2.1.0 v1.0.0 v1.0.0
assert_passes_with_warning "an unreadable installed version fails open, with a warning (R7)" \
    unreadable v2.0.0 v2.0.0
# No version argument: the target comes from the stubbed releases/latest API.
# The guard must judge the version it resolved, not the one it was given (R5).
assert_refused "the guard judges the version resolved by the API, not the argument (R5)" \
    1.0.0 v2.0.0 v2.0.0 \
    1.0.0 v2.0.0
assert_refused "a pre-release target: the major is read before the dash" \
    1.4.0 v2.0.0-rc.1 v2.0.0-rc.1 \
    1.4.0 v2.0.0-rc.1
assert_passes  "the flag may come before the version (D2)" \
    1.4.0 v2.0.0 --allow-major-upgrade v2.0.0
# The semver promise starts at v1.0: a 0.x installation carries none, so the
# guard does not apply to it. 0.12.2 → v1.0.0 is the launch itself — refusing it
# would turn every member's update into an error on launch day. assert_passes
# also checks that no "major" line was printed.
assert_passes  "a 0.x installation upgrades to v1.0.0 untouched: the promise starts at v1.0" \
    0.12.2 v1.0.0 v1.0.0
assert_passes  "a 0.x installation is outside the guard for any target major" \
    0.12.2 v2.0.0 v2.0.0

# ── Anchoring assertions ─────────────────────────────────────────────────────
# A detector whose corpus is a single file can go green by losing its grip: if
# the guard leaves install.sh, every "let through" case stays green by
# construction. These two rows are what rot would redden.
RC=0
OUT=""
if grep -q -- '--allow-major-upgrade' "$SCRIPT"; then
    PASS=$((PASS + 1)); echo "ok   install.sh still carries the --allow-major-upgrade option"
else
    FAIL=$((FAIL + 1))
    echo "FAIL install.sh no longer carries --allow-major-upgrade: the guard or its option left the script (mika#2897)"
fi

if grep -q '\[\[' "$SCRIPT"; then
    FAIL=$((FAIL + 1))
    echo "FAIL install.sh contains '[[': it runs under /bin/sh (dash on Debian and Ubuntu) and must stay POSIX (mika#2897)"
else
    PASS=$((PASS + 1)); echo "ok   install.sh stays POSIX: no '[[' bashism"
fi

echo
echo "install-major-gate: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
