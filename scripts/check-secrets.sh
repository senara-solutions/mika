#!/usr/bin/env bash
# Shared secret + large-file scanner — single source of truth for the
# pre-commit hook (lefthook.yml) and the CI `secret-scan` job (.github/workflows/ci.yml).
# See: https://github.com/senara-solutions/mika/issues/1689
#
# The `--no-verify` rescue path in dispatch-lib.sh (mika#1685) bypasses the entire
# lefthook pre-commit block, dropping the `no-secrets` / `no-large-files` gates.
# CI cannot be bypassed, so it must carry the same net. To keep the two nets from
# drifting, the detection regex and the 1 MB threshold live here, in exactly one place.
#
# Modes:
#   check-secrets.sh <file>...            explicit file list (lefthook {staged_files})
#   check-secrets.sh --changed <base-ref> files added/copied/modified vs merge-base(base, HEAD)
#   check-secrets.sh                       no files → exit 0 (nothing to scan)
#
# Exit 0 if clean, exit 1 with an actionable message if a secret or oversized file is found.
# Oversized files may be exempted by exact path via LARGE_FILE_ALLOWLIST below; secrets never are.

set -euo pipefail

# --- Single source of truth (mirrored by nothing; consumed everywhere) ---
SECRET_REGEX='(sk-ant-api[0-9]+-[A-Za-z0-9_-]{20,}|sk-ant-oat[0-9]+-[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|-----BEGIN (RSA |EC )?PRIVATE KEY)'
LARGE_FILE_LIMIT=1048576  # 1 MB, in bytes

# --- Large-file named-exception allowlist (mika#1689 Fire-Disposition (a)) ---
#
# The secret detector has NO content allowlist — a live credential in a changed
# file is halt-and-surface, always. The large-file detector is different: a
# legitimately-oversized file (a golden fixture, a test binary) is an occasionally
# valid need, so its disposition is an *explicit, named* exception rather than a
# blanket bypass.
#
# Contract for adding an entry:
#   - one exact repo-relative path per line (no globs, no directories),
#   - an inline comment stating WHY it is exempt and referencing a tracker issue,
#   - the PR that adds the first entry files that one-line follow-up issue — an
#     exception is auditable here, never silent.
# A path not listed still halts with exit 1.
#
# Empty at rollout: both detectors are diff-scoped (only files the PR itself
# adds/modifies are scanned), so no pre-existing fixture can force an entry.
LARGE_FILE_ALLOWLIST=(
    # "path/to/fixture.bin"  # why it is exempt + mika#<issue>
)

# Exact-path membership test against LARGE_FILE_ALLOWLIST.
# Consulted identically in explicit-file mode and --changed mode, so lefthook and
# CI honor the same named exceptions (single source of truth, R3/D1).
is_large_file_allowlisted() {
    local candidate="$1" entry
    # `${arr[@]+...}` guard: an empty array is an unbound expansion under `set -u`
    # on bash < 4.4 (macOS ships 3.2, and lefthook runs this locally).
    for entry in ${LARGE_FILE_ALLOWLIST[@]+"${LARGE_FILE_ALLOWLIST[@]}"}; do
        [[ "$candidate" == "$entry" ]] && return 0
    done
    return 1
}

# --- Collect the file set ---
FILES=()
if [[ "${1:-}" == "--changed" ]]; then
    base_ref="${2:-}"
    if [[ -z "$base_ref" ]]; then
        echo "ERROR: --changed requires a base ref (e.g. --changed origin/main)" >&2
        exit 2
    fi
    merge_base="$(git merge-base "$base_ref" HEAD)"
    # `-z` (NUL-delimited) is load-bearing, not style: without it git renders a
    # non-ASCII path as a C-quoted string (`"s\303\251cret.rs"`), which no longer
    # names a file on disk. The scanner would then skip it as "not present" and
    # report the change set clean — a silent miss in a security net (mika#1689).
    while IFS= read -r -d '' f; do
        [[ -n "$f" ]] && FILES+=("$f")
    done < <(git diff -z --name-only --diff-filter=ACM "$merge_base"..HEAD)
else
    FILES=("$@")
fi

# Empty set (no args, or a diff with only deletions) → nothing to scan.
if [[ ${#FILES[@]} -eq 0 ]]; then
    echo "check-secrets: no files to scan — clean."
    exit 0
fi

VIOLATIONS=0
SCANNED=0

# Secret glob scope — parity with lefthook's no-secrets glob (excludes lefthook*.yml).
is_secret_scannable() {
    local f="$1"
    case "$(basename "$f")" in
        lefthook*.yml) return 1 ;;
    esac
    case "$f" in
        *.rs|*.ts|*.tsx|*.js|*.jsx|*.toml|*.json|*.sh|*.env) return 0 ;;
        *) return 1 ;;
    esac
}

for f in "${FILES[@]}"; do
    # Deleted files can surface in a raw diff even with --diff-filter=ACM guarding
    # (belt & suspenders). Skip anything not present on disk, but say so — a
    # silent skip in a security net reads as "scanned and clean".
    if [[ ! -f "$f" ]]; then
        echo "check-secrets: skipping $f — not a regular file on disk"
        continue
    fi
    SCANNED=$((SCANNED + 1))

    # --- Secret check (glob-scoped, allowlist-filtered) ---
    if is_secret_scannable "$f"; then
        # The `grep -v` excludes run against `<path>:<line>:<content>` — the exact
        # shape lefthook's `grep -rn ... {staged_files}` produced. That matters:
        # the excludes were path-scoped as well as content-scoped, so
        # `crates/mika-agent/src/secret_scrubber.rs` (whose test fixtures are
        # deliberately secret-shaped) passed as a whole. Scanning file-by-file
        # drops the path from grep's output, so it is re-prefixed here; without
        # that, every PR touching the scrubber's own tests fails the gate.
        match="$(grep -nE "$SECRET_REGEX" "$f" 2>/dev/null \
            | sed "s|^|$f:|" \
            | grep -v 'TEST_RSA_PEM' \
            | grep -v 'secret_scrubber' || true)"
        if [[ -n "$match" ]]; then
            while IFS= read -r hit; do
                echo "ERROR: potential secret in $hit"
            done <<< "$match"
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
    fi

    # --- Large-file check (all changed files, no extension filter) ---
    if ! is_large_file_allowlisted "$f"; then
        size="$(wc -c < "$f" 2>/dev/null || echo 0)"
        if [[ "$size" -gt "$LARGE_FILE_LIMIT" ]]; then
            echo "ERROR: $f is $(( size / 1024 ))KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST with a named reason if legitimate)"
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
    fi
done

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "check-secrets: $VIOLATIONS of $SCANNED scanned file(s) carry violations — see errors above."
    echo "Secret patterns and the 1MB cap are defined in scripts/check-secrets.sh (single source of truth)."
    exit 1
fi

echo "check-secrets: scanned $SCANNED of ${#FILES[@]} file(s) in the set — clean."
exit 0
