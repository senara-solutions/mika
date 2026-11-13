#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-pilot-instruction-paths.sh (mika#2108).
#
# "Delete the thing the guard protects; confirm the guard goes red."
#
# A guard nobody has watched go red is a decoration — mika#2103 is the incident
# where a lint stayed green through 26 production panics because it knew one
# spelling of the defect. mika#2108 AC2 asks for both directions explicitly, so
# both are here: the repository's real surface passes, and a file that
# reintroduces a host path fails, in each of the shapes it can be reintroduced.
#
# Two properties are pinned that a fixture-only harness cannot reach, so half
# this suite builds a THROWAWAY GIT REPOSITORY and runs the guard inside it:
#
#   * the exemption is narrow — it covers the `[kg].docs_roots` array entries in
#     crates/mika-agent/CLAUDE.md and nothing else, not even another line of
#     that same file;
#   * the surface is what the repository TRACKS — an untracked file is not
#     scanned. That is not an optimisation: a dispatch worktree's
#     .claude/commands/ carries the meta-repo's seeded orchestration commands
#     (mika#1173/#1415), and a guard that went red on another repository's
#     files would be disabled rather than obeyed.
#
# The battery carries accented fixtures on purpose. This repository writes its
# plans, tickets and logs in French and its worktree paths follow; an
# ASCII-only fixture set tests a population that does not exist here.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-pilot-instruction-paths.sh"

# Hermeticity: the fixture repositories below `git init` + `git commit` and
# would otherwise inherit the host's signing config and default branch name.
export GIT_CONFIG_COUNT=4
export GIT_CONFIG_KEY_0=commit.gpgsign
export GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=init.defaultBranch
export GIT_CONFIG_VALUE_1=main
export GIT_CONFIG_KEY_2=user.name
export GIT_CONFIG_VALUE_2=mika2108 test suite
export GIT_CONFIG_KEY_3=user.email
export GIT_CONFIG_VALUE_3=test@localhost

PASS=0
FAIL=0

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

# Run the guard over explicit files (fixture mode); assert its exit equals $1.
assert_files_exit() {
    local want="$1" name="$2"; shift 2
    local got=0
    bash "$GUARD" "$@" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"; PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"; FAIL=$((FAIL + 1))
    fi
}

assert_output_contains() {
    local needle="$1" name="$2"; shift 2
    local out
    out="$(bash "$GUARD" "$@" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"; PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"; FAIL=$((FAIL + 1))
        echo "      got: $out"
    fi
}

# A single-file fixture at relative path $2 (default CLAUDE.md) with body $1.
make_fixture() {
    local body="$1" rel="${2:-CLAUDE.md}" dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    mkdir -p "$dir/$(dirname "$rel")"
    printf '%s\n' "$body" > "$dir/$rel"
    echo "$dir/$rel"
}

# A throwaway repository carrying a copy of the guard, so the git-mode surface
# enumeration, the exemption and the dead-exemption check all run for real.
# Echoes the repo path. Nothing is committed yet — the caller stages what it
# wants tracked.
make_repo() {
    local dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    mkdir -p "$dir/scripts"
    cp "$GUARD" "$dir/scripts/"
    git -C "$dir" init --quiet
    echo "$dir"
}

run_in_repo() {
    local dir="$1"
    ( cd "$dir" && bash scripts/check-pilot-instruction-paths.sh 2>&1 ) || return $?
}

assert_repo_exit() {
    local dir="$1" want="$2" name="$3"
    local got=0
    run_in_repo "$dir" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"; PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"; FAIL=$((FAIL + 1))
        echo "      output: $(run_in_repo "$dir" 2>&1 || true)"
    fi
}

# The six-line recipe the single declared exemption exists for, verbatim in
# shape. Reused by several cases below.
DOCS_ROOTS_RECIPE='[kg]
enabled = true
docs_roots = [
  "/data/workspace/mika-platform/mika/docs/solutions",
  "/data/workspace/mika-platform/mika-cloud/docs/solutions",
]'

echo "── 1. POSITIVE CONTROL — the repository's own surface is clean"
assert_files_exit 0 "the real repository passes with no arguments"
assert_output_contains "exempt line(s) skipped" \
    "the clean report states how many lines the exemption skipped"

echo ""
echo "── 2. NEGATIVE CONTROL — a reintroduced host path goes red"
f="$(make_fixture 'Run the sweep from `/data/workspace/mika-platform/mika` before dispatching.')"
assert_files_exit 1 "a CLAUDE.md citing the host workspace is rejected" "$f"
assert_output_contains "/data/workspace/mika-platform/mika" \
    "the failure reproduces the offending line verbatim" "$f"

f="$(make_fixture 'Inbox lives at /data/workspace/mika-platform/.claude/inbox/' '.claude/commands/inbox.md')"
assert_files_exit 1 "a slash command citing the host workspace is rejected" "$f"

echo ""
echo "── 3. THE RULE IS THE ROOT, NOT ONE REPOSITORY UNDER IT"
# mika#2103: a guard that knows one spelling of a defect protects nothing else.
for other in autre-repo mika-cloud scratch; do
    f="$(make_fixture "See /data/workspace/$other/docs for the rest.")"
    assert_files_exit 1 "a sibling checkout /data/workspace/$other is rejected too" "$f"
done

echo ""
echo "── 4. NO FALSE POSITIVES — a relative instruction is what we asked for"
f="$(make_fixture 'Run the sweep from the working root: `docs/solutions/`. Never an absolute host path.')"
assert_files_exit 0 "a worktree-relative instruction passes" "$f"

f="$(make_fixture 'Le pilote démarre à la racine de travail — jamais ailleurs. Cf. `docs/plans/`.')"
assert_files_exit 0 "an accented, host-path-free file passes" "$f"

f="$(make_fixture 'Cf. `/data/workspace/mika-platform/mika` — chemin hôte.' "dépôt d'essai/CLAUDE.md")"
assert_files_exit 1 "an accented path with an accented body is still rejected" "$f"
assert_output_contains "chemin hôte" \
    "the failure message reproduces the accents verbatim" "$f"

echo ""
echo "── 5. THE EXEMPTION IS NARROW — file AND line shape, or it leaks"
# The exact exempt line shape, in a file that is not the exempt file.
f="$(make_fixture "$DOCS_ROOTS_RECIPE")"
assert_files_exit 1 "the docs_roots shape is NOT exempt in another file" "$f"

repo="$(make_repo)"
mkdir -p "$repo/crates/mika-agent"
printf '%s\n' "$DOCS_ROOTS_RECIPE" > "$repo/crates/mika-agent/CLAUDE.md"
git -C "$repo" add -A >/dev/null 2>&1
assert_repo_exit "$repo" 0 "the docs_roots recipe IS exempt in crates/mika-agent/CLAUDE.md"

# ...and the exemption does not spread to a different line of that same file.
printf '\nGo look in /data/workspace/mika-platform/mika/crates for the rest.\n' \
    >> "$repo/crates/mika-agent/CLAUDE.md"
git -C "$repo" add -A >/dev/null 2>&1
assert_repo_exit "$repo" 1 "another host path in the SAME exempt file is still rejected"

echo ""
echo "── 6. A DEAD EXEMPTION IS A SILENT WIDENING — it must say so"
repo="$(make_repo)"
printf 'No host paths here at all.\n' > "$repo/CLAUDE.md"
git -C "$repo" add -A >/dev/null 2>&1
assert_repo_exit "$repo" 4 "an exemption that matches nothing exits 4, not 0"

echo ""
echo "── 7. THE SURFACE IS WHAT THE REPOSITORY TRACKS"
# A dispatch worktree carries the meta-repo's seeded .claude/commands/, shielded
# from git status. Scanning them would make this guard red on files this repo
# cannot fix — and a guard red where it cannot be obeyed gets disabled.
repo="$(make_repo)"
mkdir -p "$repo/crates/mika-agent"
printf '%s\n' "$DOCS_ROOTS_RECIPE" > "$repo/crates/mika-agent/CLAUDE.md"
git -C "$repo" add -A >/dev/null 2>&1
mkdir -p "$repo/.claude/commands"
printf 'ls -1t /data/workspace/mika-platform/.claude/inbox/\n' \
    > "$repo/.claude/commands/seeded-from-the-meta-repo.md"
assert_repo_exit "$repo" 0 "an UNTRACKED seeded command is not scanned"
git -C "$repo" add -A >/dev/null 2>&1
assert_repo_exit "$repo" 1 "...and the moment the repo tracks it, it is"

echo ""
echo "── 8. NOTHING TO CHECK IS NOT THE SAME AS CLEAN"
repo="$(make_repo)"
git -C "$repo" add -A >/dev/null 2>&1
assert_repo_exit "$repo" 3 "a repository with no auto-loaded surface exits 3, not 0"

assert_files_exit 2 "an unreadable file exits 2, not 0" "/nonexistent/path/CLAUDE.md"

echo ""
echo "check-pilot-instruction-paths anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
