#!/usr/bin/env bash
# CI lint (mika#1809 — E3 dé-liage identité↔requête, build-time invariant):
# any file OUTSIDE the authorized egress-search module tree that either
#   (a) constructs an HTTP request whose URL targets the Brave upstream, or
#   (b) adds a header / query-param field whose lowercase name contains an
#       identity-shaped substring (tenant, user, agent, customer, session, …)
#       AND sits in the crates/ tree,
# fails the build.
#
# Complementary to `scripts/verify-egress-uniqueness.sh` (mika#1807):
# - egress-uniqueness catches HOST-STRING mentions (`api.search.brave.com`).
# - egress-request-shape catches SHAPE violations — a hypothetical future PR
#   that pipes a Brave-shaped request through a general `reqwest::Client` or
#   adds a per-tenant header to the substrate.
#
# Both scripts together fence the E3 LIAGE invariant at repo level; the
# in-code lint tests (`tests_e3_request_shape::outgoing_headers_are_shared_only`
# and `query_params_carry_no_identifier`) fence it at compile+test time.
#
# Discipline analog: `scripts/verify-egress-uniqueness.sh`, `scripts/check-byte-slices.sh`
# (#764), `scripts/check-loop-select.sh` (#848) — construct the incapacity, don't
# promise the restraint.
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# -- Layer A: Brave-shaped request construction outside the substrate --
#
# The `X-Subscription-Token` header is Brave's auth mechanism. Any file
# constructing a request that names this header is either:
#   (a) inside `egress_search/*` (allowed — the substrate itself), or
#   (b) a violation — someone bypassing the substrate to hit Brave directly.
#
# We grep for the literal header name. Substring match on the file path is the
# allowlist — see AUTHORIZED_PATHS below.
LAYER_A_PATTERNS=(
    "X-Subscription-Token"
)

# -- Layer B: per-tenant / per-user attribution fields on egress requests --
#
# Any file inside crates/ that adds a header or query param whose name
# contains an identity-shaped substring is a violation IF it also references
# the egress path. To keep the grep narrow (we don't want to flag every
# `.header("X-User-Foo", ...)` in unrelated gateway code — some of it is
# legitimate for internal auth headers), we scope Layer B to the egress
# module tree PLUS a repo-wide sweep for the specific pattern
# `.header("X-.*-tenant"|"X-.*-user"|...)` combined with a Brave-shaped
# call site (`config.endpoint` or `api.search.brave.com`).
#
# The narrow strategy: grep the egress module tree for any header/query-param
# name containing a forbidden substring. If a future PR adds a per-tenant
# header inside egress_search/*, that's still an E3 violation and the
# egress_search/* allowlist does NOT exempt it. This is a shape lint on the
# egress module itself, not a location gate.
LAYER_B_FORBIDDEN_HEADER_PATTERNS=(
    # `.header("X-Anything-tenant..."|...)` and `.header("Anything-user..."|...)`
    # Case-insensitive via `-i`. Match against the header-string literal.
    '\.header\s*\(\s*"[^"]*(tenant|user|agent|customer|session|correlation|trace|request[-_]id)[^"]*"'
    # Same for query params passed as tuples: `("tenant_id", ...)`, `("user", ...)`
    '\(\s*"(tenant|user|agent|customer|session|correlation|trace|request[-_]id)[^"]*"\s*,'
)

# -- Allowlist --
#
# Files/dirs where Layer A hits are legitimate. Substring match on the file
# path. NOTE: Layer B does NOT use this allowlist — a forbidden header inside
# the substrate is still a violation (that's the whole point of E3).
AUTHORIZED_PATHS_LAYER_A=(
    "crates/mika-gateway/src/egress_search/"
    "crates/mika-gateway/src/egress_search.rs"  # historical single-file form
    "crates/mika-gateway/docs/egress-search.md"
    "crates/mika-gateway/docs/egress-search-threat-model.md"
    "crates/mika-gateway/tests/egress_search"
    "docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md"
    "scripts/verify-egress-request-shape.sh"
    "scripts/verify-egress-uniqueness.sh"
)

# Legacy allowlist — pre-substrate code paths shipping this identifier and
# scheduled for migration in a specific sibling ticket. Each entry MUST
# name the ticket that removes it. If the ticket lands, remove the entry.
# Kept in sync with LEGACY_ALLOWLIST in scripts/verify-egress-uniqueness.sh.
LEGACY_ALLOWLIST_LAYER_A=(
    # mika-agent's `web_search` builtin still calls Brave directly with its
    # own `X-Subscription-Token` header. The agent-side migration to
    # `POST /internal/search` on the gateway is tracked separately (E2 body
    # + follow-up); once the builtin is removed, drop this entry.
    "crates/mika-agent/src/skills/builtin_handlers.rs"
)

# Files where Layer B header patterns are known-legitimate (auth infra for
# internal APIs unrelated to search egress). Kept narrow — if you add a new
# entry here, comment WHY.
AUTHORIZED_PATHS_LAYER_B=(
    # This lint script's own grep patterns (self-referential — the patterns
    # here contain the words `tenant`, `user`, etc. by design).
    "scripts/verify-egress-request-shape.sh"
)

is_allowed_layer_a() {
    local path="$1"
    local entry
    for entry in "${AUTHORIZED_PATHS_LAYER_A[@]}" "${LEGACY_ALLOWLIST_LAYER_A[@]}"; do
        if [[ "$path" == *"$entry"* ]]; then
            return 0
        fi
    done
    return 1
}

is_allowed_layer_b() {
    local path="$1"
    local entry
    for entry in "${AUTHORIZED_PATHS_LAYER_B[@]}"; do
        if [[ "$path" == *"$entry"* ]]; then
            return 0
        fi
    done
    return 1
}

violations=0

# -- Layer A sweep: Brave auth header outside the substrate --
for pat in "${LAYER_A_PATTERNS[@]}"; do
    while IFS= read -r hit; do
        file="${hit%%:*}"
        # Strip $REPO_ROOT prefix for allowlist matching to work with the
        # short-path entries above.
        rel="${file#"$REPO_ROOT"/}"
        if is_allowed_layer_a "$rel"; then
            continue
        fi
        echo "ERROR (egress-request-shape, Layer A): Brave-auth header '$pat' referenced outside the substrate at $hit"
        violations=$((violations + 1))
    done < <(grep -rnF "$pat" "$REPO_ROOT/crates/" "$REPO_ROOT/docs/" 2>/dev/null || true)
done

# -- Layer B sweep: forbidden per-tenant fields anywhere in the egress module --
#
# This is a shape lint on the substrate itself, complementing the compile-time
# E3 test. If someone adds a `.header("X-Mika-Tenant-Id", ...)` inside
# egress_search/*, the compile-time test catches it AND this script catches
# it as a second defense at grep time (fast local run, no cargo needed).
for pat in "${LAYER_B_FORBIDDEN_HEADER_PATTERNS[@]}"; do
    while IFS= read -r hit; do
        file="${hit%%:*}"
        rel="${file#"$REPO_ROOT"/}"
        # Layer B: scan the egress module tree AND the top-level scripts/ dir
        # (this script itself contains the pattern; excluded via
        # AUTHORIZED_PATHS_LAYER_B).
        if is_allowed_layer_b "$rel"; then
            continue
        fi
        # Only report hits inside the egress-search module tree — Layer B is
        # a shape lint on the substrate, not on unrelated auth code elsewhere
        # in the gateway. Broader grep would false-positive.
        case "$rel" in
            crates/mika-gateway/src/egress_search/*)
                echo "ERROR (egress-request-shape, Layer B): forbidden identity-shaped field inside the substrate at $hit"
                violations=$((violations + 1))
                ;;
        esac
    done < <(grep -rniE "$pat" "$REPO_ROOT/crates/mika-gateway/src/egress_search/" 2>/dev/null || true)
done

if [[ $violations -gt 0 ]]; then
    echo ""
    echo "Found $violations egress-request-shape violation(s)."
    echo ""
    echo "The E3 dé-liage invariant (mika#1809) requires:"
    echo "  1) Only crates/mika-gateway/src/egress_search/* may construct a"
    echo "     request against the Brave upstream (bearing 'X-Subscription-Token')."
    echo "  2) No file inside egress_search/* may add a header / query param"
    echo "     whose lowercase name contains 'tenant', 'user', 'agent',"
    echo "     'customer', 'session', 'trace', 'correlation', or 'request_id'."
    echo ""
    echo "See crates/mika-gateway/docs/egress-search-threat-model.md for the"
    echo "full threat model + LIAGE-side invariant documentation."
    exit 1
fi

echo "No egress-request-shape violations found."
exit 0
