#!/usr/bin/env bash
# CI lint (mika#1807 AC4 — build-time invariant, Q2 point 3): the
# egress-search substrate at `crates/mika-gateway/src/egress_search*` is
# the ONLY place in the platform allowed to reference a search-upstream
# identifier. Any hit outside the authorized path fails the build.
#
# Discipline analog: `scripts/check-byte-slices.sh` (#764) — construct
# the incapacity, don't promise the restraint. Same shape as
# `scripts/check-loop-select.sh` (#848).
#
# What we grep for: well-known search-upstream domain + path identifiers.
# If a future E2 / E6 ticket adds another upstream (SearXNG, etc.), extend
# the PATTERNS array below AND add the arm to `SearchUpstream`.
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found
#
# Legacy allowlist:
#   `crates/mika-agent/src/skills/builtin_handlers.rs` currently owns the
#   pre-E1 `web_search` builtin that talks to Brave directly. E2 (#1808)
#   migrates it to `POST /internal/search` on the gateway. Until then
#   this file is explicitly allowlisted below. Any OTHER hit is a hard
#   failure.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Search-upstream identifier substrings the substrate is authoritative for.
# Anything in this list appearing in a source file outside the authorized
# path is a discipline violation.
#
# Scope discipline: this list catches real API endpoint hosts / paths — the
# strings that appear ONLY in code performing an actual network call. Marketing
# URLs (like `https://brave.com/search/api/` — the free-key sign-up landing
# page) are intentionally out of scope; they cannot reach the upstream and
# their presence in docs is legitimate.
PATTERNS=(
    "api.search.brave.com"
    # Future upstreams — extend as `SearchUpstream` variants are added:
    #   "searx"
    #   "duckduckgo.com/api"
    #   "google.com/customsearch"
)

# Files/dirs allowed to contain these identifiers. Substring match.
AUTHORIZED_PATHS=(
    "crates/mika-gateway/src/egress_search.rs"
    "crates/mika-gateway/src/egress_search/"
    "crates/mika-gateway/docs/egress-search.md"
    "crates/mika-gateway/docs/egress-search-threat-model.md"
    "crates/mika-gateway/tests/egress_search"
    "docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md"
    "scripts/verify-egress-uniqueness.sh"
    "scripts/verify-egress-request-shape.sh"
)

# Legacy allowlist — code paths that ship this identifier pre-E1 and are
# scheduled for migration in a specific sibling ticket. Each entry MUST
# name the ticket that removes it. If the ticket lands, remove the entry.
LEGACY_ALLOWLIST=(
    # E2 (#1808) migrates the `web_search` builtin to `/internal/search`.
    "crates/mika-agent/src/skills/builtin_handlers.rs"
)

# Path matcher — returns 0 (allowed) if $1 contains any entry in
# AUTHORIZED_PATHS or LEGACY_ALLOWLIST.
is_allowed() {
    local path="$1"
    local entry
    for entry in "${AUTHORIZED_PATHS[@]}" "${LEGACY_ALLOWLIST[@]}"; do
        if [[ "$path" == *"$entry"* ]]; then
            return 0
        fi
    done
    return 1
}

violations=0

for pat in "${PATTERNS[@]}"; do
    # Grep the whole crates/ tree (avoid target/, node_modules, etc.).
    # -r recursive, -n line numbers, -F literal string (no regex surprises).
    while IFS= read -r hit; do
        # `hit` shape: `<relpath>:<line>:<content>`
        file="${hit%%:*}"
        if is_allowed "$file"; then
            continue
        fi
        echo "ERROR (egress-uniqueness): search-upstream identifier '$pat' at $hit"
        violations=$((violations + 1))
    done < <(grep -rnF "$pat" "$REPO_ROOT/crates/" 2>/dev/null || true)

    # Also grep the top-level docs/ tree (plan docs / ADRs may legitimately
    # cite the upstream identifier — allowlist covers the E1 plan doc).
    while IFS= read -r hit; do
        file="${hit%%:*}"
        if is_allowed "$file"; then
            continue
        fi
        echo "ERROR (egress-uniqueness): search-upstream identifier '$pat' at $hit"
        violations=$((violations + 1))
    done < <(grep -rnF "$pat" "$REPO_ROOT/docs/" 2>/dev/null || true)
done

if [[ $violations -gt 0 ]]; then
    echo ""
    echo "Found $violations egress-uniqueness violation(s)."
    echo ""
    echo "The E1 egress-search substrate (mika#1807) is the sole controlled"
    echo "reachability path to search upstreams. To route via the substrate:"
    echo "  1) POST /internal/search on the gateway with a bearer token."
    echo "  2) Do NOT call Brave / SearXNG / etc. from any other code path."
    echo ""
    echo "If you are extending the substrate itself, add your file to"
    echo "AUTHORIZED_PATHS in $0."
    exit 1
fi

echo "No egress-uniqueness violations found."
exit 0
