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
SCRIPT_PATH="$0"

# The E1 substrate directory. CI passes no argument and the default path is
# scanned. The test harness (`scripts/test-verify-egress-no-log.sh`) passes a
# fixture directory as $1 so the lint's negative behaviour can be pinned
# against synthesized source it never touches in production.
EGRESS_DIR="${1:-$REPO_ROOT/crates/mika-gateway/src/egress_search}"

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

# Shared awk body — returns the line stripped of its string literals and of its
# trailing `//` comment, so delimiter counting reads SYNTAX and not prose.
#
# ONE definition, deliberately (mika#2054 R5). Both parsers in this file depend
# on it: the `#[cfg(test)]` scope tracker below, which decides what gets audited
# at all, and `macro_block()`, which delimits an `info!` call. A second copy
# breaks nothing the day it is written — it lets one parser stay correct while
# the other silently regresses, which is the divergence nobody notices. Pinned
# by `scripts/test-verify-egress-no-log.sh` (exactly one definition, at least
# two call sites); when that scan fires you interpolate this constant, you do
# not add an exemption.
#
# Known and accepted limits: no block comments (`/* */`), no raw strings
# (`r#"…"#`), no string spanning lines. Simple char literals (`'{'`, `'\''`) ARE
# dropped (mika#2054 review). None of the unmodeled forms occurs in the scanned
# substrate. A single mis-count fails closed at end-of-file; two opposite
# mis-counts in two test modules would cancel there, so the scope tracker also
# refuses a `#[cfg(test)]` met while it believes itself inside a test module.
# That covers mis-counts that straddle a later test module, not every pairing:
# the limits above stay limits.
readonly AWK_SANITIZE_FN='
function sanitize(s,   out, i, c, n, instr, prev) {
    out = ""; instr = 0; prev = ""
    n = length(s)
    for (i = 1; i <= n; i++) {
        c = substr(s, i, 1)
        if (instr) {
            if (c == "\"" && prev != "\\") instr = 0
            prev = c; continue
        }
        # Char literal (quote, one char, quote; or quote, backslash, char,
        # quote): dropped whole, so a brace char counts no brace and a
        # double-quote char opens no string. A lifetime has no closing quote at
        # either offset and passes through untouched. \047 is the single quote,
        # spelled so because this program sits inside a shell single-quoted
        # string.
        if (c == "\047" && substr(s, i + 2, 1) == "\047") { i += 2; prev = ""; continue }
        if (c == "\047" && substr(s, i + 1, 1) == "\\" && substr(s, i + 3, 1) == "\047") { i += 3; prev = ""; continue }
        if (c == "\"") { instr = 1; prev = c; continue }
        if (c == "/" && substr(s, i + 1, 1) == "/") break
        out = out c; prev = c
    }
    return out
}
'

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
# brace, then resumes. Both the opening line and every line after it are
# counted on their SANITIZED form (mika#2054 R4) — see the two failures that
# forced it, below.
#
# Between the attribute and its item, Rust routinely puts lines that carry no
# item at all: a blank line, a `//` comment, a further attribute on a line of
# its own. Those are TRAVERSED without consuming the pending state (mika#2054
# R2); the first meaningful line decides. A line that carries an item — on the
# attribute's own line, or after a traversed attribute — is never traversed: it
# is refused, or the state would carry past it onto the next production item.
# A second `cfg` attribute is not neutral and fails closed — two cfg conditions
# on one item make the item undecidable here.
#
# Any other `#[cfg(test)]` form (bare `fn`, `impl`, `struct`, …) is a shape
# this parser does NOT model. It cannot delimit the test scope, so it cannot
# know where production code resumes. Rather than silently drop the rest of
# the file from the scan — which turns every egress violation below that point
# into a false green — the parser FAILS CLOSED: it prints a diagnostic that
# names the offending line and says what to add to the parser, then exits
# non-zero so the whole guard fails. Construct the incapacity, don't promise
# the restraint (header, :8-11). This is the same class as mika#2039's parser,
# which returned a partial audit indistinguishable from a complete one.
#
# Exit codes, each mapped to its own operator-visible pattern by the caller:
#   3 — a `#[cfg(test)]` form this parser does not model
#   4 — an inline test block whose braces never balance (scope undeterminable)
#   5 — a `#[cfg(test)]` attribute with no item before end-of-file
production_lines() {
    local file="$1"
    awk -v script="$SCRIPT_PATH" -v src="$file" "$AWK_SANITIZE_FN"'
        BEGIN {
            pending_cfg_test = 0; pending_line = 0
            brace_depth = 0; in_inline_test = 0; open_line = 0
            bailed = 0
        }

        # Inside an inline test module. Count SANITIZED delimiters: the raw
        # count included braces living in string literals and comments, so an
        # unpaired `{` in a test string pushed the depth up and ran the skip
        # past the module, swallowing the production code behind it — a
        # fail-open. (An excess `}` ended the skip early and audited test code
        # as production: same cause, opposite sign.) mika#2054 D3b.
        in_inline_test == 1 {
            # A `#[cfg(test)]` while still inside a test module means the count
            # went wrong somewhere above: two opposite mis-counts in two modules
            # would otherwise cancel before end-of-file, and the skip would have
            # run over the production code between them with exit 0. Refuse.
            if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) {
                printf("ERROR (egress-no-log, parser): `#[cfg(test)] mod` (block undeterminable) at %s:%d: a `#[cfg(test)]` appears while the inline test module opened at %s:%d is still counted open.\n", src, FNR, src, open_line) > "/dev/stderr"
                printf("  The brace count went wrong above, so the parser cannot say where production code resumed. Refusing to emit a partial audit.\n") > "/dev/stderr"
                printf("  Remove the unmodeled brace (block comment, raw string, multi-line string), or extend sanitize() in %s to model it, then re-run.\n", script) > "/dev/stderr"
                bailed = 1
                exit 4
            }
            s = sanitize($0)
            t = s; n_open = gsub(/\{/, "", t)
            t = s; n_close = gsub(/\}/, "", t)
            brace_depth += n_open - n_close
            if (brace_depth <= 0) {
                in_inline_test = 0
                brace_depth = 0
            }
            next
        }

        # Recognize the `#[cfg(test)]` attribute — decision made on the next
        # MEANINGFUL line. Re-arming on a repeated `#[cfg(test)]` is correct:
        # the condition is unchanged, so the item stays decidable.
        /^[[:space:]]*#\[cfg\(test\)\]/ {
            pending_cfg_test = 1
            pending_line = FNR
            # An item on the SAME line (`#[cfg(test)] use x;`) is not a line
            # that carries no item: arming `pending` past it let the traversal
            # below carry the state to the next production `mod NAME {` and
            # skip it as test code, exit 0. Decide nothing here; fail closed.
            rest = sanitize($0)
            sub(/^[[:space:]]*#\[cfg\(test\)\]/, "", rest)
            if (rest !~ /^[[:space:]]*$/) {
                printf("ERROR (egress-no-log, parser): unmodeled `#[cfg(test)]` form at %s:%d: %s\n", src, FNR, $0) > "/dev/stderr"
                printf("  The attribute and its item share one line; production_lines() models the item only on a line of its own.\n") > "/dev/stderr"
                printf("  Put the item on the next line, or extend production_lines() in %s to model this form, then re-run.\n", script) > "/dev/stderr"
                bailed = 1
                exit 3
            }
            next
        }

        pending_cfg_test == 1 {
            # --- Lines that carry no item: traverse, do NOT consume the state.
            # This branch is mika#2054 D1. The state used to be consumed by the
            # next line whatever it was, so a blank line, a comment or a second
            # attribute fell into the fail-closed branch below: the guard went
            # red on ordinary Rust, naming a blank line and prescribing a `mod`
            # wrapper for code that already had one. A guard that reddens on
            # legitimate code, with a diagnostic that does not describe the
            # fault, is a guard that gets disarmed.
            if ($0 ~ /^[[:space:]]*$/) { next }
            if ($0 ~ /^[[:space:]]*\/\//) { next }
            if ($0 ~ /^[[:space:]]*#\[/) {
                # …but a second `cfg` is not a neutral intercalary. Two cfg
                # conditions on one item make the item undecidable for this
                # parser, and `#[cfg(test)]` then `#[cfg(not(test))]` is
                # outright contradictory. The predicate is SYNTACTIC — does the
                # attribute start with `#[cfg`? — so it needs no model of
                # composed cfg forms. It refuses a stack that would in
                # principle be coherent; that is the named price, and the right
                # side of the trade: arbitrating two cfg conditions is not this
                # parser'"'"'s job.
                if ($0 ~ /^[[:space:]]*#\[cfg/) {
                    printf("ERROR (egress-no-log, parser): stacked `cfg` attribute at %s:%d: %s\n", src, FNR, $0) > "/dev/stderr"
                    printf("  It follows a `#[cfg(test)]` opened at %s:%d, so which item this attributes to is undecidable here.\n", src, pending_line) > "/dev/stderr"
                    printf("  Refusing to guess. Put the test code under a single `#[cfg(test)] mod NAME { ... }` block, or extend production_lines() in %s to model stacked cfg conditions, then re-run.\n", script) > "/dev/stderr"
                    bailed = 1
                    exit 3
                }
                # Traverse only a line that is the attribute and nothing else.
                # `#[allow(..)] use x;` carries an item: it falls through to
                # the decision below, which refuses it, instead of being skipped
                # while `pending` stays armed for the next production item.
                if (sanitize($0) ~ /^[[:space:]]*#\[[^]]*\][[:space:]]*$/) { next }
            }

            pending_cfg_test = 0
            # External mod declaration — `mod NAME;` — does not wrap code.
            if ($0 ~ /^[[:space:]]*mod [A-Za-z0-9_]+;[[:space:]]*$/) {
                next
            }
            # Inline mod block — `mod NAME {`. Count the SANITIZED delimiters
            # OF THIS LINE instead of assuming depth 1: `mod tests { fn t() {} }`
            # closes on the line that opens it, and posting depth 1 there left
            # the parser in skip for the REST OF THE FILE — every production
            # line after it dropped from the scan, a partial audit that reads
            # exactly like a clean one. mika#2054 D3a, the ticket'"'"'s own class.
            if ($0 ~ /^[[:space:]]*mod [A-Za-z0-9_]+[[:space:]]*\{/) {
                s = sanitize($0)
                t = s; opens = gsub(/\{/, "", t)
                t = s; closes = gsub(/\}/, "", t)
                depth = opens - closes
                if (depth <= 0) { next }
                in_inline_test = 1
                brace_depth = depth
                open_line = FNR
                next
            }
            # Anything else after `#[cfg(test)]` — bare `fn`, `impl`, `struct`,
            # an item after an attribute on one line, etc. — is a form this parser
            # cannot delimit. Do NOT abandon the file silently: name the line,
            # say what to add, and fail non-zero so the guard fails closed.
            printf("ERROR (egress-no-log, parser): unmodeled `#[cfg(test)]` form at %s:%d: %s\n", src, FNR, $0) > "/dev/stderr"
            printf("  production_lines() models only `#[cfg(test)] mod NAME;` and `#[cfg(test)] mod NAME {`.\n") > "/dev/stderr"
            printf("  It cannot delimit this item, so the remainder of the file would go unscanned — a partial audit that reads exactly like a clean one.\n") > "/dev/stderr"
            printf("  Refusing to emit it. Either wrap the test code in a `#[cfg(test)] mod NAME { ... }` block, or extend production_lines() in %s to model this form (skip-until-end-of-item), then re-run.\n", script) > "/dev/stderr"
            bailed = 1
            exit 3
        }

        { printf("%s:%d:%s\n", src, FNR, $0) }

        # `exit` runs END, so both arms are guarded by `bailed` — without it a
        # body bail would be re-diagnosed here under the wrong name and the
        # wrong exit code.
        END {
            if (!bailed && in_inline_test == 1 && brace_depth > 0) {
                printf("ERROR (egress-no-log, parser): `#[cfg(test)] mod` (block undeterminable) — the inline test module opened at %s:%d never closes (%d unmatched `{` at end of file).\n", src, open_line, brace_depth) > "/dev/stderr"
                printf("  The parser cannot say where production code resumes, so the rest of the file would go unscanned. Refusing to emit a partial audit.\n") > "/dev/stderr"
                printf("  Close the block, or extend production_lines() in %s if the module legitimately ends some other way, then re-run.\n", script) > "/dev/stderr"
                exit 4
            }
            if (!bailed && pending_cfg_test == 1) {
                printf("ERROR (egress-no-log, parser): dangling `#[cfg(test)]` at %s:%d — the attribute has no item before end of file.\n", src, pending_line) > "/dev/stderr"
                printf("  That is code which does not compile; staying silent would make a truncated file indistinguishable from a healthy one.\n") > "/dev/stderr"
                printf("  Give the attribute its item, then re-run.\n") > "/dev/stderr"
                exit 5
            }
        }
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
#
# `production_lines` fails closed (exit != 0) when it meets a `#[cfg(test)]`
# form it cannot model. Capture that per-file: a parser failure on ANY file is
# a hard guard failure — a lint that stops modelling a file must stop LOUDLY,
# never pass a partial audit.
prod_lines=""
while IFS= read -r file; do
    file_prod=""
    parse_rc=0
    file_prod="$(production_lines "$file")" || parse_rc=$?
    if [[ $parse_rc -ne 0 ]]; then
        # Each refusal gets its own operator-visible pattern: "unknown shape",
        # "block left open" and "attribute with no item" call for three
        # different fixes, and one shared label would hide which. The default
        # arm also catches an awk failure we did not anticipate — fail closed
        # on the unexpected rather than assume the familiar cause.
        case $parse_rc in
            4) parse_pattern="#[cfg(test)] mod (block undeterminable)" ;;
            5) parse_pattern="dangling #[cfg(test)] attribute" ;;
            *) parse_pattern="unmodeled #[cfg(test)] form" ;;
        esac
        report_violation "parser" "$parse_pattern" "$file"
        echo "  the file above could not be fully modelled; the guard refuses to"
        echo "  pass a partial audit (see the parser diagnostic printed above)."
    fi
    prod_lines+="$file_prod"$'\n'
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

# 1c — `info!` calls must belong to the Q4 allowlist. For each `info!(`
# invocation, read its ACTUAL macro block — from the opening line until the
# `)` that balances the `(` opened by `info!(` — and require an allowlisted
# `event = "<name>"` token inside that exact span.
#
# The old form read a fixed 8-line window below the call (`line_no + 8`). That
# is a fail-open parser: a non-allowlisted `info!` passed whenever an
# allowlisted `event = "…"` token merely happened to sit within 8 lines below
# it, in an unrelated statement. `macro_block` (below) attaches to the call's
# own parenthesised argument list instead, and if it cannot balance the parens
# — a truncated file, a macro it cannot delimit — it FAILS CLOSED (exit != 0)
# and the guard fails, rather than judging the call on an arbitrary window.
info_hits=$(printf "%s\n" "$live_lines" | grep -F "info!(" || true)

# Print the lines of the `info!(` macro invocation that begins at line `start`
# of `file` — the opener through the line whose closing `)` returns paren
# depth to zero. Parens inside `//` line comments and double-quoted strings do
# not count. Exit 2 if the block never closes (parens cannot be balanced).
macro_block() {
    local file="$1" start="$2"
    awk -v start="$start" "$AWK_SANITIZE_FN"'
        NR < start { next }
        {
            print
            s = sanitize($0)
            t = s; opens = gsub(/\(/, "", t)
            t = s; closes = gsub(/\)/, "", t)
            depth += opens - closes
            started = 1
            if (depth <= 0) exit 0
        }
        END { if (started && depth > 0) exit 2 }
    ' "$file"
}

if [[ -n "$info_hits" ]]; then
    while IFS= read -r hit; do
        [[ -z "$hit" ]] && continue
        file="${hit%%:*}"
        rest="${hit#*:}"
        line_no="${rest%%:*}"

        block=""
        block_rc=0
        block=$(macro_block "$file" "$line_no") || block_rc=$?

        if [[ $block_rc -ne 0 ]]; then
            report_violation "layer-1" "info! (block undeterminable)" "$hit"
            echo "  the info! invocation above could not be delimited (its"
            echo "  parentheses do not balance before end-of-file); the guard"
            echo "  refuses to judge it on an incomplete block."
            continue
        fi

        matched=0
        for allowed in "${ALLOWED_EVENT_NAMES[@]}"; do
            if grep -qF "event = \"${allowed}\"" <<< "$block"; then
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
