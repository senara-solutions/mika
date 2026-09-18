#!/bin/bash
# Test suite for mika#2348 — what the loop ships must be fmt-clean.
#
# WHY. Two PRs produced by the loop on 2026-09-16 failed the CI `Check` job on
# `cargo fmt --all -- --check` and cost an operator a manual `cargo fmt`:
#
#   #2344 (feat/2335) → fixed by d062f716, "cargo fmt seul"
#   #2345 (bug/2286)  → fixed by e2c7eef3, "cargo fmt seul"
#
# The ticket named one cause — "the mika#1282 rescue commits without running
# cargo fmt" — and that site has in fact run `cargo fmt --all` since mika#1336.
# Reading the two commits the manual fixes had to reformat moves the cause:
#
#   T1  #2344 → `868e90e4 wip(mika#2335): trailing content after pilot end_turn
#       (mika#1383)` — the SIBLING rescue site, which never received mika#1336.
#   T2  #2345 → `0931d86c refactor(2286): …` — no `wip(` prefix, no rescue
#       anywhere near it: a plain commit by the pilot session. The lefthook
#       `rust-fmt` gate that should have caught it is declared in `lefthook.yml`
#       and is NOT installed on the dispatch machine (no `.git/hooks/`, no
#       `core.hooksPath` in any scope), so it has never run.
#
# A remedy confined to the rescue sites closes T1 and leaves T2 entirely open.
# Hence two mechanisms, and this suite covers both plus the two halves that are
# easy to lose: the no-op (T-d) and the perimeter (T-f).
#
# IT CALLS THE REAL FUNCTIONS against a real throwaway crate — never a
# reimplementation of the rescue, which could not falsify the shipped code
# (same discipline as test_dev_groom_dirty_rescue.sh).
#
# COST, AND WHY IT IS NOT CLIPPY'S (mika#2348 D7). The sibling suites announce
# "No network / cargo / clippy required" and this one does depend on `cargo fmt`
# — deliberately: a stub that simulates formatting proves only that something
# was called, not the property the ticket asks for ("assert the rescue commit is
# fmt-clean"). The price is small and worth naming so it is not confused with
# clippy's: **`cargo fmt` does not compile**. A five-line `src/lib.rs` with no
# dependencies renders in tenths of a second, with no network and no registry.
# With no `cargo` on PATH the suite SKIPS with a message rather than failing — a
# test that goes red on a machine without a toolchain teaches the wrong thing.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_fmt_clean.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the rescue's `2>&9` redirects have a destination.
exec 9>/dev/null

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 400)'"
    fi
}

# ============================================================================
# Toolchain gate (D7): skip loudly rather than fail on a machine without cargo.
# ============================================================================
if ! command -v cargo >/dev/null 2>&1; then
    echo ""
    echo "SKIP: mika#2348 fmt suite needs \`cargo fmt\` and cargo is not on PATH."
    echo "      The property under test is 'the committed tree is fmt-clean',"
    echo "      which a stub cannot establish. Install a Rust toolchain to run it."
    exit 0
fi

# Unformatted on purpose: extra spaces, no spacing around `->`, body on the
# signature line. `cargo fmt --all -- --check` rejects this shape.
UNFORMATTED_RS='pub fn  add( a:i32 ,b:i32 )->i32{a+b}'
FORMATTED_MARKER='pub fn add(a: i32, b: i32) -> i32 {'

# A throwaway crate with a real `origin/main`, because _normalize_committed_rust
# resolves the branch perimeter through `git merge-base origin/main HEAD`.
# Layout: `src/legacy.rs` is committed on main, unformatted, and never touched by
# the branch — it is the out-of-perimeter control for T-f.
make_crate() {
    local root repo bare
    root="$(mktemp -d "${TMPDIR:-/tmp}/mika-2348-test.XXXXXX")"
    bare="$root/origin.git"
    repo="$root/work"

    git init -q --bare "$bare"
    git init -q -b main "$repo"
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    git -C "$repo" config commit.gpgsign false

    mkdir -p "$repo/src"
    cat > "$repo/Cargo.toml" <<'TOML'
[package]
name = "mika2348_fixture"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
TOML
    printf 'pub mod legacy;\n' > "$repo/src/lib.rs"
    # Unformatted, on main, out of every branch diff below.
    printf '%s\n' "$UNFORMATTED_RS" > "$repo/src/legacy.rs"

    git -C "$repo" add -A
    git -C "$repo" commit -q -m "seed" --no-verify
    git -C "$repo" remote add origin "$bare"
    git -C "$repo" push -q origin main
    git -C "$repo" checkout -q -b "fix/2348/fmt"

    printf '%s' "$repo"
}

crate_root_of() { dirname "$1"; }

# `cargo fmt --all -- --check` against the COMMITTED tree, not the worktree:
# checkout HEAD into a scratch dir and ask there. That is the property the CI
# `Check` job measures, and asking the worktree would pass on a tree whose
# formatting was never committed.
head_is_fmt_clean() {
    local repo="$1" scratch
    scratch="$(mktemp -d "${TMPDIR:-/tmp}/mika-2348-head.XXXXXX")"
    git -C "$repo" archive HEAD | tar -x -C "$scratch" 2>/dev/null
    if (cd "$scratch" && cargo fmt --all -- --check >/dev/null 2>&1); then
        rm -rf "$scratch"; echo "clean"
    else
        rm -rf "$scratch"; echo "dirty"
    fi
}

# Same question, scoped to ONE committed file.
#
# WHY IT HAS TO EXIST. `head_is_fmt_clean` asks about the whole workspace, and
# for _normalize_committed_rust that is the wrong question by construction: D3
# scopes it to the branch diff, so `src/legacy.rs` — unformatted on main, never
# touched by the branch — stays unformatted on purpose (T-f asserts exactly
# that). A workspace-wide assertion on T-c would therefore fail on the very
# decision the ticket took, and "fixing" it would mean dragging main's debt into
# a rescue PR. Compares the file before and after a `cargo fmt --all` run on a
# scratch copy of HEAD, so it needs no `rustfmt` on PATH beyond cargo's own.
file_at_head_is_fmt_clean() {
    local repo="$1" path="$2" scratch
    scratch="$(mktemp -d "${TMPDIR:-/tmp}/mika-2348-file.XXXXXX")"
    git -C "$repo" archive HEAD | tar -x -C "$scratch" 2>/dev/null
    cp "$scratch/$path" "$scratch/.before" 2>/dev/null
    (cd "$scratch" && cargo fmt --all >/dev/null 2>&1) || true
    if cmp -s "$scratch/$path" "$scratch/.before"; then
        rm -rf "$scratch"; echo "clean"
    else
        rm -rf "$scratch"; echo "dirty"
    fi
}

# Set the globals the rescue reads, for the zero-commit shape (HEAD unchanged).
set_globals() {
    local repo="$1"
    WORKTREE_DIR="$repo"
    SKILL="dev-pilot"
    REPO="mika"
    ISSUE_NUM="2348"
    BRANCH="fix/2348/fmt"
    SESSION_ID="sess-test"
    PILOT_EXIT=1
    STATUS=""
    PR_URL=""
    RESULT="claude-pilot completed (status: terminated)."
    RESCUED_DIRTY_WORKTREE=""
    RESCUE_COMMITS=""
    RESCUE_COMMITS_SIGNALLED=""
    LOG_ID="log-test"
}

# `_post_flight_recovery` signals into an open PR through `gh`. Shadow it so the
# suite stays offline; a shell function wins over the external command for every
# call site in the sourced lib.
gh() { return 1; }

# ============================================================================
# T-a: dirty worktree, unformatted .rs, HEAD unchanged
#      → the mika#1282 rescue commit is fmt-clean.
#      NON-REGRESSION on mika#1336 — this is the site that already worked.
# ============================================================================
echo ""
echo "T-a: mika#1282 dirty-worktree rescue → commit is fmt-clean"
echo "----------------------------------------------------------"
REPO_DIR="$(make_crate)"
printf 'pub mod legacy;\n%s\n' "$UNFORMATTED_RS" > "$REPO_DIR/src/lib.rs"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
POST_RUN_HEAD="$PRE_RUN_HEAD"
_rescue_dirty_worktree || true
assert_eq "T-a: HEAD advanced (content rescued)" "1" \
    "$( [ "$PRE_RUN_HEAD" != "$(git -C "$REPO_DIR" rev-parse HEAD)" ] && echo 1 || echo 0 )"
assert_eq "T-a: the rescue commit is fmt-clean (AC1)" "clean" "$(head_is_fmt_clean "$REPO_DIR")"
assert_contains "T-a: the rescued content is really in the commit" "$FORMATTED_MARKER" \
    "$(git -C "$REPO_DIR" show HEAD:src/lib.rs)"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-b: HEAD advanced + trailing dirty unformatted .rs
#      → the mika#1383 Phase A commit is fmt-clean.
#      THIS IS #2344 / T1 — the site the ticket's literal remedy would have missed.
# ============================================================================
echo ""
echo "T-b: mika#1383 trailing-content rescue → commit is fmt-clean (closes #2344)"
echo "---------------------------------------------------------------------------"
REPO_DIR="$(make_crate)"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
# The pilot committed something (HEAD advances), then left more content dirty.
printf 'pub mod legacy;\npub fn seed() {}\n' > "$REPO_DIR/src/lib.rs"
git -C "$REPO_DIR" add -A
git -C "$REPO_DIR" commit -q -m "feat(mika#2348): pilot commit" --no-verify
POST_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'pub mod legacy;\npub fn seed() {}\n%s\n' "$UNFORMATTED_RS" > "$REPO_DIR/src/lib.rs"
_post_flight_recovery || true
assert_eq "T-b: trailing content was committed" "1" \
    "$(git -C "$REPO_DIR" log --format=%s | grep -c 'trailing content after pilot end_turn')"
assert_eq "T-b: the trailing-content commit is fmt-clean (AC2)" "clean" "$(head_is_fmt_clean "$REPO_DIR")"
assert_contains "T-b: the trailing content is really in the tree" "$FORMATTED_MARKER" \
    "$(git -C "$REPO_DIR" show HEAD:src/lib.rs)"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-c: the PILOT committed unformatted .rs, worktree clean
#      → a style() commit normalizes it before the branch is published.
#      THIS IS #2345 / T2 — the half the ticket's literal remedy did not cover.
# ============================================================================
echo ""
echo "T-c: pilot's own unformatted commit → normalized (closes #2345)"
echo "---------------------------------------------------------------"
REPO_DIR="$(make_crate)"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'pub mod legacy;\n%s\n' "$UNFORMATTED_RS" > "$REPO_DIR/src/lib.rs"
git -C "$REPO_DIR" add -A
git -C "$REPO_DIR" commit -q -m "refactor(mika#2348): pilot commit, never fmt-ed" --no-verify
POST_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
assert_eq "T-c: precondition — the pilot's commit is NOT fmt-clean" "dirty" "$(head_is_fmt_clean "$REPO_DIR")"
_post_flight_recovery || true
assert_eq "T-c: a style() commit was produced" "1" \
    "$(git -C "$REPO_DIR" log --format=%s | grep -c 'normalisation cargo fmt post-flight')"
# Scoped to the branch's own file, not the workspace: `src/legacy.rs` is
# unformatted on main and out of this branch's diff, so D3 leaves it alone — and
# a workspace-wide assertion here would fail on the decision itself. T-f is
# where that boundary is asserted head-on.
assert_eq "T-c: the pilot's committed file is fmt-clean on the published branch (AC3)" \
    "clean" "$(file_at_head_is_fmt_clean "$REPO_DIR" "src/lib.rs")"
assert_eq "T-c: POST_RUN_HEAD advanced so the push sees the style commit" \
    "$(git -C "$REPO_DIR" rev-parse HEAD)" "$POST_RUN_HEAD"
assert_eq "T-c: the style commit is recorded for the mika#2151 signal" "1" \
    "$(grep -c "$(git -C "$REPO_DIR" rev-parse HEAD)" <<<"${RESCUE_COMMITS:-}")"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-d (anti-vacuity): clean tree, already fmt-clean → NO commit at all.
#      The half that is easy to forget: a normalization that fires
#      unconditionally is indistinguishable from one that never fires.
# ============================================================================
echo ""
echo "T-d: clean + already fmt-clean → no commit (AC5)"
echo "------------------------------------------------"
REPO_DIR="$(make_crate)"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'pub mod legacy;\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n' > "$REPO_DIR/src/lib.rs"
git -C "$REPO_DIR" add -A
git -C "$REPO_DIR" commit -q -m "feat(mika#2348): pilot commit, already clean" --no-verify
POST_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
BEFORE_HEAD="$POST_RUN_HEAD"
_post_flight_recovery || true
assert_eq "T-d: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq "T-d: no style() commit exists" "0" \
    "$(git -C "$REPO_DIR" log --format=%s | grep -c 'normalisation cargo fmt post-flight')"
assert_eq "T-d: worktree left clean" "" "$(git -C "$REPO_DIR" status --porcelain)"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-e: syntactically invalid Rust → cargo fmt fails, the rescue STILL commits.
#      D4: the rescue is salvage, not a gate. A `set -e` on the fmt would lose
#      the pilot's content; a failed fmt only leaves CI red, which is today.
# ============================================================================
echo ""
echo "T-e: invalid Rust → fmt fails, content is preserved anyway (AC6)"
echo "----------------------------------------------------------------"
REPO_DIR="$(make_crate)"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
POST_RUN_HEAD="$PRE_RUN_HEAD"
# A pilot interrupted mid-file: unbalanced braces, rustfmt cannot parse it.
printf 'pub mod legacy;\npub fn broken( {\n' > "$REPO_DIR/src/lib.rs"
FMT_STDERR="$( { _rescue_dirty_worktree || true; } 2>&1 >/dev/null )"
assert_eq "T-e: HEAD advanced — content was preserved" "1" \
    "$( [ "$PRE_RUN_HEAD" != "$(git -C "$REPO_DIR" rev-parse HEAD)" ] && echo 1 || echo 0 )"
assert_contains "T-e: the pilot's broken content is in the commit" "pub fn broken( {" \
    "$(git -C "$REPO_DIR" show HEAD:src/lib.rs)"
assert_contains "T-e: the failure is named on stderr, not swallowed" "rescue_fmt_failed" "$FMT_STDERR"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-f: an unformatted file OUTSIDE the branch diff must not enter the commit.
#      D3: `cargo fmt --all` walks the workspace; with no hook installed, main
#      plausibly carries unformatted files unrelated to this branch. Dragging
#      them in would inflate an already fragile rescue PR.
# ============================================================================
echo ""
echo "T-f: out-of-perimeter file is restored, not committed (AC7)"
echo "-----------------------------------------------------------"
REPO_DIR="$(make_crate)"
set_globals "$REPO_DIR"
PRE_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'pub mod legacy;\n%s\n' "$UNFORMATTED_RS" > "$REPO_DIR/src/lib.rs"
git -C "$REPO_DIR" add -A
git -C "$REPO_DIR" commit -q -m "refactor(mika#2348): pilot commit, never fmt-ed" --no-verify
POST_RUN_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
_normalize_committed_rust || true
assert_eq "T-f: in-perimeter src/lib.rs WAS normalized" "1" \
    "$(git -C "$REPO_DIR" log -1 --name-only --format= | grep -c '^src/lib.rs$')"
assert_eq "T-f: out-of-perimeter src/legacy.rs did NOT enter the commit" "0" \
    "$(git -C "$REPO_DIR" log -1 --name-only --format= | grep -c '^src/legacy.rs$')"
assert_eq "T-f: out-of-perimeter file restored, worktree left clean" "" \
    "$(git -C "$REPO_DIR" status --porcelain)"
assert_contains "T-f: src/legacy.rs is still its unformatted self on disk" "$UNFORMATTED_RS" \
    "$(cat "$REPO_DIR/src/legacy.rs")"
rm -rf "$(crate_root_of "$REPO_DIR")"

# ============================================================================
# T-g (static): every content-bearing `git commit` in dispatch-lib.sh is
#      preceded, inside its own function, by a formatting gesture.
#
#      WHY STATIC. The regression to prevent is not "a decision becomes wrong"
#      but "a commit producer is not covered" — and every behavioural test stays
#      green while that runs. That is literally what happened between mika#1336
#      and mika#2348: the mika#1383 site was added, all suites passed, and #2344
#      shipped unformatted anyway. A future third rescue site would repeat it.
#
#      `--allow-empty` commits are excluded: the mika#1383 auto-PR-create marker
#      carries no content, so there is nothing to format.
# ============================================================================
echo ""
echo "T-g: static guard — every content commit passes through a fmt (AC8)"
echo "--------------------------------------------------------------------"
GUARD_OUT=$(awk '
    # Track the enclosing function and whether a fmt was seen inside it, before
    # the commit line. Top-level `}` closes the function.
    /^[a-zA-Z_][a-zA-Z0-9_]*\(\)[ \t]*\{/ { fn = $0; sub(/\(\).*/, "", fn); fmt_line = 0; next }
    /^\}/ { fn = ""; fmt_line = 0; next }
    /_fmt_and_stage_rust/ || /cargo fmt --all/ { if (fn != "") fmt_line = NR }
    /git[^|]*commit / {
        if ($0 ~ /--allow-empty/) next          # marker commit, carries no content
        if ($0 !~ /commit -m/) next             # not a commit invocation
        if (fn == "" || fmt_line == 0) {
            printf "UNCOVERED %d %s\n", NR, (fn == "" ? "<top-level>" : fn)
        }
    }
' "$DISPATCH_LIB")
assert_eq "T-g: no content commit bypasses formatting" "" "$GUARD_OUT"
# Anti-vacuity for the guard itself: it must actually be looking at commit sites.
assert_eq "T-g: the guard sees the four content-commit sites it is meant to cover" "4" \
    "$(grep -c 'git -C "\$WORKTREE_DIR" commit -m' "$DISPATCH_LIB")"

echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1
