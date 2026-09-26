#!/usr/bin/env bash
# Enumerates every STRICT match site for a machine token in the tree (mika#2201).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THIS SCRIPT EXISTS AT ALL
#
# mika#2201's closing bound, from Prime: "the canonical list must be EXHAUSTIVE
# against the code that matches. One token forgotten, and 'seconde passe' comes
# back under another name."
#
# The plan's first revision presented its inventory as "surveyed on the tree at
# 17f42a6b" and shipped no gesture producing it. That was a promise, not a
# proof — and the revision demonstrated it on itself: the manual survey was
# *exact* on every line it carried and *incomplete*, missing
# `dispatch-lib.sh::_extract_plan_path`, which is the strictest reader of the
# `Plan` callout in the whole repository. A manual survey can be right and still
# miss a site; that is precisely the class AC4 exists to close, and founding AC2
# on the mechanism AC4 declares insufficient would be incoherent.
#
# So the list is PRODUCED, by this script, and confronted to the tree by
# `--check`. That mode is the SHELL HALF of the AC4 exhaustiveness guard, and it
# checks exactly one direction: every strict site in the tree is declared. The
# stale direction belongs to the Rust half
# (`canonical_tokens::tests::mika2201_every_declared_symbol_still_exists`); the
# comment above the comparison says why a two-way comparison cannot work on this
# particular list and what forcing it was measured to cost.
#
# The two halves are deliberately complementary rather than redundant: this one
# starts from the reading FORM, the Rust one from the TOKEN. Neither is the
# other's copy, so neither can silently disagree with it — which is the shape
# `grooming_marker.rs` had to write down after mika#2158, where two regexes
# answering the same question diverged for months and nothing broke.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHAT A "STRICT MATCH SITE" IS, AND WHY THE PREDICATE IS ON THE READER
#
# The naive rule — "a machine token is canonical English, anything else is a
# variant" — accuses two shapes this repository reads DELIBERATELY WELL:
# `seconde passe` (read by `LATER_PASS_RE`, `(?i)` + FR alternation, bilingual
# by written decision since mika#2158) and the paraphrase tiers of
# `_parse_disposition_fuzzy`. A lint built on it is red on the day it is born,
# gets disarmed, and the regression it exists to catch passes in the noise.
# That is the failure mode of `check-a2a-timeout-literals.sh` § M1, reached by
# the remedy.
#
# So the discriminator is not the language of the token. It is the TOLERANCE OF
# ITS READER:
#
#   * a reader that is `(?i)` / bilingual / paraphrase-tiered is TOLERANT —
#     a variant is READ, and the lint must stay silent (class A);
#   * a reader that is line-anchored, case-sensitive, or a bare `starts_with`
#     is STRICT — a variant is MIS-read, and the lint must fire (class B).
#
# This script enumerates the STRICT ones. Deciding tolerant-vs-strict is a
# reading of the reader's code, not a textual motif, so it stays the one column
# `scripts/canonical-tokens.tsv` carries by hand (`classe`).
#
# ─────────────────────────────────────────────────────────────────────────────
# A SITE IS NAMED BY ITS SYMBOL, NEVER BY A LINE NUMBER
#
# A line number rots on the first commit inserting a line above it, and it rots
# IN SILENCE: the column stays syntactically valid while designating something
# else. The format is `path::symbol`
# (`crates/mika-agent/src/auto_pull.rs::PLAN_CALLOUT_RE`,
# `skills/bundled/_shared/dispatch-lib.sh::_extract_plan_path`).
#
# ─────────────────────────────────────────────────────────────────────────────
# PERIMETER, AND THE TWO EXCLUSIONS THAT ARE LOAD-BEARING
#
#   scanned:  crates/*/src/**/*.rs, scripts/*.sh, skills/bundled/**/*.sh
#   excluded: test code and comments.
#
# Test code, because a fixture body legitimately writes every shape this guard
# enumerates — `test-dispatch-lib.sh` alone carries a dozen `Grooming history`
# callouts — and counting them would make the survey a census of its own test
# suite. Comments, because a doc-comment DESCRIBING a match site is not one, and
# `grooming_marker.rs`'s module header quotes all four of its own regexes.
#
# A `.github/labels.yml` site in the TSV is OUT of this perimeter by
# construction (it is not source), so `--check` never compares those rows —
# stated here rather than discovered, because a guard that silently ignores part
# of its own list is the junk drawer one step early.
#
# Usage:
#   canonical-tokens-survey.sh              human-readable table
#   canonical-tokens-survey.sh --tsv        `token<TAB>site`, sorted, diff-ready
#   canonical-tokens-survey.sh --check      diff against the shipped TSV
#   canonical-tokens-survey.sh --check <tsv>|<root>   for the test harness
#
# Exit 0 clean, 1 on divergence (--check), 2 on a usage error.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

MODE="table"
SCAN_ROOT="$REPO_ROOT"
TSV_FILE=""

case "${1:-}" in
    "") ;;
    --tsv) MODE="tsv" ;;
    --check) MODE="check" ;;
    *)
        echo "usage: $(basename "$0") [--tsv|--check] [scan-root]" >&2
        exit 2
        ;;
esac

# Second argument lets the anti-vacuity harness point the survey at a fixture
# tree (the mika#2103 model, as `check-a2a-timeout-literals.sh` does).
if [[ -n "${2:-}" ]]; then
    SCAN_ROOT="$2"
fi
TSV_FILE="$SCAN_ROOT/scripts/canonical-tokens.tsv"

# ── The four token families this survey recognizes.
#
# Deliberately NOT a list of known token spellings: a survey that looks for the
# tokens it already knows cannot discover the site that forgot one. These are
# SHAPES.
#
#   callout        a blockquote-list callout key:  `> - **Something:**`
#   verdict_token  an ALL-CAPS pipeline token:     GROOMED, ESCALATE, PR_OPENED
#   marker_key     an HTML-comment marker key:     rescue-pipeline-verified
#   webhook_prefix a gateway message prefix:       `[GitHub] …`, `[callback…`
#
# `long_verdict_token` is >= 5 chars on purpose: 4 would sweep up `JSON`, `HTTP`,
# `NULL` and every SQL keyword in a query string, and an inventory drowned in
# those is an inventory nobody reads — R4 again, one layer down.

# ── Survey one file, in ONE awk pass.
#
# One pass, not three, and that is a correctness property before it is a speed
# one: the symbol a line belongs to is maintained as the file is read, so it can
# never disagree with the line that produced it. The first draft re-scanned the
# file from the top for every token found, which was both quadratic (a
# 7 000-line `dispatch-lib.sh` never finished) and a second reader of the same
# question — the shape mika#2158 forbids one file away.
#
# What the pass does, in order, per line:
#
#   1. LIVENESS. Rust `#[cfg(test)]` blocks are skipped with brace-depth
#      tracking (so a mid-file test module does not blind the rest — the shape
#      `check-a2a-timeout-literals.sh` already uses), and comment lines are
#      dropped. A doc-comment DESCRIBING a match site is not one, and
#      `grooming_marker.rs`'s header quotes all four of its own regexes.
#   2. SYMBOL. `static`/`const`/`fn` (Rust), `name()`/`function name` (shell).
#      A line with no declaration above it resolves to `<file-scope>` rather
#      than being dropped: a top-level `grep` in a shell script is a real match
#      site, and losing it silently is the incompleteness this script ends.
#   3. STRICT FORM. The five the repository actually employs. `contains` is NOT
#      among them: it is the loose form (`executor`'s `docs/plans/` substring
#      test), and including it would put the tolerant half of a deliberately
#      asymmetric pair into the strict inventory — the asymmetry
#      `grooming_marker.rs` documents as a decision.
#   4. TOKENS. The four shapes above, in order, longest-shape first.
survey_file() {
    local abs="$1" kind="$2" rel="$3"
    awk -v kind="$kind" -v rel="$rel" '
        # Split a line into its string literals. Handles the three quoting
        # forms the tree uses: `"…"`, `'…'` and Rust raw strings (`r"…"`,
        # `r#"…"#`). Deliberately NOT a parser: it is a scanner that errs
        # towards returning MORE text than a strict parse would (an unbalanced
        # quote yields the rest of the line), because the failure direction that
        # matters here is missing a site, not carrying one extra candidate.
        function literals(line, out,   i, c, n, q, buf, len, hashes) {
            n = 0; len = length(line); i = 1
            while (i <= len) {
                c = substr(line, i, 1)
                if (c == "\"" || c == "\47") {
                    q = c; buf = ""; i++
                    while (i <= len) {
                        c = substr(line, i, 1)
                        if (c == "\\" && q == "\"") { buf = buf substr(line, i + 1, 1); i += 2; continue }
                        if (c == q) { i++; break }
                        buf = buf c; i++
                    }
                    n++; out[n] = buf
                    continue
                }
                if (c == "r" && substr(line, i + 1, 1) ~ /[#"]/) {
                    hashes = 0; i++
                    while (substr(line, i, 1) == "#") { hashes++; i++ }
                    if (substr(line, i, 1) == "\"") {
                        i++; buf = ""
                        while (i <= len) {
                            if (substr(line, i, 1) == "\"") {
                                if (hashes == 0 || substr(line, i + 1, hashes) == substr("##########", 1, hashes)) {
                                    i += 1 + hashes; break
                                }
                            }
                            buf = buf substr(line, i, 1); i++
                        }
                        n++; out[n] = buf
                        continue
                    }
                }
                i++
            }
            return n
        }

        # The argument of a Rust call, bounded at its matching close paren.
        #
        # Bounding is load-bearing, not tidiness: without it,
        # `Regex::new(p).expect("SECRET_PATTERNS contains invalid regex")`
        # reports a panic message as a token. The regex there is a VARIABLE;
        # the only literal on the line belongs to `.expect`, which reads
        # nothing. An unbalanced line yields the rest of it — the safe
        # direction, since a site carried twice is cheaper than a site lost.
        function arg_of(rest,   i, c, depth, len, q) {
            len = length(rest); depth = 1; i = 1
            while (i <= len) {
                c = substr(rest, i, 1)
                if (c == "\"" || c == "\47") {
                    q = c; i++
                    while (i <= len) {
                        if (substr(rest, i, 1) == "\\" && q == "\"") { i += 2; continue }
                        if (substr(rest, i, 1) == q) break
                        i++
                    }
                    i++
                    continue
                }
                if (c == "(") depth++
                else if (c == ")") { depth--; if (depth == 0) return substr(rest, 1, i - 1) }
                i++
            }
            return rest
        }

        function emit(tok,   t) {
            t = tok
            gsub(/\\/, "", t)
            sub(/[[:space:]]+$/, "", t)
            if (t == "") return
            # An environment-variable name is a process identifier, never a
            # token somebody writes into a ticket body. `MIKA_` is not even a
            # token, it is the scrubbing prefix.
            if (t ~ /^(MIKA_|GH_|GITHUB_|TEST_)/) return
            print t "\t" rel "::" (sym == "" ? "<file-scope>" : sym) "\t" form
        }

        # ── 1. liveness
        kind == "rust" && skipping {
            o = gsub(/\{/, "{"); c = gsub(/\}/, "}")
            if (o > 0) started = 1
            depth += o - c
            if (started && depth <= 0) skipping = 0
            else if (!started && index($0, ";") > 0) skipping = 0
            next
        }
        kind == "rust" && /^[[:space:]]*#\[cfg\(test\)\]/ { skipping = 1; depth = 0; started = 0; next }
        kind == "rust" && /^[[:space:]]*\/\// { next }
        kind == "shell" && /^[[:space:]]*#/ { next }

        # ── 2. symbol
        {
            if (kind == "rust") {
                s = $0; sub(/^[[:space:]]*/, "", s)
                sub(/^pub(\([^)]*\))?[[:space:]]+/, "", s)
                if (match(s, /^(static|const)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
                    sub(/^(static|const)[[:space:]]+/, "", s)
                    match(s, /^[A-Za-z_][A-Za-z0-9_]*/); sym = substr(s, 1, RLENGTH)
                } else {
                    sub(/^async[[:space:]]+/, "", s)
                    if (match(s, /^fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
                        sub(/^fn[[:space:]]+/, "", s)
                        match(s, /^[A-Za-z_][A-Za-z0-9_]*/); sym = substr(s, 1, RLENGTH)
                    }
                }
            } else {
                if (match($0, /^[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(\)/)) {
                    match($0, /^[A-Za-z_][A-Za-z0-9_]*/); sym = substr($0, 1, RLENGTH)
                } else if (match($0, /^function[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
                    s = $0; sub(/^function[[:space:]]+/, "", s)
                    match(s, /^[A-Za-z_][A-Za-z0-9_]*/); sym = substr(s, 1, RLENGTH)
                }
            }
        }

        # ── 3. strict form, and the SLICE it consumes.
        #
        # `consumed` is the text AFTER the form marker, and tokens are read from
        # that slice only — never from the whole line. Measured reason: a line
        # can carry a literal the form never reads, and
        # `secret_scrubber.rs::INDIVIDUAL_REGEXES` is the case in the tree —
        # `Regex::new(p).expect("SECRET_PATTERNS contains invalid regex")`, where
        # the regex is a variable and the only literal is a panic message. A
        # panic message is not a token anybody writes into a ticket.
        #
        # THE `const` FORM. A token single-sourced in a constant, read through
        # its name, is the BEST-written shape in the tree
        # (`READY_LABEL_DISPATCH_MARKER`, `PIPELINE_VERIFIED_KEY`), and a survey
        # reading only inline literals is blind to exactly those. The site is
        # then the constant — which is also the right name for it, since the
        # several readers of one constant are one site, not several.
        # Restricted to constants whose NAME declares a protocol token
        # (`…_MARKER`, `…_PREFIX`, `…_KEY`, `…_LINE`, `CANCEL_REASON_…`): the
        # tree writes ~40 other `&str` constants — SQL statements, `env!()`
        # passthroughs, a PEM fixture, tool-description prose — whose ALL-CAPS
        # runs are not tokens anybody writes into a ticket. The naming
        # convention is the author DECLARING his intent, which makes this
        # structural rather than a denylist. Named limit: a protocol constant
        # named otherwise escapes, and is then misnamed, so the remedy is to
        # rename it.
        #
        # `.contains(` is DELIBERATELY ABSENT, and the exclusion was measured
        # rather than reasoned. Admitting it adds sixteen rows, of which two are
        # real (`check_grooming_markers`) and fourteen are calibration
        # assertions checking that a model answer carries `VERDICT:`, plus
        # `FOREIGN KEY`, `PATCH`, `SKIPPED` and `ANALYSIS`. A survey drowned in
        # that is the junk drawer of § R4 one step early. The two real sites are
        # not lost: the RUST half of the guard starts from the TOKEN instead of
        # the form, so it sees a `contains` reader this one cannot. That
        # complementarity is the design.
        #
        # awk note: no comment may sit between a closing brace and its `else`,
        # so the chain below carries none. Reasons live here.
        {
            form = ""
            if ((p = index($0, "Regex::new(")) > 0)         { form = "regex";        consumed = arg_of(substr($0, p + 11)) }
            else if ((p = index($0, ".starts_with(")) > 0)  { form = "starts_with";  consumed = arg_of(substr($0, p + 13)) }
            else if ((p = index($0, ".strip_prefix(")) > 0) { form = "strip_prefix"; consumed = arg_of(substr($0, p + 14)) }
            else if ((p = index($0, ".match_indices(")) > 0){ form = "match_indices"; consumed = arg_of(substr($0, p + 15)) }
            else if (kind == "rust" && match($0, /(const|static)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*(_MARKER|_PREFIX|_KEY|_LINE|_TOKEN)[[:space:]]*:[[:space:]]*&[^=]*=/)) {
                form = "const"; consumed = substr($0, RSTART + RLENGTH)
            }
            else if (kind == "rust" && match($0, /(const|static)[[:space:]]+CANCEL_REASON_[A-Za-z0-9_]*[[:space:]]*:[[:space:]]*&[^=]*=/)) {
                form = "const"; consumed = substr($0, RSTART + RLENGTH)
            }
            else if (match($0, /(^|[^A-Za-z_-])grep([[:space:]]|$)/)) { form = "grep"; consumed = substr($0, RSTART + RLENGTH) }
            else if (match($0, /sed[[:space:]]+-n[[:space:]]/))       { form = "sed";  consumed = substr($0, RSTART + RLENGTH) }
            if (form == "") next
        }

        # ── 4. tokens, extracted from the line`s STRING LITERALS ONLY.
        #
        # The discriminator is load-bearing and was found by measurement, not by
        # reasoning: the first draft read the whole line and returned 236 rows,
        # of which some 180 were `$SHELL_VARS` and the names of the `static`s
        # being declared (`DEPTH_RE` yielding a token `DEPTH`). A token is
        # something a HUMAN OR A MODEL WRITES into a ticket body, a PR body or a
        # callback envelope — never an identifier the compiler resolves. Living
        # inside a literal is the structural form of that distinction, and a
        # `$`-expansion inside a literal is still an identifier.
        #
        # This is R4 applied to the survey rather than to the lint: an inventory
        # drowned in its own noise becomes the junk drawer one step early, and
        # its `--check` would redden on every new `X_FILE=` in any script.
        {
            n = literals(consumed, lit)
            for (i = 1; i <= n; i++) {
                s = lit[i]

                # callout key — `> - **Name:**`, possibly regex-escaped.
                t = s
                while (match(t, /> - \\?\*\\?\*[A-Za-z][A-Za-z ]*:\\?\*\\?\*/)) {
                    emit(substr(t, RSTART, RLENGTH))
                    t = substr(t, RSTART + RLENGTH)
                }

                # webhook / callback prefix — `[GitHub] …`, `[callback…`.
                t = s
                while (match(t, /\[(GitHub|callback)[^\]]*\]?/)) {
                    emit(substr(t, RSTART, RLENGTH))
                    t = substr(t, RSTART + RLENGTH)
                }

                # HTML-comment marker key — a kebab key used as a marker.
                t = s
                while (match(t, /[a-z][a-z0-9]*(-[a-z0-9]+)+/)) {
                    tok = substr(t, RSTART, RLENGTH)
                    if (tok ~ /(verified|pending|marker)/) emit(tok)
                    t = substr(t, RSTART + RLENGTH)
                }

                # ALL-CAPS pipeline token, >= 5 chars (see the note above).
                #
                # Space-separated runs are joined. Tokens in this tree include
                # `PIPELINE FAILURE`, `STRUCTURAL VIOLATION`, `HANDLER CRASH`
                # and `DECISION NEEDED`, and splitting them would report the
                # halves of a token instead of the token.
                t = s
                while (match(t, /[A-Z][A-Z_]{4,}([ -][A-Z][A-Z_]+)*/)) {
                    tok = substr(t, RSTART, RLENGTH)
                    # `$VAR`, `${VAR}`, `$_VAR`, `${_VAR}` are expansions, not
                    # text. Walk back over leading underscores first — half the
                    # shell variables in `dispatch-lib.sh` are `_`-prefixed, and
                    # a check on the single preceding character misses all of
                    # them.
                    j = RSTART - 1
                    while (j >= 1 && substr(t, j, 1) == "_") j--
                    pre = (j >= 1) ? substr(t, j, 1) : ""
                    pre2 = (j >= 2) ? substr(t, j - 1, 1) : ""
                    if (pre != "$" && !(pre == "{" && pre2 == "$")) emit(tok)
                    t = substr(t, RSTART + RLENGTH)
                }
            }
        }
    ' "$abs"
}

# ── Files OUT of the perimeter: source scanners.
#
# A source scanner reads CODE, never a written text. The ALL-CAPS strings it
# carries are the names of the identifiers it hunts (`DYNAMIC_SANCTIONED`,
# `PILOT_SANDBOX_SECRET_ALLOWLIST`, `SECRET_PATTERNS`), and asking whether those
# are "canonical" is the question `check-a2a-timeout-literals.sh` § M1 already
# had to refuse: it means nothing, so the guard gets disarmed or its list
# becomes the drawer the real regression hides in.
#
# This is a PERIMETER, not an allowlist, and the distinction matters for the
# mika#2201 § D5/D6 doctrine ("declare, never allowlist"): that doctrine governs
# match SITES inside the perimeter. Declaring the perimeter is what
# `check-a2a-timeout-literals.sh` does with its four paths.
#
# `check-pr-body-consistency.sh` is deliberately NOT here: it reads a PR BODY,
# which is written text, and its `Closes #N` / `Tracked in:` readers are a class
# A row of the canonical list.
#
# Compared BOTH WAYS below: an entry naming no file fails, exactly like an
# undeclared site.
SOURCE_SCANNERS=(
    "scripts/check-a2a-timeout-literals.sh"
    "scripts/check-byte-slices.sh"
    "scripts/check-dispatch-seats-declared.sh"
    "scripts/check-loop-select.sh"
    "scripts/check-secrets.sh"
    "scripts/check-shared-checkout-guard-wiring.sh"
    "scripts/verify-no-secret-in-setenv.sh"
    "scripts/verify-no-sigpipe-grep.sh"
    "scripts/canonical-tokens-survey.sh"
    "scripts/check-canonical-tokens.sh"
)

is_source_scanner() {
    local rel="$1" entry
    for entry in "${SOURCE_SCANNERS[@]}"; do
        [[ "$rel" == "$entry" ]] && return 0
    done
    return 1
}

# ── Collect every candidate file.
#
# Test code is excluded: a fixture body legitimately writes every shape this
# survey enumerates (`test-dispatch-lib.sh` alone carries a dozen `Grooming
# history` callouts), and counting them would make the inventory a census of
# its own test suite. Both naming conventions in the tree are covered —
# `test-*.sh` under `scripts/`, `test_*.sh` and `*/tests/*` under `skills/`.
collect_files() {
    local found=0 rel
    if [[ -d "$SCAN_ROOT/crates" ]]; then
        found=1
        find "$SCAN_ROOT/crates" -path '*/src/*' -name '*.rs' -type f \
            -not -path '*/tests/*' -not -name 'tests.rs' \
            | sed "s|^$SCAN_ROOT/||" | sort
    fi
    if [[ -d "$SCAN_ROOT/scripts" ]]; then
        found=1
        find "$SCAN_ROOT/scripts" -maxdepth 1 -name '*.sh' -type f \
            -not -name 'test-*' -not -name 'test_*' | sed "s|^$SCAN_ROOT/||" | sort
    fi
    if [[ -d "$SCAN_ROOT/skills/bundled" ]]; then
        found=1
        find "$SCAN_ROOT/skills/bundled" -name '*.sh' -type f \
            -not -path '*/tests/*' -not -name 'test-*' -not -name 'test_*' \
            | sed "s|^$SCAN_ROOT/||" | sort
    fi
    [[ $found -eq 0 ]] && { echo "__NO_PERIMETER__"; return 0; }
    return 0
}

filter_perimeter() {
    local rel
    while IFS= read -r rel; do
        [[ -n "$rel" ]] || continue
        if [[ "$rel" == "__NO_PERIMETER__" ]]; then echo "$rel"; continue; fi
        is_source_scanner "$rel" && continue
        echo "$rel"
    done
}

survey() {
    local rel kind
    while IFS= read -r rel; do
        [[ -n "$rel" ]] || continue
        [[ "$rel" == "__NO_PERIMETER__" ]] && { echo "$rel"; return 0; }
        case "$rel" in
            *.rs) kind="rust" ;;
            *)    kind="shell" ;;
        esac
        survey_file "$SCAN_ROOT/$rel" "$kind" "$rel"
    done < <(collect_files | filter_perimeter)
}

# A `SOURCE_SCANNERS` entry naming no file is stale — the same two-way rule the
# match-site comparison obeys, and for the same reason: an exclusion that
# outlives its file is how a perimeter becomes a drawer.
check_perimeter_entries() {
    local entry rc=0
    for entry in "${SOURCE_SCANNERS[@]}"; do
        if [[ ! -f "$SCAN_ROOT/$entry" ]]; then
            echo "ERROR: SOURCE_SCANNERS entry names no file: $entry" >&2
            rc=1
        fi
    done
    return $rc
}

RAW="$(survey)"

if [[ "$RAW" == "__NO_PERIMETER__" ]]; then
    echo "ERROR: no perimeter path exists under '$SCAN_ROOT'." >&2
    echo "       A survey with nothing to survey is a vacuous pass, not a clean one." >&2
    exit 1
fi

# `token<TAB>site`, deduplicated — one row per (token, site) pair, whatever the
# number of lines inside a symbol that carry it.
PAIRS="$(printf '%s\n' "$RAW" | cut -f1,2 | sort -u)"

case "$MODE" in
    table)
        printf '%-34s  %-72s  %s\n' "TOKEN" "SITE" "FORM"
        printf '%s\n' "$RAW" | sort -u | while IFS=$'\t' read -r tok site form; do
            printf '%-34s  %-72s  %s\n' "$tok" "$site" "$form"
        done
        ;;
    tsv)
        printf '%s\n' "$PAIRS"
        ;;
    check)
        if [[ ! -f "$TSV_FILE" ]]; then
            echo "ERROR: canonical list not found at $TSV_FILE" >&2
            exit 1
        fi
        # ONE direction, and the asymmetry is deliberate.
        #
        # MISSING — every site this survey finds must be declared, whatever its
        # class. That is what closes Prime's bound: a strict reader nobody wrote
        # down fails the build.
        #
        # STALE is NOT checked here, and this is the one place a reader is
        # likely to think something is missing. The obvious two-way comparison —
        # the shape `check-dispatch-seats-declared.sh` uses, and for a good
        # reason there — cannot work on this list, because the TSV legitimately
        # carries rows this survey structurally cannot produce: a `(?i)`
        # alternation, a fuzzy tier, a `contains` reader. Comparing that way
        # would fail the build on CORRECT rows, and the natural repair would be
        # to widen the survey until it caught them, which is measured at
        # fourteen false rows against two real ones.
        #
        # The stale direction is held by the OTHER half instead, and held
        # better: `canonical_tokens::tests::mika2201_every_declared_symbol_still_exists`
        # checks that every declared `chemin::symbole` still names something,
        # which is the question "is this row stale?" asked directly rather than
        # inferred from a coincidence with a survey. Residual rot it does not
        # catch, named rather than hidden: a row whose file and symbol both
        # still exist but which no longer reads the token.
        ALL_DECLARED="$(awk -F'\t' '
            /^#/ || NF < 3 { next }
            $3 ~ /^(crates\/|scripts\/|skills\/)/ { printf "%s\t%s\n", $1, $3 }
        ' "$TSV_FILE" | sort -u)"

        MISSING="$(comm -23 <(printf '%s\n' "$PAIRS") <(printf '%s\n' "$ALL_DECLARED") || true)"
        STALE=""

        rc=0
        if [[ -n "$MISSING" ]]; then
            echo "ERROR: match sites found in the tree and absent from $TSV_FILE:"
            printf '%s\n' "$MISSING" | sed 's/^/       /'
            echo "       Declare them in the TSV. Do NOT allowlist them — a match site"
            echo "       you do not want to declare is a match site to delete (mika#2201 § D5/D6)."
            rc=1
        fi
        # `$STALE` is always empty here — see the comment above the comparison.
        # The variable is kept so the reason stays attached to the code rather
        # than living only in a commit message.
        if [[ -n "$STALE" ]]; then
            echo "ERROR: rows in $TSV_FILE matching no site in the tree:"
            printf '%s\n' "$STALE" | sed 's/^/       /'
            rc=1
        fi
        if [[ $rc -eq 0 ]]; then
            echo "Canonical token list agrees with the tree ($(printf '%s\n' "$PAIRS" | grep -c . || true) match sites)."
        fi
        exit $rc
        ;;
esac
