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
# SOURCE UNIQUE — mika#2408 AC5. Cette liste vivait ici, maintenue à la main,
# et rien ne forçait son entrée : un nouvel upstream absent de la liste passait
# en silence. Elle est désormais DÉRIVÉE de `docs/egress/egress-manifest.toml`
# — les `destination` (ou `confined_hosts`) des entrées portant
# `confined = true` — que le lockstep de `scripts/verify-egress-manifest.sh`
# tient en phase avec le code dans les deux sens.
#
# Scope discipline: this list catches real API endpoint hosts / paths — the
# strings that appear ONLY in code performing an actual network call. Marketing
# URLs (like the free-key sign-up landing page of a search upstream) are
# intentionally out of scope; they cannot reach the upstream and their presence
# in docs is legitimate. C'est pourquoi le champ `confined` du manifeste est
# OPT-IN : tout sink n'est pas confinable, et seuls les substrats le sont.
#
# FAIL-CLOSED. Une dérivation qui échouerait — manifeste absent, TOML cassé,
# zéro entrée `confined` — laisserait ce lint tourner sur un tableau vide et
# rendre « aucune violation », c'est-à-dire se lire exactement comme une garde
# qui passe. On refuse plutôt que de continuer.
MANIFEST_ENGINE="$REPO_ROOT/scripts/lib/egress_manifest_lint.py"
if [[ ! -f "$MANIFEST_ENGINE" ]]; then
    echo "ERROR (egress-uniqueness): moteur du manifeste introuvable à $MANIFEST_ENGINE" >&2
    echo "  Les PATTERNS de ce lint en sont dérivés depuis mika#2408 (AC5)." >&2
    echo "  Sans lui, ce lint tournerait sur une liste vide et se lirait comme vert." >&2
    exit 2
fi

PATTERNS=()
derive_rc=0
# La capture est FAITE AVANT la boucle, et c'est ce qui rend `derive_rc` vrai.
# La forme `done < <(cmd || { derive_rc=$?; })` place l'affectation dans le
# sous-shell de la substitution de processus : elle est perdue au retour, et le
# refus ci-dessous rapportait « exit 0 » sur une dérivation qui avait bel et
# bien échoué (mesuré le 2026-09-29 avec un `tomllib` absent). Seul le second
# terme du refus mordait alors — donc une dérivation qui échouerait en écrivant
# tout de même une ligne ne serait pas refusée du tout.
derived=$(python3 -B "$MANIFEST_ENGINE" --confined-hosts "$REPO_ROOT") || derive_rc=$?
while IFS= read -r host; do
    [[ -n "$host" ]] && PATTERNS+=("$host")
done <<< "$derived"

if [[ $derive_rc -ne 0 || ${#PATTERNS[@]} -eq 0 ]]; then
    echo "ERROR (egress-uniqueness): dérivation des PATTERNS depuis le manifeste échouée" >&2
    echo "  (exit $derive_rc, ${#PATTERNS[@]} pattern(s) obtenu(s))" >&2
    echo "  Source: docs/egress/egress-manifest.toml, entrées \`confined = true\`." >&2
    echo "  Diagnostic: bash scripts/verify-egress-manifest.sh --report" >&2
    exit 2
fi

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
    # mika#1971 web_search substrate-routing plan — describes what the
    # consumer-side migration removes; must cite the pre-migration Brave
    # URL to document the fix. Same shape as the E1 plan above.
    "docs/plans/2026-08-23-003-fix-1971-web-search-substrate-routing-plan.md"
    # Egress manifest (mika#2408) — le manifeste nomme chaque destination par
    # définition : c'est son objet. Il ne réalise aucun appel, et une
    # déclaration est le contraire d'un chemin de reachability. Sans cette
    # entrée, le premier commit du manifeste fait rougir ce lint.
    "docs/egress/"
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
    # fetch-url skill prompt (mika#1988) — enumerates the compile-time
    # gouv.fr allowlist in prose so the LLM knows which hosts fetch_url
    # accepts. The skill itself never egresses — it declares the builtin
    # `fetch_url` which delegates to /internal/fetch on the gateway. The
    # substrate-side allowlist in `crates/mika-gateway/src/egress_fetch/`
    # remains the sole enforcement point.
    "crates/mika-agent/templates/skills/fetch-url/system_prompt.md"
    # shell-exec egress-containment doctrine (mika#1991) — cites the T1
    # measurement, which names the gouv.fr hosts reached (and not reached) in
    # prose to explain WHY run_shell+curl had to be closed. Documentation only;
    # names no enforcement path. Same category as the fetch-url prompt and the
    # grounding-regression fixtures above. The shell-exec handler itself and
    # its test deliberately do NOT name these hosts — the gate is host-agnostic
    # (it refuses all direct egress), so the allowlist stays the sole property
    # of crates/mika-gateway/src/egress_fetch/.
    "docs/solutions/best-practices/optional-path-is-no-guarantee-2026-08-30.md"
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

# CONTRÔLE POSITIF (mika#2408). Depuis que les PATTERNS sont dérivés du
# manifeste, « aucune violation » a deux causes possibles : l'arbre est propre,
# ou la liste a rétréci sans qu'on le voie. Dire COMBIEN de patterns ont été
# confrontés est ce qui sépare les deux — sans cette ligne, une dérivation
# tombée de cinq hosts à un se lirait exactement comme une flotte saine
# (classe mika#2205).
echo "No egress-uniqueness violations found (${#PATTERNS[@]} confined host(s) from docs/egress/egress-manifest.toml)."
exit 0
