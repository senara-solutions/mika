#!/bin/bash
# mika#2141: git is usable inside the pilot sandbox, and the containment that
# made it unusable is still closed.
#
# WHY THIS TEST HAS THE SHAPE IT HAS. The defect it guards ran undetected for a
# month with an adversarial canary suite already in place, because that suite's
# only git check was `git --version` — a binary-presence probe, green inside a
# namespace where no repository operation worked at all. A test that builds the
# bwrap argv and inspects it would have the same blind spot: the argv was
# always "correct", it was the resulting namespace that had no gitdir in it.
#
# So this suite launches a REAL bwrap, through the REAL `_run_pilot_sandboxed`,
# against a REAL linked worktree of this repository — the only shape in which
# `.git` is a file pointing outside the bind, which is the whole defect.
# Argv-only assertions are explicitly out of contract here.
#
# Both halves run against the same sandbox in the same run, because either half
# alone is vacuous: binding /data/workspace in bulk would pass every must-work
# check, and a sandbox that fails to launch would pass every must-fail one.
#
#   MUST WORK   rev-parse, status, log, add, commit, the remote resolving to
#               https (not ssh — the sandbox has no key by design), and a
#               committer identity (which lives only in the operator's
#               ~/.gitconfig, blanked by --tmpfs /home).
#
#   MUST FAIL   another worktree of the same repository, by path and through
#               `git -C`; and deleting a branch head that is not this
#               worktree's. Each is proven present/possible host-side first, so
#               its failure inside is isolation rather than absence.
#
# Companions, neither subsuming this one:
#   test_sandbox_no_secret_in_argv.sh          — mika#2039 argv channel
#   test-pilot-github-token-not-in-sandbox.sh  — mika#2056 credential absence
#   scripts/canary-pilot-containment           — live post-deploy probe
#
# Run: bash skills/bundled/_shared/tests/test_sandbox_git_usable.sh
# Expected: all assertions pass, exit 0. Skips cleanly when bwrap is absent.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

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
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to contain: '$needle'"; echo "    actual:              '$haystack'"
    fi
}

if ! command -v bwrap >/dev/null 2>&1; then
    echo "⊘ skipped — bwrap not installed on PATH (mika#2141 needs a real sandbox)"
    exit 0
fi
if ! git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    echo "⊘ skipped — $REPO_ROOT is not a git repository"
    exit 0
fi

STAMP="$$-$(date +%s)"
PROBE_BRANCH="probe/2141/sandbox-git-usable-$STAMP"
OTHER_BRANCH="probe/2141/negative-control-$STAMP"
# A disposable deletion target. NEVER refs/heads/main: this assertion exists to
# catch a regression to a wider bind, and on that regression the first `make
# test` run would delete the operator live main with no restore path. Its prefix
# is deliberately NOT the probe branch dirname, so a correct bind refuses it.
VICTIM_BRANCH="victim/2141/$STAMP"

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2141-XXXXXX")
WORKTREE_DIR="$TMPROOT/worktree"
OTHER_WORKTREE="$TMPROOT/other-worktree"

cleanup() {
    git -C "$REPO_ROOT" worktree remove --force "$WORKTREE_DIR" >/dev/null 2>&1 || true
    git -C "$REPO_ROOT" worktree remove --force "$OTHER_WORKTREE" >/dev/null 2>&1 || true
    git -C "$REPO_ROOT" branch -D "$PROBE_BRANCH" >/dev/null 2>&1 || true
    git -C "$REPO_ROOT" branch -D "$OTHER_BRANCH" >/dev/null 2>&1 || true
    git -C "$REPO_ROOT" branch -D "$VICTIM_BRANCH" >/dev/null 2>&1 || true
    # The helper mkdir -p's these under the parent repo; leaving them behind
    # would litter the operator repository on every run.
    if [ -n "${PARENT_GIT_DIR:-}" ]; then
        rmdir "$PARENT_GIT_DIR/refs/heads/probe/2141" "$PARENT_GIT_DIR/refs/heads/probe" \
              "$PARENT_GIT_DIR/logs/refs/heads/probe/2141" "$PARENT_GIT_DIR/logs/refs/heads/probe" \
              "$PARENT_GIT_DIR/refs/heads/victim/2141" "$PARENT_GIT_DIR/refs/heads/victim" \
              >/dev/null 2>&1 || true
    fi
    git -C "$REPO_ROOT" worktree prune >/dev/null 2>&1 || true
    rm -rf "$TMPROOT"
}
trap cleanup EXIT

BASE_REF=$(git -C "$REPO_ROOT" rev-parse HEAD)
if ! git -C "$REPO_ROOT" worktree add --quiet -b "$PROBE_BRANCH" "$WORKTREE_DIR" "$BASE_REF" 2>/dev/null; then
    echo "⊘ skipped — could not create the probe worktree"
    exit 0
fi
git -C "$REPO_ROOT" worktree add --quiet -b "$OTHER_BRANCH" "$OTHER_WORKTREE" "$BASE_REF" 2>/dev/null || true
git -C "$REPO_ROOT" branch --quiet "$VICTIM_BRANCH" "$BASE_REF" 2>/dev/null || true

# The probe worktree must have the defect's shape, or this suite proves nothing.
dotgit_kind="missing"
[ -f "$WORKTREE_DIR/.git" ] && dotgit_kind="file"
[ -d "$WORKTREE_DIR/.git" ] && dotgit_kind="directory"
echo ""
echo "PRECONDITION — the probe worktree reproduces the defect's shape"
echo "---------------------------------------------------------------"
assert_eq "the worktree's .git is a FILE, not a directory" "file" "$dotgit_kind"
gitdir_target=$(sed -n 's/^gitdir: *//p' "$WORKTREE_DIR/.git" 2>/dev/null | head -1)
outside="no"
case "$gitdir_target/" in "$WORKTREE_DIR"/*) outside="no" ;; *) outside="yes" ;; esac
assert_eq "its gitdir points OUTSIDE the worktree bind" "yes" "$outside"

# HOME is redirected so the suite proves the mechanism rather than inheriting
# the operator's real configuration — and so the sandbox's git identity can
# only have arrived through the staged config file under test.
export HOME="$TMPROOT/home"
mkdir -p "$HOME" "$HOME/.mika/data/pilot-transcripts"
cat > "$HOME/.gitconfig" <<'GITCFG'
[user]
	name = Probe 2141
	email = probe-2141@example.invalid
GITCFG

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# No real egress proxy / mitmproxy for a test. Phase 2a (fs cut) exercises the
# fallback bwrap construction — the degraded path, which must carry the gitdir
# binds too; wiring only the Phase 2b branch is a named risk of this ticket.
_ensure_pilot_egress_proxy() { return 1; }
_ensure_pilot_helper() { return 1; }
_PILOT_SANDBOX_SECRET_ALLOWLIST=()

echo ""
echo "MUST WORK — a real repository, inside the real sandbox"
echo "------------------------------------------------------"

got_toplevel=$(_run_pilot_sandboxed /bin/sh -c 'git rev-parse --show-toplevel 2>&1' 2>/dev/null)
assert_eq "git rev-parse --show-toplevel resolves to the worktree" "$WORKTREE_DIR" "$got_toplevel"

got_status=$(_run_pilot_sandboxed /bin/sh -c 'git status -sb 2>&1 | head -1' 2>/dev/null)
assert_contains "git status names the probe branch" "$PROBE_BRANCH" "$got_status"

got_log=$(_run_pilot_sandboxed /bin/sh -c 'git log --oneline -1 2>&1' 2>/dev/null)
assert_contains "git log reads the shared object store" "${BASE_REF:0:8}" "$got_log"

got_ident=$(_run_pilot_sandboxed /bin/sh -c 'git config --get user.email 2>&1' 2>/dev/null)
assert_eq "the staged config carries a committer identity" "probe-2141@example.invalid" "$got_ident"

# The single check that separates "status works" from "the pilot can deliver".
# The parent remote is git@github.com: (SSH) and the sandbox has neither ~/.ssh
# nor SSH_AUTH_SOCK; without the url.insteadOf rewrite, push is dead on a fix
# that passes every other assertion here.
got_url=$(_run_pilot_sandboxed /bin/sh -c 'git ls-remote --get-url origin 2>&1' 2>/dev/null)
case "$got_url" in
    https://*) assert_eq "the remote resolves over https, not ssh" "https" "https" ;;
    *)         assert_eq "the remote resolves over https, not ssh" "https://..." "$got_url" ;;
esac

got_commit=$(_run_pilot_sandboxed /bin/sh -c '
    cd "$(git rev-parse --show-toplevel)" || exit 1
    echo mika2141 > probe-2141.txt
    git add probe-2141.txt >/dev/null 2>&1 || { echo "add-failed"; exit 1; }
    git -c commit.gpgsign=false commit -q --no-verify -m "probe 2141" >/dev/null 2>&1 || { echo "commit-failed"; exit 1; }
    git log --oneline -1 --format=%s
' 2>/dev/null)
assert_eq "git add + git commit succeed inside the sandbox" "probe 2141" "$got_commit"

# The commit must be visible HOST-side on the branch head — proof it reached
# the shared ref store and the object store, not a namespace-local copy that
# evaporates with the sandbox. This is the property a month of wip-rescue
# recoveries was silently substituting for.
host_subject=$(git -C "$REPO_ROOT" log --oneline -1 --format=%s "$PROBE_BRANCH" 2>/dev/null)
assert_eq "the commit is visible host-side on the branch head" "probe 2141" "$host_subject"

echo ""
echo "MUST FAIL — the containment AC2 exists to keep is still closed"
echo "-------------------------------------------------------------"

# Positive controls first: each target is reachable HOST-side, so its failure
# inside the sandbox is isolation and not a path that simply does not exist.
host_other=$( [ -d "$OTHER_WORKTREE" ] && echo present || echo missing )
assert_eq "control: the other worktree exists host-side" "present" "$host_other"
host_victim=$(git -C "$REPO_ROOT" rev-parse --verify --quiet "$VICTIM_BRANCH" >/dev/null 2>&1 && echo yes || echo no)
assert_eq "control: the victim branch exists host-side" "yes" "$host_victim"

got_other_ls=$(_run_pilot_sandboxed /bin/sh -c "[ -e '$OTHER_WORKTREE' ] && echo visible || echo hidden" 2>/dev/null)
assert_eq "another worktree is not visible by path" "hidden" "$got_other_ls"

# The PARENT repository's own checkout is a different case, and the distinction
# is load-bearing. Mounting paths under its .git necessarily materialises its
# directory as a skeleton, so its PATH exists inside the sandbox. What must not
# exist is its content — AC2 is about file access, not about path visibility,
# and a probe that confused the two would report a leak that is not one (or,
# worse, pass because the path it named never existed).
# The PARENT repository, not this checkout: $REPO_ROOT is itself a linked
# worktree, so its .git is a file and its path is not what gets materialised.
PARENT_GIT_DIR=$(git -C "$WORKTREE_DIR" rev-parse --path-format=absolute --git-common-dir)
PARENT_CHECKOUT=$(dirname "$PARENT_GIT_DIR")
got_parent_content=$(_run_pilot_sandboxed /bin/sh -c "
    leaked=
    for f in Cargo.toml crates README.md Makefile; do
        [ -e '$PARENT_CHECKOUT'/\$f ] && leaked=\"\$leaked \$f\"
    done
    [ -n \"\$leaked\" ] && echo \"leaked:\$leaked\" || echo unreachable" 2>/dev/null)
got_skeleton=$(_run_pilot_sandboxed /bin/sh -c "[ -d '$PARENT_CHECKOUT' ] && echo present || echo absent" 2>/dev/null)
assert_eq "control: the parent checkout path IS materialised (else the next two are vacuous)" "present" "$got_skeleton"
assert_eq "the parent checkout's working tree is unreachable" "unreachable" "$got_parent_content"

# And only this worktree is listed under the parent's worktrees/ — bwrap
# materialises the intermediate directory without revealing its siblings.
got_wt_siblings=$(_run_pilot_sandboxed /bin/sh -c "ls '$PARENT_CHECKOUT'/.git/worktrees 2>/dev/null | wc -l" 2>/dev/null)
assert_eq "only this worktree is listed under the parent's worktrees/" "1" "$got_wt_siblings"

got_other_git=$(_run_pilot_sandboxed /bin/sh -c "git -C '$OTHER_WORKTREE' status -sb >/dev/null 2>&1 && echo readable || echo blocked" 2>/dev/null)
assert_eq "another worktree is not readable through git -C" "blocked" "$got_other_git"

# Widening `refs` to read-write would grant deletion of every head in the
# repository. It is bound read-only with only this branch's directory writable,
# so a delete of an unrelated head must be refused by the filesystem.
# The classification distinguishes "the filesystem refused" from "git is not
# working at all" on purpose. Collapsing them would let a totally broken
# sandbox — the pre-fix state — read as a passing containment check on one run
# and as a spurious DELETED on another; neither is the truth.
got_delete=$(_run_pilot_sandboxed /bin/sh -c '
    out=$(git update-ref -d refs/heads/'"$VICTIM_BRANCH"' 2>&1)
    if echo "$out" | grep -qi "not a git repository"; then
        echo git-broken
    elif echo "$out" | grep -qi "read-only\|cannot lock\|permission denied"; then
        echo refused
    elif git rev-parse --verify --quiet refs/heads/'"$VICTIM_BRANCH"' >/dev/null 2>&1; then
        echo refused
    else
        echo DELETED
    fi' 2>/dev/null)
assert_eq "deleting an unrelated branch head is refused (refs is read-only)" "refused" "$got_delete"

host_victim_after=$(git -C "$REPO_ROOT" rev-parse --verify --quiet "$VICTIM_BRANCH" >/dev/null 2>&1 && echo yes || echo no)
assert_eq "the victim branch still exists host-side after the attempt" "yes" "$host_victim_after"

# AC3 — the new bind must not carry a credential in either direction.
got_gitconfig_home=$(_run_pilot_sandboxed /bin/sh -c '[ -e "$HOME/.gitconfig" ] && echo present || echo missing' 2>/dev/null)
assert_eq "the operator ~/.gitconfig is not visible in the sandbox" "missing" "$got_gitconfig_home"
got_gh_config=$(_run_pilot_sandboxed /bin/sh -c '[ -e "$HOME/.config/gh" ] && echo present || echo missing' 2>/dev/null)
assert_eq "~/.config/gh is not visible in the sandbox" "missing" "$got_gh_config"
got_ssh=$(_run_pilot_sandboxed /bin/sh -c '[ -e "$HOME/.ssh" ] && echo present || echo missing' 2>/dev/null)
assert_eq "~/.ssh is not visible in the sandbox" "missing" "$got_ssh"
got_cfg_writable=$(_run_pilot_sandboxed /bin/sh -c '
    git config --local --add probe.twentyfourtyone 1 >/dev/null 2>&1 && echo writable || echo readonly' 2>/dev/null)
assert_eq "the parent repo config cannot be rewritten from the sandbox" "readonly" "$got_cfg_writable"

echo ""
echo "GUARD — a branch with no slash is refused, not silently widened"
echo "--------------------------------------------------------------"
# dirname("refs/heads/main") is "refs/heads": binding that read-write would hand
# the sandbox every branch head in the repository. The dispatcher must abandon.
_PILOT_GITDIR_BIND_ARGS=()
_PILOT_GITDIR_BIND_ABORT=""
saved_head=$(cat "$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/HEAD")
printf 'ref: refs/heads/noslashbranch\n' > "$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/HEAD"
_pilot_gitdir_bind_args "$WORKTREE_DIR"; guard_rc=$?
printf '%s\n' "$saved_head" > "$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/HEAD"
assert_eq "a slashless branch makes the helper fail closed" "1" "$guard_rc"
assert_contains "the refusal names the branch and the reason" "noslashbranch" "$_PILOT_GITDIR_BIND_ABORT"
assert_eq "no bind args are emitted on refusal" "0" "${#_PILOT_GITDIR_BIND_ARGS[@]}"

echo ""
echo "ADVERSARIAL — attacker-writable inputs cannot widen the bind set"
echo "---------------------------------------------------------------"
# This helper runs HOST-side and decides what gets mounted rw into the NEXT
# sandbox, but every input it reads lives inside the PREVIOUS sandbox's
# writable surface: `.git` is in $WORKTREE_DIR, and worktrees/<name>/HEAD is
# bound rw by the helper itself. Worktrees persist between dispatches, so a
# pilot that tampered with either would be choosing the next dispatch's mounts.
# Each probe restores what it changed.
WT_GITDIR=$(git -C "$WORKTREE_DIR" rev-parse --path-format=absolute --git-dir 2>/dev/null)
SAVED_HEAD=$(cat "$WT_GITDIR/HEAD")
SAVED_DOTGIT=$(cat "$WORKTREE_DIR/.git")

probe_refuses() {
    local label="$1"
    _PILOT_GITDIR_BIND_ARGS=()
    _PILOT_GITDIR_BIND_ABORT=""
    _pilot_gitdir_bind_args "$WORKTREE_DIR"; local rc=$?
    if [ "$rc" -ne 0 ] && [ -n "$_PILOT_GITDIR_BIND_ABORT" ] && [ "${#_PILOT_GITDIR_BIND_ARGS[@]}" -eq 0 ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    rc=$rc abort='$_PILOT_GITDIR_BIND_ABORT' args=${#_PILOT_GITDIR_BIND_ARGS[@]}"
        local i
        for ((i = 0; i < ${#_PILOT_GITDIR_BIND_ARGS[@]}; i++)); do
            echo "      arg[$i]=${_PILOT_GITDIR_BIND_ARGS[$i]}"
        done
    fi
}

printf 'ref: refs/heads/../../../../../../tmp\n' > "$WT_GITDIR/HEAD"
probe_refuses "a path-traversal in HEAD is refused, not turned into a rw mount"
printf '%s\n' "$SAVED_HEAD" > "$WT_GITDIR/HEAD"

printf 'ref: refs/tags/v1\n' > "$WT_GITDIR/HEAD"
probe_refuses "a HEAD outside refs/heads/ is refused"
printf '%s\n' "$SAVED_HEAD" > "$WT_GITDIR/HEAD"

printf 'gitdir: /tmp\n' > "$WORKTREE_DIR/.git"
probe_refuses "a .git repointed outside the parent's worktrees/ is refused"
printf '%s\n' "$SAVED_DOTGIT" > "$WORKTREE_DIR/.git"

# And the un-tampered worktree still resolves — otherwise the four refusals
# above would be indistinguishable from a helper that refuses everything.
_PILOT_GITDIR_BIND_ARGS=()
_PILOT_GITDIR_BIND_ABORT=""
_pilot_gitdir_bind_args "$WORKTREE_DIR"; restore_rc=$?
assert_eq "control: the restored worktree still resolves" "0" "$restore_rc"
nonzero=0; [ "${#_PILOT_GITDIR_BIND_ARGS[@]}" -gt 0 ] && nonzero=1
assert_eq "control: it still emits bind args" "1" "$nonzero"

echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="
[ "$FAIL" -eq 0 ] || exit 1
exit 0
