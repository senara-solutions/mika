#!/usr/bin/env bash
# CI lint (mika#1810 E4 — no-log-verified-end-to-end): the egress-search
# substrate at `crates/mika-gateway/src/egress_search/` MUST leak nothing to
# any observable side channel — no application-log call, no `println`/`dbg`,
# no filesystem write, no SQL write. Adding one is a Q4 STRIP TOTAL / Prime
# no-retention violation.
#
# Discipline analog: `scripts/verify-egress-uniqueness.sh` (#1807) — construct
# the incapacity, don't promise the restraint. Same shape as
# `scripts/check-byte-slices.sh` (#764) and `scripts/check-loop-select.sh`
# (#848).
#
# Scope: PRODUCTION code inside `egress_search/*.rs` — everything before the
# first `#[cfg(test)]` marker in each file. Test-only code (CapturingLayer,
# wiremock harness) is allowed to use `tracing::field::Visit`, subscribers,
# etc. — that discipline is enforced at the *emit* side (production), not
# at the visit side (tests).
#
# Coverage:
#   Layer 1 (app logs)     — forbid `info!/warn!/error!/debug!/trace!` macros
#                            except the two E1 audit events (`search_requested`,
#                            `search_egress`) already covered by the Q4
#                            CapturingLayer test. Also forbid `println!`,
#                            `eprintln!`, `print!`, `eprint!`, `dbg!`, and
#                            `log::*` macros anywhere.
#   Layer 3 (persistence)  — forbid `File::create`, `std::fs::write`,
#                            `OpenOptions`, `write_all`, `sqlx::query`,
#                            `insert_into`, `INSERT INTO`, `rusqlite`.
#
# Layer 2 (network metadata — iptables/nft/proxy) is *substrate spec*, not a
# source-code invariant. See `crates/mika-gateway/docs/egress-search-no-log-audit.md`
# for the deploy-side rules and `scripts/audit-egress-no-log.sh` for the
# runtime probe.
#
# Exit codes:
#   0 — clean
#   1 — violation(s) found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EGRESS_DIR="$REPO_ROOT/crates/mika-gateway/src/egress_search"

if [[ ! -d "$EGRESS_DIR" ]]; then
    echo "ERROR: egress_search directory not found at $EGRESS_DIR"
    echo "The E1 substrate is expected to live at this path; if the module was"
    echo "moved, update EGRESS_DIR in $0 and re-run."
    exit 1
fi

# Q4 allowlist — the two INFO events the E1 CapturingLayer test already
# enforces the field-shape of. Any `info!(` call whose block does NOT include
# one of these `event = "..."` tokens is a discipline violation.
ALLOWED_EVENT_NAMES=(
    "search_requested"
    "search_egress"
)

# Forbidden logging macros — any occurrence is a hard fail regardless of
# arguments. `info!` is handled separately because of the allowlist above.
FORBIDDEN_LOG_MACROS=(
    "debug!"
    "warn!"
    "error!"
    "trace!"
    "println!"
    "eprintln!"
    "print!"
    "eprint!"
    "dbg!"
)

# Forbidden fully-qualified logging macros — catch both `use log::info; info!(...)`
# and inlined `log::info!(...)` shapes even when the same short name is
# elsewhere disallowed.
FORBIDDEN_LOG_QUALIFIED_MACROS=(
    "log::info!"
    "log::debug!"
    "log::warn!"
    "log::error!"
    "log::trace!"
    "tracing::debug!"
    "tracing::warn!"
    "tracing::error!"
    "tracing::trace!"
)

# Forbidden persistence / disk / DB patterns — the substrate MUST NOT write
# anywhere. Prime 2026-07-19: zero rétention.
FORBIDDEN_PERSISTENCE_PATTERNS=(
    "File::create"
    "std::fs::write"
    "fs::write("
    "OpenOptions"
    "write_all"
    "sqlx::query"
    "insert_into"
    "INSERT INTO"
    "rusqlite"
)

violations=0

# Emit each production-code file (everything OUTSIDE `#[cfg(test)]` scopes)
# with `file:lineno:content` shape so the checks below can grep without
# extra bookkeeping.
#
# `#[cfg(test)] mod NAME;` (external file declaration, ends with `;`) is
# treated as a declaration only — it does NOT wrap production code, so the
# scan continues past it.
#
# `#[cfg(test)] mod NAME {` (inline block, ends with `{`) enters an
# inline-test scope; the scan skips every line until the matching closing
# brace, then resumes.
#
# Any other `#[cfg(test)]` (bare `fn`, other item forms) also opens a
# skip-until-end-of-item scope — we stop the scan there because the E1/E2
# substrate does not use those forms today and adding one would need a
# lint update anyway.
production_lines() {
    local file="$1"
    awk '
        BEGIN { pending_cfg_test = 0; brace_depth = 0; in_inline_test = 0 }

        # Skip anything inside an inline test module (tracked via brace depth).
        in_inline_test == 1 {
            n_open = gsub(/\{/, "{")
            n_close = gsub(/\}/, "}")
            brace_depth += n_open - n_close
            if (brace_depth <= 0) {
                in_inline_test = 0
                brace_depth = 0
            }
            next
        }

        # Recognize the `#[cfg(test)]` attribute — decision made on the NEXT
        # meaningful line.
        /^[[:space:]]*#\[cfg\(test\)\]/ {
            pending_cfg_test = 1
            next
        }

        pending_cfg_test == 1 {
            pending_cfg_test = 0
            # External mod declaration — `mod NAME;` — does not wrap code.
            if ($0 ~ /^[[:space:]]*mod [A-Za-z0-9_]+;[[:space:]]*$/) {
                next
            }
            # Inline mod block — `mod NAME {` — enter skip-until-close scope.
            if ($0 ~ /^[[:space:]]*mod [A-Za-z0-9_]+[[:space:]]*\{/) {
                in_inline_test = 1
                brace_depth = 1
                next
            }
            # Anything else after `#[cfg(test)]` — bare fn, other item — is
            # test-scoped. Stop scanning the file (defensive: adding a
            # `#[cfg(test)] fn` next to production code is unusual and worth
            # a manual lint revisit).
            exit
        }

        { printf("%s:%d:%s\n", FILENAME, FNR, $0) }
    ' "$file"
}

# Strip Rust single-line comment lines (`//` or `///` or `//!`) — these are
# discipline commentary and never carry live macro calls. Kept as a simple
# line filter; inline comments on live-code lines are not stripped because
# a macro call followed by an explanatory `//` on the same line IS a live
# call and must be flagged.
strip_comment_lines() {
    grep -Ev '^[^:]+:[0-9]+:[[:space:]]*//' || true
}

report_violation() {
    local layer="$1"
    local pattern="$2"
    local hit="$3"
    echo "ERROR (egress-no-log, $layer): forbidden pattern '$pattern' at $hit"
    violations=$((violations + 1))
}

# ------------------------------------------------------------------
# Layer 1 — application logs
# ------------------------------------------------------------------

# All production lines across every source file in egress_search/.
prod_lines=""
while IFS= read -r file; do
    prod_lines+="$(production_lines "$file")"$'\n'
done < <(find "$EGRESS_DIR" -type f -name '*.rs' | sort)

# Live (non-comment-line) production lines — the surface the lint operates on.
live_lines=$(printf "%s" "$prod_lines" | strip_comment_lines)

# 1a — forbidden log-macro short names.
for pat in "${FORBIDDEN_LOG_MACROS[@]}"; do
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        report_violation "layer-1" "$pat" "$hit"
    done < <(printf "%s\n" "$live_lines" | grep -F "$pat(" || true)
done

# 1b — forbidden fully-qualified log macros (log::*!/tracing::*! non-info).
for pat in "${FORBIDDEN_LOG_QUALIFIED_MACROS[@]}"; do
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        report_violation "layer-1" "$pat" "$hit"
    done < <(printf "%s\n" "$live_lines" | grep -F "${pat}(" || true)
done

# 1c — `info!` calls must belong to the Q4 allowlist. Build the set of
# allowed line numbers per file by finding `info!(` invocations whose block
# (the call itself + the following 8 lines) contains one of the allowed
# `event = "<name>"` tokens.
info_hits=$(printf "%s\n" "$live_lines" | grep -F "info!(" || true)

if [[ -n "$info_hits" ]]; then
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        file="${hit%%:*}"
        rest="${hit#*:}"
        line_no="${rest%%:*}"

        # Read a small window of the source file to see whether this info!
        # call belongs to an allowlisted audit event. The E1 events open the
        # macro on one line and name `event = "…"` within the following few.
        window_end=$((line_no + 8))
        window=$(sed -n "${line_no},${window_end}p" "$file")

        matched=0
        for allowed in "${ALLOWED_EVENT_NAMES[@]}"; do
            if grep -qF "event = \"${allowed}\"" <<< "$window"; then
                matched=1
                break
            fi
        done

        if [[ $matched -eq 0 ]]; then
            report_violation "layer-1" "info! (not in Q4 allowlist)" "$hit"
        fi
    done <<< "$info_hits"
fi

# ------------------------------------------------------------------
# Layer 3 — persistence
# ------------------------------------------------------------------
#
# Persistence must not appear ANYWHERE in the egress_search tree (production
# OR tests) — a test that writes to disk is itself a signal the substrate
# grew a persistence path.

all_lines=""
while IFS= read -r file; do
    all_lines+="$(awk '{ printf("%s:%d:%s\n", FILENAME, FNR, $0) }' "$file")"$'\n'
done < <(find "$EGRESS_DIR" -type f -name '*.rs' | sort)
all_live_lines=$(printf "%s" "$all_lines" | strip_comment_lines)

for pat in "${FORBIDDEN_PERSISTENCE_PATTERNS[@]}"; do
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        report_violation "layer-3" "$pat" "$hit"
    done < <(printf "%s\n" "$all_live_lines" | grep -F "$pat" || true)
done

# ------------------------------------------------------------------
# Report
# ------------------------------------------------------------------

if [[ $violations -gt 0 ]]; then
    echo ""
    echo "Found $violations egress no-log violation(s)."
    echo ""
    echo "The E4 invariant (mika#1810): the egress-search substrate must leak"
    echo "nothing to any observable side channel. Only the two E1 audit events"
    echo "(search_requested + search_egress) are permitted, and only inside"
    echo "$EGRESS_DIR."
    echo ""
    echo "To resolve:"
    echo "  * Layer 1 (logs)         — drop the new log call; the audit event"
    echo "                             already carries the taxonomy label."
    echo "  * Layer 3 (persistence)  — drop the write; queries and responses"
    echo "                             MUST NOT persist anywhere on our side."
    echo ""
    echo "If you are adding a genuinely-new audit event (rare — requires Prime"
    echo "bearing), extend ALLOWED_EVENT_NAMES in $0 AND extend the Q4"
    echo "allowlist test in crates/mika-gateway/src/egress_search/mod.rs."
    exit 1
fi

echo "No egress no-log violations found."
exit 0
