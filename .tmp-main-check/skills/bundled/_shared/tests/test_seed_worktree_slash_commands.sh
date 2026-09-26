#!/bin/bash
# Regression reproducer for mika#1415 — worktree-setup must not clobber a
# sub-repo's tracked .claude/commands/ nor dirty git status when seeding the
# meta-repo orchestration slash commands.
#
# Before the fix, dispatch-lib seeded commands with a blanket
#   cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"
# which (1) overwrote the sub-repo's polymorphic /mika (the mika#1255 fix) with
# the 260-line meta-repo dispatcher, and (2) dropped ~18 untracked sibling
# command files into the tracked tree — dirtying `git status` and breaking the
# resume rebase ("cannot rebase: You have unstaged changes").
#
# This test exercises _seed_worktree_slash_commands() against a real linked
# worktree and asserts the AC3 contract: clean worktree in, clean worktree out,
# meta-only commands available, sub-repo's own commands preserved byte-for-byte.
#
# Run:      bash skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: [$expected]"
        echo "    actual:   [$actual]"
    fi
}

# Source the library to obtain _seed_worktree_slash_commands (function defs only;
# dispatch-lib.sh runs no top-level code on source).
# shellcheck disable=SC1090
source "$DISPATCH_LIB"

# --- Hermetic fixtures -------------------------------------------------------
TMP="$(mktemp -d "${TMPDIR:-/tmp}/test-seed-cmds.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# Platform meta-repo command set (the cp SOURCE): a dispatcher mika.md, a
# sub-repo-colliding mika-issue.md, and two meta-only orchestration commands.
PLATFORM="$TMP/platform"
mkdir -p "$PLATFORM/.claude/commands"
printf 'META DISPATCHER mika.md (260-line equivalent)\n' > "$PLATFORM/.claude/commands/mika.md"
printf 'META mika-issue.md\n'                            > "$PLATFORM/.claude/commands/mika-issue.md"
printf 'META mika-groom-ticket.md\n'                     > "$PLATFORM/.claude/commands/mika-groom-ticket.md"
printf 'META mika-handsoff.md\n'                         > "$PLATFORM/.claude/commands/mika-handsoff.md"

# Sub-repo with its OWN tracked .claude/commands/ (polymorphic /mika + scoped
# /mika-issue), then a linked worktree off it — matching the real dispatch shape
# where common-dir != per-worktree git-dir.
SUBREPO="$TMP/subrepo"
mkdir -p "$SUBREPO/.claude/commands"
git -C "$SUBREPO" init -q
git -C "$SUBREPO" config user.email t@t.t
git -C "$SUBREPO" config user.name t
printf 'POLYMORPHIC sub-repo /mika (72-line equivalent)\n' > "$SUBREPO/.claude/commands/mika.md"
printf 'SUBREPO mika-issue.md\n'                           > "$SUBREPO/.claude/commands/mika-issue.md"
git -C "$SUBREPO" add -A
git -C "$SUBREPO" commit -qm init

WT="$TMP/wt"
git -C "$SUBREPO" worktree add -q "$WT" -b dispatch-branch

POLY_BEFORE="$(cat "$WT/.claude/commands/mika.md")"

echo "Test: _seed_worktree_slash_commands preserves #1255 and keeps the worktree clean"

# Baseline must be clean (sanity).
assert_eq "baseline worktree clean" "" "$(git -C "$WT" status --porcelain)"

# --- Exercise the function under test ---------------------------------------
_seed_worktree_slash_commands "$PLATFORM" "$WT"

# AC3 core: worktree stays clean after seeding.
assert_eq "worktree clean after seed" "" "$(git -C "$WT" status --porcelain)"

# Invariant 1: the sub-repo's tracked commands are NOT overwritten.
assert_eq "polymorphic mika.md preserved" "$POLY_BEFORE" "$(cat "$WT/.claude/commands/mika.md")"
assert_eq "sub-repo mika-issue.md preserved" "SUBREPO mika-issue.md" "$(cat "$WT/.claude/commands/mika-issue.md")"

# mika#1173: meta-only orchestration commands ARE available to the inner session.
assert_eq "meta-only mika-groom-ticket.md seeded" "META mika-groom-ticket.md" "$(cat "$WT/.claude/commands/mika-groom-ticket.md" 2>/dev/null || echo MISSING)"
assert_eq "meta-only mika-handsoff.md seeded" "META mika-handsoff.md" "$(cat "$WT/.claude/commands/mika-handsoff.md" 2>/dev/null || echo MISSING)"

# Invariant 2: the meta-only copies are shielded via the common-dir exclude.
COMMON_DIR="$(git -C "$WT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || git -C "$WT" rev-parse --git-common-dir)"
assert_eq "scaffold shielded in common-dir exclude" "yes" \
    "$(grep -qxF '.claude/commands/mika-groom-ticket.md' "$COMMON_DIR/info/exclude" 2>/dev/null && echo yes || echo no)"

# Idempotency: a second seed (resume / re-dispatch) keeps it clean and adds no
# duplicate exclude lines.
_seed_worktree_slash_commands "$PLATFORM" "$WT"
assert_eq "worktree clean after re-seed" "" "$(git -C "$WT" status --porcelain)"
assert_eq "no duplicate exclude entry" "1" \
    "$(grep -cxF '.claude/commands/mika-groom-ticket.md' "$COMMON_DIR/info/exclude")"

# --- Negative control: prove the gate would CATCH the old `cp -r` regression --
# A disposable second worktree seeded the OLD way must come out dirty AND with a
# clobbered mika.md. This pins the regression-catching property into the suite
# rather than relying on an external manual run — if a future refactor silently
# reverted to a blanket copy, these two assertions flip and fail the gate.
NEG="$TMP/wt-neg"
git -C "$SUBREPO" worktree add -q "$NEG" -b neg-control
cp -r "$PLATFORM/.claude/commands" "$NEG/.claude/"
assert_eq "OLD cp -r dirties the worktree (regression is detectable)" "dirty" \
    "$([ -z "$(git -C "$NEG" status --porcelain)" ] && echo clean || echo dirty)"
assert_eq "OLD cp -r clobbers polymorphic mika.md (regression is detectable)" \
    "META DISPATCHER mika.md (260-line equivalent)" "$(cat "$NEG/.claude/commands/mika.md")"

echo ""
echo "Passed: $PASS  Failed: $FAIL"
[ "$FAIL" -eq 0 ]
