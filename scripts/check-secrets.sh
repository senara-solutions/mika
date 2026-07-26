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

set -euo pipefail

# --- Single source of truth (mirrored by nothing; consumed everywhere) ---
SECRET_REGEX='(sk-ant-api[0-9]+-[A-Za-z0-9_-]{20,}|sk-ant-oat[0-9]+-[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|-----BEGIN (RSA |EC )?PRIVATE KEY)'
LARGE_FILE_LIMIT=1048576  # 1 MB, in bytes

# --- Collect the file set ---
FILES=()
if [[ "${1:-}" == "--changed" ]]; then
    base_ref="${2:-}"
    if [[ -z "$base_ref" ]]; then
        echo "ERROR: --changed requires a base ref (e.g. --changed origin/main)" >&2
        exit 2
    fi
    merge_base="$(git merge-base "$base_ref" HEAD)"
    while IFS= read -r f; do
        [[ -n "$f" ]] && FILES+=("$f")
    done < <(git diff --name-only --diff-filter=ACM "$merge_base"..HEAD)
else
    FILES=("$@")
fi

# Empty set (no args, or a diff with only deletions) → nothing to scan.
if [[ ${#FILES[@]} -eq 0 ]]; then
    echo "check-secrets: no files to scan — clean."
    exit 0
fi

VIOLATIONS=0

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
    # (belt & suspenders). Skip anything not present on disk.
    [[ -f "$f" ]] || continue

    # --- Secret check (glob-scoped, allowlist-filtered) ---
    if is_secret_scannable "$f"; then
        match="$(grep -nE "$SECRET_REGEX" "$f" 2>/dev/null | grep -v 'TEST_RSA_PEM' | grep -v 'secret_scrubber' || true)"
        if [[ -n "$match" ]]; then
            while IFS= read -r hit; do
                echo "ERROR: potential secret in $f:$hit"
            done <<< "$match"
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
    fi

    # --- Large-file check (all changed files, no extension filter) ---
    size="$(wc -c < "$f" 2>/dev/null || echo 0)"
    if [[ "$size" -gt "$LARGE_FILE_LIMIT" ]]; then
        echo "ERROR: $f is $(( size / 1024 ))KB — exceeds 1MB limit"
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "check-secrets: found $VIOLATIONS violation(s) — see errors above."
    echo "Secret patterns and the 1MB cap are defined in scripts/check-secrets.sh (single source of truth)."
    exit 1
fi

echo "check-secrets: scanned ${#FILES[@]} file(s) — clean."
exit 0
