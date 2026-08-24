#!/usr/bin/env bash
# CI lint (mika#1807 AC4 + mika#1969 — build-time invariant, Q2 point 3):
# each controlled-egress substrate at `crates/mika-gateway/src/egress_*`
# is the ONLY place in the platform allowed to reference the upstream
# identifier tokens for that class. Any hit outside the authorized
# path fails the build.
#
# Currently guards two egress classes:
#   - egress_search (mika#1807 E1) — Brave Search API
#   - egress_fetch  (mika#1969)    — gouv.fr GET-only allowlist
#
# The mirror-substrate-module pattern for adding a third class is
# documented in:
#   docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md
#
# Discipline analog: `scripts/check-byte-slices.sh` (#764) — construct
# the incapacity, don't promise the restraint. Same shape as
# `scripts/check-loop-select.sh` (#848).
#
# What we grep for: well-known upstream domain + path identifiers. If a
# future ticket adds another upstream, extend the PATTERNS array below
# AND add the module arm (or a sibling substrate module per the
# mirror-module pattern).
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found
#
# Legacy allowlist:
#   `crates/mika-agent/src/skills/builtin_handlers.rs` currently owns the
#   pre-E1 `web_search` builtin that talks to Brave directly. E2 (#1808)
#   migrates it to `POST /internal/search` on the gateway. Until then
#   this file is explicitly allowlisted for the Brave identifier below.
#
#   The `fetch_url` builtin added in mika#1969 does NOT name the gouv.fr
#   hosts — it delegates to `POST /internal/fetch` on the gateway. It
#   therefore does NOT get a LEGACY_ALLOWLIST entry: absence of that
#   entry is load-bearing, since a future reviewer might add one
#   defensively. If you find yourself adding an allowlist for the fetch
#   builtin, the delegation shape has regressed — fix that instead.

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
    # egress_fetch (mika#1969) — gouv.fr allowlist. Each substring must
    # match ALLOWED_HOSTS in `crates/mika-gateway/src/egress_fetch/mod.rs`.
    # Extension is a code change + deploy per KTD2 — do not turn into
    # an env var.
    "service-public.fr"
    "ants.gouv.fr"
    "impots.gouv.fr"
    "data.gouv.fr"
    # Future upstreams — extend as new egress classes are added.
)

# Files/dirs allowed to contain these identifiers. Substring match.
AUTHORIZED_PATHS=(
    "crates/mika-gateway/src/egress_search.rs"
    "crates/mika-gateway/src/egress_search/"
    "crates/mika-gateway/docs/egress-search.md"
    "crates/mika-gateway/docs/egress-search-threat-model.md"
    "crates/mika-gateway/docs/egress-search-no-log-audit.md"
    "crates/mika-gateway/tests/egress_search"
    "docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md"
    # egress_fetch (mika#1969)
    "crates/mika-gateway/src/egress_fetch/"
    "crates/mika-gateway/src/egress_fetch.rs"
    "crates/mika-gateway/tests/egress_fetch"
    "docs/plans/1969-egress-fetch-fetch-url-builtin.md"
    "docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md"
    "scripts/verify-egress-uniqueness.sh"
    "scripts/verify-egress-request-shape.sh"
    "scripts/verify-egress-no-log.sh"
    "scripts/audit-egress-no-log.sh"
    # Test fixtures (mika#1970) — grounding_regressions eval scenarios use
    # "service-public.fr" as prose payload text in a mock LLM response; the
    # tests never egress. Post-#1978 merge these files landed on main; the
    # lint substring-match flags them until authorized.
    "crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_qualification.rs"
    "crates/mika-agent/tests/eval/grounding_assertions/mod.rs"
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
