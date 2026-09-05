#!/usr/bin/env bash
# CI lint (mika#2108): the consignes a pilot receives WITHOUT ASKING must not
# cite an absolute path under the host workspace root.
#
# THE PROPERTY, not a list of files.
#   A dispatch pilot runs under bwrap with `--chdir "$WORKTREE_DIR"` and a bind
#   set that mounts the worktree and the strict minimum of its gitdir
#   (dispatch-lib.sh `_pilot_gitdir_bind_args`, mika#2141/#2146). The host
#   workspace root — `/data/workspace/…` — is NOT mounted. What is visible
#   there is a skeleton of mount points created as a side effect of the gitdir
#   binds. A path under it therefore resolves to nothing useful, or worse to a
#   directory that answers plausibly and falsely.
#
#   The surface this guard defends is the one Claude Code loads BY ITSELF:
#   every `CLAUDE.md` from the cwd upward, and every `.claude/commands/*.md`.
#   The pilot does not choose to read those — it receives them. A host path in
#   that surface is a navigation instruction that cannot be followed, and the
#   founding incident (mika#2108) is two sessions that burned their whole turn
#   budget looking for a checkout that was never mounted, then died reaching for
#   a wider command shape (cpp#128).
#
# WHY `git ls-files` AND NOT A FILESYSTEM GLOB.
#   A dispatch worktree's `.claude/commands/` is not this repository's surface
#   alone: `_seed_worktree_slash_commands` copies the meta-repo's orchestration
#   commands in at setup time (mika#1173/#1415) and shields them from
#   `git status` via the common-dir `info/exclude`. Globbing the directory
#   would make this guard RED inside every dispatch worktree, on files owned by
#   another repository that this one cannot fix — and a guard that is red where
#   it cannot be obeyed gets disabled rather than obeyed. So the guard checks
#   what this repository TRACKS, which is exactly what it has the authority to
#   keep true.
#
#   That boundary is real and it leaves a real gap; it is measured and named in
#   docs/solutions/best-practices/2108-la-surface-de-consignes-chargee-automatiquement.md
#   rather than left to be rediscovered. Do not close it by widening the glob.
#
# WHEN YOU EXTEND THIS SCRIPT, EXTEND IT BY PROPERTY.
#   The rule is "no absolute path under the host workspace root", not "no
#   occurrence of the string /data/workspace/mika-platform". A sibling checkout
#   at /data/workspace/anything-else is the same defect wearing a different
#   name, and it is caught here because the anchor is the root. This is the
#   mika#2103 lesson, learned the expensive way by check-byte-slices.sh: a guard
#   that knows one spelling of a defect protects nothing else.
#
# THE EXEMPTION IS THE ATTACK SURFACE, so it is narrow, named, and must stay
# ALIVE. Exactly one exists today: the `[kg].docs_roots` recipe in
# crates/mika-agent/CLAUDE.md, whose six entries are configuration values
# describing an agent that runs HOST-side — not navigation addressed to a
# pilot. It is anchored on the exact shape of a TOML array entry, so any other
# host path added to that same file still fires. And if it ever stops matching
# anything, the guard exits 4: a stale exemption is a silent widening, and a
# dead one must be deleted rather than left standing.
#
# Usage: check-pilot-instruction-paths.sh [file...]
#   No arguments: the tracked auto-loaded surface of this repository.
#   Explicit files: fixture mode, used by the anti-vacuity harness
#   (scripts/test-check-pilot-instruction-paths.sh).
#
# Exit codes:
#   0 — clean
#   1 — a host path was found in the auto-loaded surface
#   2 — a file or the repository could not be read
#   3 — the surface was empty (nothing to check is not the same as clean)
#   4 — a declared exemption matched nothing and is now dead

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# The host workspace root. Anchored on the root, not on one repository under
# it — see "extend by property" above.
HOST_WORKSPACE_ROOT='/data/workspace/'

# Declared exemptions: "<repo-relative path>|<ERE the whole line must match>|<reason>".
# Both halves are load-bearing. The path keeps the exemption from leaking to
# another file; the line pattern keeps it from leaking to another line of the
# same file.
EXEMPTIONS=(
  'crates/mika-agent/CLAUDE.md|^[[:space:]]*"/data/workspace/mika-platform/[A-Za-z0-9._-]+/docs/solutions",$|[kg].docs_roots recipe — host-side agent configuration, not pilot navigation (mika#2108 AC7)'
)

# --- the surface -------------------------------------------------------------

declare -a SURFACE=()

if [[ $# -gt 0 ]]; then
    for f in "$@"; do
        if [[ ! -f "$f" ]]; then
            echo "ERROR: file not readable: $f" >&2
            exit 2
        fi
        SURFACE+=("$f")
    done
else
    if ! git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
        echo "ERROR: $REPO_ROOT is not a git repository — the tracked surface cannot be enumerated." >&2
        exit 2
    fi
    while IFS= read -r rel; do
        [[ -n "$rel" ]] || continue
        [[ -f "$REPO_ROOT/$rel" ]] || continue
        SURFACE+=("$REPO_ROOT/$rel")
    done < <(git -C "$REPO_ROOT" ls-files -- 'CLAUDE.md' '*/CLAUDE.md' '.claude/commands/*.md' '*/.claude/commands/*.md')
fi

if [[ ${#SURFACE[@]} -eq 0 ]]; then
    echo "ERROR: the auto-loaded instruction surface is empty." >&2
    echo "This guard has nothing to check, which is not the same as a clean tree." >&2
    exit 3
fi

# --- the scan ----------------------------------------------------------------

VIOLATIONS=0
declare -a EXEMPTION_HITS=()
for ((i = 0; i < ${#EXEMPTIONS[@]}; i++)); do
    EXEMPTION_HITS+=(0)
done

for file in "${SURFACE[@]}"; do
    rel="${file#"$REPO_ROOT"/}"

    # `grep || true`: no match is the nominal case, not an error, and `set -e`
    # would abort the sweep on the first clean file.
    hits=$(grep -nF -- "$HOST_WORKSPACE_ROOT" "$file" 2>/dev/null || true)
    [[ -n "$hits" ]] || continue

    while IFS= read -r hit; do
        [[ -n "$hit" ]] || continue
        lineno="${hit%%:*}"
        content="${hit#*:}"

        exempted=0
        for ((i = 0; i < ${#EXEMPTIONS[@]}; i++)); do
            IFS='|' read -r ex_path ex_re _ex_reason <<<"${EXEMPTIONS[$i]}"
            [[ "$rel" == "$ex_path" ]] || continue
            # Here-string, never `printf | grep -q`: the pipeline form takes
            # SIGPIPE under pipefail and is rejected by
            # scripts/verify-no-sigpipe-grep.sh (mika#2055).
            if grep -qE -- "$ex_re" <<<"$content"; then
                exempted=1
                EXEMPTION_HITS[$i]=$(( ${EXEMPTION_HITS[$i]} + 1 ))
                break
            fi
        done
        [[ $exempted -eq 1 ]] && continue

        echo "ERROR: host path in the pilot's auto-loaded surface: $rel:$lineno"
        echo "       $content"
        VIOLATIONS=$((VIOLATIONS + 1))
    done <<<"$hits"
done

# --- dead-exemption check ----------------------------------------------------
#
# Only meaningful over the repository's own surface. In fixture mode the caller
# chose the files, and an exemption that does not apply to them is not stale.
if [[ $# -eq 0 ]]; then
    DEAD=0
    for ((i = 0; i < ${#EXEMPTIONS[@]}; i++)); do
        if [[ ${EXEMPTION_HITS[$i]} -eq 0 ]]; then
            IFS='|' read -r ex_path ex_re ex_reason <<<"${EXEMPTIONS[$i]}"
            echo "ERROR: dead exemption — nothing in $ex_path matches /$ex_re/" >&2
            echo "       declared reason: $ex_reason" >&2
            DEAD=$((DEAD + 1))
        fi
    done
    if [[ $DEAD -gt 0 ]]; then
        echo "" >&2
        echo "An exemption that protects nothing still widens this guard for every" >&2
        echo "future line that grows into its shape. Delete it from EXEMPTIONS." >&2
        exit 4
    fi
fi

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS host path(s) in the surface a pilot loads without asking."
    echo ""
    echo "A dispatch pilot runs in a bwrap namespace where ${HOST_WORKSPACE_ROOT} is NOT"
    echo "mounted. Since mika#2146 the gitdir binds materialise its intermediate"
    echo "directories, so such a path no longer fails loudly — it resolves to a"
    echo "skeleton that answers plausibly and falsely. That is mika#2108."
    echo ""
    echo "Fix: write the instruction relative to the working root, or name the"
    echo "value as host-side configuration rather than as somewhere to go. If it"
    echo "genuinely is host-side configuration, add a NARROW exemption to"
    echo "EXEMPTIONS in this script — anchored on the file AND the line shape —"
    echo "and annotate it where it lives."
    exit 1
fi

total_exempt=0
for ((i = 0; i < ${#EXEMPTIONS[@]}; i++)); do
    total_exempt=$(( total_exempt + ${EXEMPTION_HITS[$i]} ))
done
echo "Pilot instruction surface clean: ${#SURFACE[@]} file(s) checked, ${total_exempt} exempt line(s) skipped."
exit 0
