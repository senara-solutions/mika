#!/bin/sh
# Shared `cwd` composability guard for long-running skill handlers (mika#2536).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE DEFECT THIS CLOSES
#
# `build-mika` crashed FOUR times on the QA of PR #2530 — every attempt, same
# `tasks.result`:
#
#     HANDLER CRASH (exit code 1). Script failed before building result.
#
# The crash was located to the line: `cd "$CWD"`, the ONLY literal `exit 1`
# between the trap install and the first `RESULT` assignment. `cd` writes its
# diagnostic to a stderr the long-running path discards, so the one surface an
# operator reads carried a sentence about an exit code and nothing about the
# path. Four identical crashes produced four identical, uninformative results.
#
# The guard's whole job is that an uncomposable `cwd` is REFUSED BY NAME, in the
# `RESULT` the callback delivers, instead of dying under `cd`.
#
# ─────────────────────────────────────────────────────────────────────────────
# IT VALIDATES AND NAMES; IT DOES NOT CANONICALIZE
#
# Each caller keeps its own `pwd -P` and composes its own `RESULT`. That is not
# tidiness: `deploy-mika` additionally enforces an allowed-prefix perimeter on
# the canonicalized path, and folding canonicalization in here would make one
# function answer two questions — composability and authority — whose failure
# directions differ. This guard adds to that perimeter; it never replaces it.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE ORDER OF THE FOUR TESTS IS A DELIVERABLE, NOT COSMETICS
#
# A `cwd` holding the literal text `$MIKA_PLATFORM_DIR/.claude/worktrees/…` — the
# founding suspicion, and the shape a prompt that prescribes an undeveloped
# variable produces — is simultaneously:
#
#   * carrying an unexpanded variable,
#   * not an absolute path (it starts with `$`),
#   * non-existent.
#
# All three statements are TRUE. Only the first is USEFUL: the other two send an
# operator to create a directory or to fix a caller's path, when the repair is in
# a prompt. So `unexpanded_variable` is tested FIRST, and the remaining three run
# from most structural to most specific.
#
# ─────────────────────────────────────────────────────────────────────────────
# POSIX sh ONLY
#
# Both callers are `#!/bin/sh`. No `[[ ]]`, no `=~`, no `local`, no arrays. The
# four tests are two `case` patterns and two `test` operators, which is the whole
# reason the ordering above is expressible without a regex engine.
#
# This file contains function definitions and constants ONLY — no top-level
# imperative code — so sourcing it is side-effect free. Its callers source it
# AFTER installing their EXIT trap, so a failure to source still delivers a
# callback.

# ── The refusal vocabulary is a WIRE FORMAT.
#
# These four tokens land in `tasks.result`, which is where an operator reads
# them (`mika tasks get <id>`), and a `GROUP BY`-style count over them is the
# only instrument this work ships. Two spellings of one motif would split a
# population without saying so, so the list has ONE definition site and
# `scripts/test-cwd-guard.sh` pins its four values.
CWD_GUARD_REFUSAL_REASONS="unexpanded_variable not_absolute does_not_exist not_a_directory"

# Ticket that owns this guard. Carried in every refusal so the `RESULT` names
# where its reasoning lives.
CWD_GUARD_TICKET="mika#2536"

# The durable surface. The caller routes it into `RESULT` and exits non-zero;
# the EXIT trap then delivers it as the callback body.
CWD_REFUSAL=""

# Compose a named refusal.
#
#   Usage: cwd_guard_refuse <reason> <cwd> <remedy>
#
# Always returns 1 (the shape of its house sibling `pr_push_guard_refuse`), so
# every call site inside `validate_cwd` consumes that status with `|| return 1`.
#
# **What that `||` does, and — measured — what it does NOT do.** It makes this
# function's internal control flow independent of the CALLER's errexit state:
# without it, a statement written after a refusal inside `validate_cwd` would be
# unreachable when errexit is live and reachable when it is suspended, which is
# a difference no test would show until somebody added that statement.
#
# It does **not** make a bare call site safe, and claiming otherwise would be a
# comment asserting a protection the code does not provide. `validate_cwd`
# signals by exit status, so `validate_cwd "$CWD"` written as a bare statement
# under `set -e` aborts the shell either way — one step later, same effect. That
# abort would kill the handler before it could put `CWD_REFUSAL` into `RESULT`,
# and the EXIT trap would compose the generic "HANDLER CRASH" this guard exists
# to remove. **So the protection is at the CALL SITE**: both handlers invoke
# `if ! validate_cwd …`, where POSIX suspends errexit for the whole call. A third
# call site must use `if !`, `||` or `&&` — never a bare statement.
#
# Also echoes to stderr — captured by the executor only on a non-zero exit,
# which is exactly this path.
cwd_guard_refuse() {
    CWD_REFUSAL="REFUSED (cwd-guard, ${CWD_GUARD_TICKET}) — $1: cwd=$2. $3"
    echo "$CWD_REFUSAL" >&2
    return 1
}

# Validate that a `cwd` can be composed into a working directory.
#
#   Usage: validate_cwd <path>
#     → 0, and `CWD_REFUSAL` empty, when the path is composable
#     → 1, and `CWD_REFUSAL` naming one of CWD_GUARD_REFUSAL_REASONS, otherwise
#
# The caller is expected to have applied its own default already, so an empty
# argument cannot arrive from either production caller. It is nevertheless
# refused as `not_absolute` rather than silently accepted — `cd ""` is a no-op
# that would leave the handler building in the skill directory.
validate_cwd() {
    CWD_REFUSAL=""

    # 1. An unexpanded variable. FIRST, for the reason in the header: this
    #    candidate also fails tests 2 and 3, and only this name points at the
    #    prompt that produced it.
    case "$1" in
        *'$'*)
            cwd_guard_refuse "unexpanded_variable" "$1" \
                "The caller passed a path still carrying a shell variable. Nothing in \
a long-running handler's environment expands it: the child env is rebuilt from a \
positive allowlist that refuses every MIKA_* name. Fix the caller so it passes a \
literal path, or omit cwd and let the handler use its own default." || return 1
            ;;
    esac

    # 2. Not absolute. Distinguishes a relative path from an absent one, and
    #    absorbs the empty-argument case with the least misleading name.
    case "$1" in
        /*) ;;
        *)
            cwd_guard_refuse "not_absolute" "$1" \
                "A handler runs with its own working directory (the skill dir), so a \
relative cwd resolves against something the caller did not intend. Pass an \
absolute path." || return 1
            ;;
    esac

    # 3. Does not exist. The case `deploy-mika` already covered on its own; the
    #    name is now shared so both handlers report it identically.
    if [ ! -e "$1" ]; then
        cwd_guard_refuse "does_not_exist" "$1" \
            "The path is absolute and fully expanded but nothing is there. If it names \
a dispatch worktree, that worktree has been reaped (mika#2420) or never existed on \
this host." || return 1
    fi

    # 4. Not a directory. `cd` on a regular file fails with a message no
    #    surface relays, which is the class this guard exists to remove.
    if [ ! -d "$1" ]; then
        cwd_guard_refuse "not_a_directory" "$1" \
            "The path exists but is not a directory, so it cannot be a working \
directory. Pass the directory that contains it." || return 1
    fi

    return 0
}
