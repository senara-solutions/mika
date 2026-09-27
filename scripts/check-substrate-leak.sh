#!/usr/bin/env bash
# CI lint: substrate-configuration detail must not reach the LLM (mika#1964).
#
# THE PROPERTY, not a list of constructors:
#   A tool result's `content` is read by the model, and on a family/champion
#   tenant it is the ONLY thing the model sees. A `content` naming an env var, a
#   config path, a service, or an operator instruction hands the sealed being
#   infrastructure it must never carry — and, measured on 2026-09-18 across six
#   tenants (mika#2407), the being then paraphrases it to its user as a wrong,
#   unactionable claim.
#
#   The remedy is `dispatch_substrate_unavailable(fallback, diagnostic, …)`:
#   the neutral half goes to `content`, the operator-shaped half goes to the
#   substrate telemetry sink. See mika#1783 (the founding fix) and
#   `crates/mika-agent/src/tools/mod.rs`.
#
# RULE 2 IS ANCHORED ON THE LITERAL, NEVER ON THE CONSTRUCTOR.
#   The founding defect of mika#1964 is `map_substrate_error`: the leaking
#   literal lives in a function of its own and appears on no line carrying
#   `ToolOutput::error`. A rule keyed on the constructor — which is what the
#   ticket's own acceptance criterion proposes — would not see it. It would also
#   miss `ToolOutput::success` (`resolve_issue_order`) and `ToolOutput::delivery`
#   (`send_message`), which serve the model just as much: three constructors in
#   one measured population. This is the mika#2103 lesson applied before the
#   incident rather than after it — a guard that knows one *writing* of a defect
#   lets every other writing through.
#
# WHEN YOU EXTEND THIS SCRIPT, EXTEND IT BY PROPERTY.
#   If you find a new shape of substrate detail reaching a model-visible string,
#   it belongs in SUBSTRATE_PATTERNS. Do not wait for a tenant to read it out.
#
# TWO ANNOTATIONS, TWO POPULATIONS, COUNTABLE APART:
#   `// substrate-diagnostic: <reason>` — this literal IS the operator channel.
#       It must name the surface; `dispatch_substrate_diagnostic` routes it.
#   `// substrate-ok: <reason>`         — this literal never reaches the
#       `content` served to the LLM (an `env()` argument, a config key, a flag
#       name, a `tracing::warn!` field).
#   One shared prefix would fuse "protected by the mechanism" with "not in the
#   subject" and make the first population uncountable the day it deserves an
#   audit. The reason is mandatory in spirit — it is what a future reader needs
#   in order to re-audit the site.
#
# AN ANNOTATION COVERS A WINDOW, AND THAT IS A CONSTRAINT OF THE LANGUAGE:
#   The offending literals are multi-line `format!`s whose continuation lines end
#   in `\`, so writing `//` inside one would put the comment INSIDE the string.
#   An annotation therefore exempts the literal that FOLLOWS it — up to the first
#   quoted line not ending in `\` — with ANNOTATION_WINDOW lines as a cap, never
#   as the extent: an unannotated arm right below stays in the population. Prefer
#   hoisting the text into a named constant and annotating its declaration (the
#   `GWS_CREDENTIALS_ABSENT_DIAGNOSTIC` shape) — that keeps the window short by
#   construction. Annotations govern rule 2 only; rule 1 has no exemption.
#
# Exit 0 clean, 1 violations found, 2 the guard could not look where it believes
# it looks (see GOOD-FAITH below) — a scan that lost its target must go red, not
# green on the empty set.

set -euo pipefail

VIOLATIONS=0
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Scan root. Defaults to the agent crate's source; an explicit argument lets the
# anti-vacuity harness point the guard at a fixture tree (mika#2103 pattern).
SCAN_ROOT="${1:-$REPO_ROOT/crates/mika-agent/src}"

# How many lines after an annotation stay exempt. Short on purpose: an annotation
# that covered a literal thirty lines down would be a silent hole in the guard.
ANNOTATION_WINDOW=12

# ── The perimeter.
#
# `builtin_handlers.rs` + `tools/*.rs` — the two paths mika#1964 declares. NOT
# `mika-gateway`, `mika-cli`, or `skills/bundled/**`, where a variable name in a
# string is nominal; extending there means first measuring the annotation volume.
PERIMETER=()
[[ -f "$SCAN_ROOT/skills/builtin_handlers.rs" ]] && PERIMETER+=("$SCAN_ROOT/skills/builtin_handlers.rs")
while IFS= read -r f; do
    [[ -n "$f" ]] && PERIMETER+=("$f")
done < <(find "$SCAN_ROOT/tools" -maxdepth 1 -name '*.rs' -type f 2>/dev/null | sort)

if [[ ${#PERIMETER[@]} -eq 0 ]]; then
    echo "ERROR: no file in the perimeter under $SCAN_ROOT"
    echo "Expected skills/builtin_handlers.rs and/or tools/*.rs."
    echo "A guard with an empty population reads exactly like a clean tree — see mika#2205."
    exit 2
fi

# ── GOOD-FAITH markers: has the production slice kept its target?
#
# The test-region cut is on the test MODULE, never on the first `#[cfg(test)]`
# (see the awk comment below for why). Should a future edit move the module
# marker, the cut could silently shrink the production slice — and a scan that
# reads a truncated file reports "clean" for the wrong reason. So a file with a
# declared marker must still contain it AFTER the cut, or the guard exits 2.
good_faith_marker_for() {
    case "$(basename "$1")" in
        builtin_handlers.rs) echo 'async fn run_gws(' ;;
        *) echo '' ;;
    esac
}

for file in "${PERIMETER[@]}"; do
    marker="$(good_faith_marker_for "$file")"
    output="$(awk \
        -v window="$ANNOTATION_WINDOW" \
        -v marker="$marker" \
        -v path="$file" \
        '
        # ── Pass 1: buffer the file so the test-region cut can be decided before
        # any line is judged.
        { line[NR] = $0 }

        END {
            total = NR

            # ── The test-region cut.
            #
            # Split on the test MODULE, not on the first `#[cfg(test)]`:
            # builtin_handlers.rs carries a `#[cfg(not(test))]` / `#[cfg(test)]`
            # pair on PROGRESS_TICKER_INTERVAL around line 600, while the real
            # test module starts near 5140. The naive cut leaves ~4500 lines of
            # production code unscanned while the guard exits 0 and reads as a
            # clean perimeter — green for the wrong reason. The same trap is
            # already written down in
            # `mika2118_probe_runs_only_on_auth_error`, whose remedy this
            # reproduces verbatim, good-faith assertion included.
            #
            # Test regions are excluded because the tests ASSERT on these very
            # tokens (FORBIDDEN_FAMILY_TIER_TOKENS contains MIKA_BRAVE_API_KEY
            # and config.toml literally). That is the difference from
            # check-byte-slices.sh, which scans crates/ whole because a UTF-8
            # panic is a defect in a test too.
            cut = total + 1
            for (i = 2; i <= total; i++) {
                if (line[i] == "mod tests {" && line[i - 1] == "#[cfg(test)]") {
                    cut = i - 1
                    break
                }
            }

            if (marker != "") {
                found = 0
                for (i = 1; i < cut; i++) {
                    if (index(line[i], marker) > 0) { found = 1; break }
                }
                if (!found) {
                    printf "GUARD-BROKEN\t%s\t%s\n", path, marker
                    exit
                }
            }

            # ── Pass 2: judge the production slice.
            #
            # ORDER IS THE CONTRACT (mika#1964 review):
            #   1. comment lines leave the population FIRST — so a comment such
            #      as `// mirrors fn dispatch_substrate_unavailable` can neither
            #      set fn_name (which would silence rule 1 below it) nor be
            #      judged; a comment that is an annotation arms the window;
            #   2. fn_name is tracked on CODE lines only;
            #   3. rule 1 runs BEFORE any window skip — annotations govern
            #      rule 2 alone. An annotated const followed by a bare
            #      constructor must still fire rule 1: "no allowlist" means
            #      no annotation either;
            #   4. only then does the window exempt the line from rule 2.
            win_open = 0
            win_end = 0
            win_seen_lit = 0
            fn_name = "(top level)"

            for (i = 1; i < cut; i++) {
                text = line[i]
                stripped = text
                sub(/^[ \t]+/, "", stripped)

                # Comment lines are out of the population: documenting an env var
                # is house style (this repo is made of it), and a comment never
                # reaches the `content` served to the LLM. The cost on the subject
                # is nil; the benefit is that the guard is not born red, which is
                # what keeps it armed. Covers `///` and `//!` by construction.
                # A comment line carrying an annotation arms the window.
                if (substr(stripped, 1, 2) == "//") {
                    if (is_annotation(text)) arm_window(i)
                    continue
                }

                # Track the enclosing top-level function, for rule 1 only —
                # on code lines, never on comments (see ORDER above).
                if (match(text, /(^|[^A-Za-z0-9_])fn[ \t]+[A-Za-z0-9_]+/)) {
                    frag = substr(text, RSTART, RLENGTH)
                    sub(/^.*fn[ \t]+/, "", frag)
                    fn_name = frag
                }

                # ── Rule 1: the bare constructor has a single site.
                #
                # `ToolOutput::substrate_unavailable` stays `pub` (tests build
                # it), but exactly one production site may call it: the one
                # inside `dispatch_substrate_unavailable`, which constructs AND
                # routes in one expression so the coupled pair cannot be written
                # apart. No allowlist: when this fires, remove the second site.
                # Evaluated before any annotation window — no annotation can
                # silence it, not even one trailing on the same line.
                if (index(text, "ToolOutput::substrate_unavailable(") > 0 && fn_name != "dispatch_substrate_unavailable") {
                    printf "VIOLATION\tRule 1: bare substrate_unavailable outside dispatch_substrate_unavailable (fn %s)\t%s:%d\t%s\n", fn_name, path, i, stripped
                }

                # A trailing annotation (`foo(); // substrate-ok: …`) arms the
                # window on its own line; that line is judged on its code part.
                code = text
                if (is_annotation(text)) {
                    arm_window(i)
                    code = substr(text, 1, annotation_at(text) - 1)
                }

                # ── The window, and where it ENDS.
                #
                # An annotation exempts the literal that follows it — not a
                # fixed count of lines. The window stays open over code lines
                # until the first quoted line that does not end in `\` (the
                # last line of the annotated string), and ANNOTATION_WINDOW is only
                # an upper cap. A fixed count exempted the adjacent match arm
                # too: at builtin_handlers.rs the `unauthorized` diagnostic
                # annotation covered the model-visible `transport_error` arm
                # beneath it (green-while-red, mika#1964 review).
                if (win_open) {
                    if (i > win_end) {
                        win_open = 0
                    } else {
                        if (index(code, "\"") > 0) win_seen_lit = 1
                        tail = code
                        sub(/[ \t\r]+$/, "", tail)
                        if (win_seen_lit && substr(tail, length(tail), 1) != "\\") win_open = 0
                        continue
                    }
                }

                # ── Rule 2: no substrate literal in a model-visible string.
                #
                # By PROPERTY: "names an env var", not "matches this spelling".
                # `MIKA_[A-Z0-9_]+` so MIKA_A2A_* (defined in crates/) matches;
                # the second pattern catches provider credentials
                # (OPENAI_API_KEY, ANTHROPIC_API_KEY, AWS_SECRET_ACCESS_KEY, …).
                # Space sentinels on both ends stand in for `^`/`$` so the
                # boundary classes work identically under gawk and mawk. The
                # left boundary also refuses `:` so a Rust path to a constant
                # (`CiAbstention::NO_TOKEN`) is not read as an env var: the
                # property is a NAME an operator sets, not an identifier.
                if (match(text, /MIKA_[A-Z0-9_]+/))       report("MIKA_* env var", path, i, stripped)
                else if (match(text, /GH_TOKEN|GITHUB_TOKEN/)) report("credential env var", path, i, stripped)
                else if (match(" " text " ", /[^A-Za-z0-9_:][A-Z][A-Z0-9_]*_(API_KEY|SECRET|SECRET_KEY|ACCESS_KEY|TOKEN)[^A-Za-z0-9_]/)) report("credential env var", path, i, stripped)
                else if (index(text, "XDG_CONFIG_HOME") > 0)   report("third-party env var", path, i, stripped)
                else if (index(text, "config.toml") > 0)       report("config path", path, i, stripped)
                else if (index(text, ".mika/") > 0)            report("runtime root path", path, i, stripped)
                else if (index(text, "Ask the operator") > 0)  report("operator instruction", path, i, stripped)
                else if (index(text, "GitHub App") > 0)        report("named operator surface", path, i, stripped)
                else if (index(text, "GitHub token") > 0)      report("named credential", path, i, stripped)
            }
        }

        function report(label, p, ln, txt) {
            printf "VIOLATION\tRule 2: substrate literal (%s)\t%s:%d\t%s\n", label, p, ln, txt
        }

        # Position of the annotation marker in s, 0 when there is none.
        function annotation_at(s,    a, b) {
            a = index(s, "// substrate-ok:")
            b = index(s, "// substrate-diagnostic:")
            if (a == 0) return b
            if (b == 0) return a
            return (a < b) ? a : b
        }

        function is_annotation(s) {
            return annotation_at(s) > 0
        }

        # Open (or re-open) the window at line n; ANNOTATION_WINDOW is the cap.
        function arm_window(n) {
            win_open = 1
            win_end = n + window
            win_seen_lit = 0
        }
        ' "$file")"

    while IFS=$'\t' read -r kind a b c; do
        [[ -z "$kind" ]] && continue
        if [[ "$kind" == "GUARD-BROKEN" ]]; then
            echo "GUARD BROKEN: production slice of $a no longer contains its good-faith marker:"
            echo "  expected to find: $b"
            echo ""
            echo "The test-region cut may have shrunk the slice, so a clean exit would mean"
            echo "nothing (mika#2205). Fix the cut or update good_faith_marker_for() — do NOT"
            echo "silence this by removing the marker."
            exit 2
        fi
        echo "ERROR: $a at $b"
        echo "       $c"
        VIOLATIONS=$((VIOLATIONS + 1))
    done <<< "$output"
done

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS substrate-leak violation(s) in ${#PERIMETER[@]} scanned file(s)."
    echo "A tool-result 'content' is what a sealed being reads — see mika#1783 and mika#2407."
    echo ""
    echo "Route the operator-shaped half through the coupled helper:"
    echo "  crate::tools::dispatch_substrate_unavailable(neutral_fallback, diagnostic, tool, ctx).await"
    echo ""
    echo "Or, if this literal never reaches the model, annotate it:"
    echo "  // substrate-ok: <why this string cannot reach a tool-result content>"
    echo "  // substrate-diagnostic: <why this string IS the operator channel>"
    exit 1
fi

echo "No substrate leaks found (${#PERIMETER[@]} files scanned)."
exit 0
