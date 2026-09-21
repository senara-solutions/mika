#!/usr/bin/env bash
# CI lint: a machine token whose reader is STRICT is written canonically, or the
# machine does not see it (mika#2201).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE TWO BITES THIS EXISTS FOR, AND WHAT THEY ARE NOT
#
#   * mika#1772 — `seconde passe` written in a callout while `is_groomed`
#     expected `second-pass`. Days of invisibility.
#   * mika#2188 — `ESCALATE` matched as a SUBSTRING of `ESCALATE-divergence`, so
#     a ticket whose escalation the operator had resolved read `Escalated`.
#
# Both were closed structurally (#2173, #2196). This is the belt at the gate.
#
# BUT THE FIRST ONE WAS NOT CLOSED BY TIGHTENING THE TEXT — it was closed by
# WIDENING THE READER. `grooming_marker.rs` is bilingual by written decision
# since mika#2158, and its doc-comment settles the direction of the alignment:
# *"it is the PREDICATE that aligns on the spec, not the reverse […] the spec
# does not have to impose English to be machine-readable, in a repository that
# writes its tickets and its plans in French."* Prime says the same thing:
# *"French did not bite — a textual boundary bit, and it is now structural."*
#
# So a lint that reddened on `seconde passe` would redden on a form the machine
# READS, on `grooming_marker.rs` itself, on its 24 tests, and on every French
# ticket body in the repository. It would be disarmed inside a week, and then
# the regression it exists to catch would pass in the noise — which is the
# failure mode `check-a2a-timeout-literals.sh` § M1 already had to name:
# **the failure the ticket exists to prevent, reached by the remedy.**
#
# Hence the rule, and it is NOT "canonical means English":
#
#     A FORM A STRICT READER CANNOT SEE IS REFUSED.
#     A FORM A TOLERANT READER SEES IS ADMITTED.
#
# The class and the tolerance of every token live in
# `scripts/canonical-tokens.tsv`, which is produced and confronted to the tree
# by `scripts/canonical-tokens-survey.sh --check`. This script reads that file;
# it hard-codes no token list, so widening a reader's tolerance silences the
# lint for that token automatically, in the same commit.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE FIVE RULES. EVERY ONE IS NEGATIVE, AND THAT IS DELIBERATE.
#
# None of them says "this text must contain X". A lint that demands a shape on
# free prose accuses all prose.
#
#   L1  ambiguous substring in a CALLOUT LINE. Inside a
#       `^> - **Grooming history:**` line, a verdict token flanked by a
#       compound-word character (`ESCALATE-divergence`, `GROOMED-partiel`) is
#       refused. Bounded to the callout line, exactly as `CALLOUT_LINE_RE` is:
#       the same string in prose is out of scope and stays out.
#
#   L2  case of a verdict token in a callout line. `groomed`, `Escalate`,
#       `ready` are refused there — unless the token's declared tolerance
#       carries `ci`, in which case the reader reads the case and the lint must
#       not fire. That consultation is the whole point of the tolerance column.
#
#   L3  a callout key that is a NEAR-VARIANT of a canonical one — French
#       typographic space before the colon, different case, or a listed French
#       translation. NOT "any key outside the canonical three": the milestone
#       flow legitimately writes `> - **Sub-issues:**`,
#       `> - **Sequencing record:**` and `> - **Coordination branch:**`, and a
#       rule that accused them would be red at birth.
#
#   L4  the French typographic space before the colon of a protocol label
#       (`Verdict :`, `Disposition :`, `VERDICT :`). Derived from the TSV, not
#       listed here.
#
#   L5  a label cited as a WRITE INSTRUCTION (`--add-label` / `--remove-label`)
#       in a prescriber, and absent from `.github/labels.yml`. `--label` on
#       `gh issue create` is deliberately NOT in scope: an unknown label makes
#       that call fail loudly, whereas every `--add-label` in `dispatch-lib.sh`
#       is followed by `|| true` and fails in SILENCE — which is the asymmetry
#       that motivates the rule. Fourth occurrence of the class named in
#       docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md
#
# WHAT IS NOT COVERED, stated rather than discovered: a pure translation of a
# token in a position the lint does not know (`Résultat:` for `Outcome:` inside
# a callback envelope). A lint cannot enumerate the translations of a token it
# has never seen. What it CAN do is refuse a non-canonical key where only a
# canonical one is ever read — which is L3, on the callout, where the position
# is known. Widening beyond that accuses prose.
#
# ─────────────────────────────────────────────────────────────────────────────
# SURFACES
#
#   S1  the repository's PRESCRIBERS — `.claude/commands/*.md`,
#       `skills/bundled/*/system_prompt.md`, `_shared/dispatch-lib.sh`. The
#       structural lever: a prompt prescribing a wrong form reproduces it on
#       EVERY ticket it produces. This is where `seconde passe` would have
#       become systematic again.
#   S2  a PR body, via `--body <file>` (wired into pr-body-validation.yml).
#   S3  an issue body, same flag, annotation only — GitHub offers no gate on an
#       issue, and claiming to ship one there would announce a protection that
#       does not exist.
#
# Usage:
#   check-canonical-tokens.sh [scan-root]          S1, the tree
#   check-canonical-tokens.sh --body <file> [root] S2/S3, one body
#
# Exit 0 clean, 1 with actionable errors, 2 on a usage error.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

BODY_FILE=""
SCAN_ROOT="$REPO_ROOT"

if [[ "${1:-}" == "--body" ]]; then
    BODY_FILE="${2:-}"
    if [[ -z "$BODY_FILE" ]]; then
        echo "usage: $(basename "$0") --body <file> [scan-root]" >&2
        exit 2
    fi
    SCAN_ROOT="${3:-$REPO_ROOT}"
elif [[ -n "${1:-}" ]]; then
    SCAN_ROOT="$1"
fi

TSV_FILE="$SCAN_ROOT/scripts/canonical-tokens.tsv"
EXCEPTIONS_FILE="$SCAN_ROOT/scripts/canonical-tokens-exceptions.tsv"
LABELS_FILE="$SCAN_ROOT/.github/labels.yml"

VIOLATIONS=0

fail() {
    echo "ERROR: $*"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ── The canonical callout keys, and the French forms L3 refuses.
#
# A bounded table rather than a guess: the lint cannot enumerate every possible
# translation of a key, so it refuses the ones a French-writing author actually
# reaches for. Its limit is stated in the header; what it guarantees is that the
# PROBABLE French form of the three canonical keys does not pass.
CANONICAL_CALLOUT_KEYS=("Branch" "Plan" "Grooming history")
FRENCH_CALLOUT_KEYS=("Branche" "Historique de grooming" "Historique du grooming" "Plan de travail")

if [[ ! -f "$TSV_FILE" ]]; then
    echo "ERROR: canonical list not found at $TSV_FILE" >&2
    echo "       A lint with no list checks nothing, which is worse than no lint." >&2
    exit 1
fi

# ── Read the list. Three derived sets, all from the TSV — never hard-coded.
#
#   CALLOUT_TOKENS   class B tokens read by `grooming_marker.rs`, i.e. exactly
#                    the tokens the callout reader reads. L1/L2 operate on
#                    these. `ITERATE` is deliberately absent: `VERDICT_TOKEN_RE`
#                    does not read it and the module says so in as many words,
#                    so it carries no class B row and the lint must not accuse
#                    it in a callout.
#   CI_TOKENS        tokens whose declared tolerance carries `ci`. L2 must not
#                    fire on these.
#   LABEL_TOKENS     class B tokens usable as a protocol label. L4 refuses the
#                    French typographic space on them.
CALLOUT_TOKENS=()
CI_TOKENS=()
LABEL_TOKENS=()

while IFS=$'\t' read -r tok cls site tol; do
    [[ "$tok" == \#* || -z "$tok" ]] && continue
    [[ -z "${site:-}" ]] && continue
    if [[ "$cls" == "B" && "$site" == crates/mika-agent/src/grooming_marker.rs::* ]]; then
        CALLOUT_TOKENS+=("$tok")
    fi
    if [[ "$cls" == "B" && "$tol" == ci* ]]; then
        CI_TOKENS+=("$tok")
    fi
    if [[ "$cls" == "B" && "$tok" =~ ^[A-Z][A-Z_]*$ ]]; then
        LABEL_TOKENS+=("$tok")
    fi
done < "$TSV_FILE"

# The class A labels `Disposition:` / `Verdict:` join L4's population: their
# fast tier is a strict `grep -oE`, so the French space costs a fuzzy fallback
# on every parse even though it is eventually read. Cheap to refuse, and the
# author almost certainly meant the canonical form.
LABEL_TOKENS+=("Disposition" "Verdict")

dedup() {
    printf '%s\n' "$@" | sort -u
}
mapfile -t CALLOUT_TOKENS < <(dedup "${CALLOUT_TOKENS[@]:-}")
mapfile -t CI_TOKENS < <(dedup "${CI_TOKENS[@]:-}")
mapfile -t LABEL_TOKENS < <(dedup "${LABEL_TOKENS[@]:-}")

is_ci_token() {
    local t="$1" c
    for c in "${CI_TOKENS[@]:-}"; do
        [[ "$c" == "$t" ]] && return 0
    done
    return 1
}

# ── Exceptions: four mandatory fields, and a SELF-CLEANING assertion.
#
# An exception that outlives its cause is how an allowlist becomes the drawer
# the regression hides in (§ R4). So the lint refuses an exception whose file no
# longer carries the accused token — it reddens ON THE DAY OF THE REPAIR, not
# months later. Shipped empty.
EXC_FILE=(); EXC_TOKEN=(); EXC_TICKET=()
if [[ -f "$EXCEPTIONS_FILE" ]]; then
    while IFS=$'\t' read -r efile etoken eticket edate; do
        [[ "$efile" == \#* || -z "$efile" ]] && continue
        if [[ -z "${etoken:-}" || -z "${eticket:-}" || -z "${edate:-}" ]]; then
            fail "exception row is missing one of its four mandatory fields (fichier, jeton, ticket, date): '$efile'"
            continue
        fi
        EXC_FILE+=("$efile"); EXC_TOKEN+=("$etoken"); EXC_TICKET+=("$eticket")
    done < "$EXCEPTIONS_FILE"
fi

is_excepted() {
    local f="$1" t="$2" i
    for i in "${!EXC_FILE[@]}"; do
        if [[ "${EXC_FILE[$i]}" == "$f" && "${EXC_TOKEN[$i]}" == "$t" ]]; then
            return 0
        fi
    done
    return 1
}

check_stale_exceptions() {
    local i f t
    for i in "${!EXC_FILE[@]}"; do
        f="${EXC_FILE[$i]}"; t="${EXC_TOKEN[$i]}"
        if [[ ! -f "$SCAN_ROOT/$f" ]] || ! grep -qF -- "$t" "$SCAN_ROOT/$f" 2>/dev/null; then
            fail "stale exception: '$f' no longer carries '$t' (ticket ${EXC_TICKET[$i]}) — remove the row."
            echo "       An exception that outlives its cause is how an allowlist becomes a junk drawer."
        fi
    done
}

# ── A token QUOTED as code is a mention, never an instruction.
#
# MEASURED, on this ticket's own plan: its § M4 describes rule L4 by citing the
# faulty forms — "`Verdict :`, `Disposition :`" — and the lint accused both. So
# would the body of the PR that ships it, and so would every future ticket
# discussing this rule. That is the R4 failure mode in its purest form: a
# document that explains the lint makes the lint red, and the lint gets
# disarmed. Exact precedent, one file away: mika#2050's Signal S, where a
# grooming session discussing the entry recreated its own false positive and the
# remedy was an anchor rather than a threshold.
#
# The gesture is the one `auto_pull::is_groomed` already makes for the same
# reason (mika#2120: read outside fenced blocks): strip fenced blocks and inline
# code spans before judging. The canonical callout survives it — its KEY sits
# outside the backticks, only the path inside them.
#
# An UNTERMINATED fence strips nothing, deliberately, and the direction is the
# safe one: a false positive costs one refused line, a blind lint costs the
# protection. mika#2120 rules the same way on the same trade-off.
#
# NOT applied to L5. A label written as `--add-label X` inside a prescriber IS
# an instruction, and in a markdown prescriber it necessarily lives in a fenced
# block — stripping there would blind the rule to its entire population.
# Blanked rather than deleted: the line numbers in an error message must be the
# line numbers of the file the reader will open.
strip_code_spans() {
    awk '
        /^[[:space:]]*```/ { fence = !fence; print ""; next }
        fence { print ""; next }
        { gsub(/`[^`]*`/, ""); print }
    '
}

# ── Rules L1–L4, applied line by line to one file.
lint_lines() {
    local rel="$1" abs="$2" lineno=0 line key norm canon fr tok

    while IFS= read -r line || [[ -n "$line" ]]; do
        lineno=$((lineno + 1))

        # ── A callout line: `> - **Key:**` with optional leading whitespace,
        #    the shape every reader anchors on.
        if [[ "$line" =~ ^[[:space:]]*\>\ -\ \*\*([^*]+)\*\* ]]; then
            key="${BASH_REMATCH[1]}"
            key="${key%:}"

            # ── L3 — a near-variant of a canonical key.
            norm="$(printf '%s' "$key" | tr -d ' ' | tr '[:upper:]' '[:lower:]')"
            for canon in "${CANONICAL_CALLOUT_KEYS[@]}"; do
                if [[ "$norm" == "$(printf '%s' "$canon" | tr -d ' ' | tr '[:upper:]' '[:lower:]')" \
                      && "$key" != "$canon" ]]; then
                    is_excepted "$rel" "$key" && continue
                    fail "L3: non-canonical callout key '> - **${key}:**' — $rel:$lineno"
                    echo "       The three readers are line-anchored and literal. Canonical: '> - **${canon}:**'."
                fi
            done
            for fr in "${FRENCH_CALLOUT_KEYS[@]}"; do
                if [[ "$key" == "$fr" ]]; then
                    is_excepted "$rel" "$key" && continue
                    fail "L3: translated callout key '> - **${key}:**' — $rel:$lineno"
                    echo "       Ticket prose is French; callout keys are canonical English."
                    echo "       Canonical keys: > - **Branch:** / > - **Plan:** / > - **Grooming history:**"
                fi
            done

            # ── L1 and L2 apply to the Grooming history line only, exactly as
            #    `CALLOUT_LINE_RE` does. The same string in prose is out of
            #    scope and must stay out.
            if [[ "$key" == "Grooming history" ]]; then
                for tok in "${CALLOUT_TOKENS[@]:-}"; do
                    [[ -n "$tok" ]] || continue

                    # L1 — the token glued to a compound-word character. `\b`
                    # is satisfied by a hyphen, so `ESCALATE-divergence` reads
                    # as `ESCALATE` (mika#2188). The positional doctrine covers
                    # it only when a LATER pass follows; with nothing after, the
                    # prose says "resolved" and the machine says "escalated".
                    if [[ "$line" =~ (^|[^A-Za-z0-9_])${tok}[-_][A-Za-z] ]] \
                       || [[ "$line" =~ [A-Za-z][-_]${tok}($|[^A-Za-z0-9_]) ]]; then
                        is_excepted "$rel" "$tok" && continue
                        fail "L1: ambiguous compound '$tok' in a Grooming history callout — $rel:$lineno"
                        echo "       $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
                        echo "       VERDICT_TOKEN_RE matches '$tok' INSIDE the compound: the hyphen satisfies"
                        echo "       its word boundary. Write the token alone, and put the qualifier outside"
                        echo "       the callout (mika#2188)."
                    fi

                    # L2 — the token in another case. Skipped when the reader
                    # declares `ci`: it reads the case, so accusing it would
                    # refuse a form the machine sees.
                    is_ci_token "$tok" && continue
                    if [[ "$line" =~ (^|[^A-Za-z0-9_])$(printf '%s' "$tok" | tr '[:upper:]' '[:lower:]')($|[^A-Za-z0-9_]) ]]; then
                        is_excepted "$rel" "$tok" && continue
                        fail "L2: lower-case '$tok' in a Grooming history callout — $rel:$lineno"
                        echo "       $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
                        echo "       The reader is case-sensitive: '$tok' is a token the pipeline produces,"
                        echo "       the same word in prose is not one (grooming_marker.rs)."
                    fi
                done
            fi
        fi

        # ── L4 — the French typographic space before a protocol label's colon.
        for tok in "${LABEL_TOKENS[@]:-}"; do
            [[ -n "$tok" ]] || continue
            if [[ "$line" == *"$tok :"* ]]; then
                is_excepted "$rel" "$tok" && continue
                fail "L4: '$tok :' carries a space before its colon — $rel:$lineno"
                echo "       $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
                echo "       The reader matches '$tok:' with no space. Canonical: '$tok:'."
            fi
        done
    done < <(strip_code_spans < "$abs")
}

# ── Rule L5 — a label written as an instruction must be declared.
lint_labels() {
    local rel abs declared lbl
    declared="$(grep -E '^\s*-?\s*name:' "$LABELS_FILE" 2>/dev/null | sed 's/.*name: *//' | tr -d '"' | sort -u || true)"
    if [[ -z "$declared" ]]; then
        fail "L5: no label could be parsed from $LABELS_FILE — a guard that parses nothing checks nothing."
        return
    fi

    while IFS= read -r rel; do
        [[ -n "$rel" ]] || continue
        abs="$SCAN_ROOT/$rel"
        while IFS= read -r lbl; do
            [[ -n "$lbl" ]] || continue
            # Shell variables, template placeholders and comma lists are not
            # label names: `"$label"`, `"phase:$phase"`, `phase:K-1` in prose,
            # `"type,priority"` in an issue-creation template.
            case "$lbl" in
                *'$'*|*'<'*|*','*|*'{'*) continue ;;
            esac
            # Here-string, not a pipeline: under `pipefail`, a producer piped
            # into `grep -q` is poisoned by SIGPIPE (mika#2055).
            if ! grep -qxF -- "$lbl" <<< "$declared"; then
                is_excepted "$rel" "$lbl" && continue
                fail "L5: label '$lbl' is written as an instruction in $rel but declared in no labels.yml"
                echo "       label-sync runs with delete-other-labels: true, so an undeclared label is"
                echo "       DELETED from the repo and from every issue carrying it, with no event and"
                echo "       no log. Every --add-label in dispatch-lib.sh is followed by '|| true', so"
                echo "       the write fails in silence. Declare it in .github/labels.yml."
            fi
        done < <(grep -hoE -- '--(add|remove)-label[= ]+["'"'"'`]?[A-Za-z0-9:_.$<,{-]+' "$abs" 2>/dev/null \
                 | sed -E 's/.*label[= ]+["'"'"'`]?//' | sort -u)
    done < <(prescriber_files)
}

# ── S1: the prescribers.
prescriber_files() {
    local found=0
    if [[ -d "$SCAN_ROOT/.claude/commands" ]]; then
        found=1
        find "$SCAN_ROOT/.claude/commands" -name '*.md' -type f | sed "s|^$SCAN_ROOT/||" | sort
    fi
    if [[ -d "$SCAN_ROOT/skills/bundled" ]]; then
        found=1
        find "$SCAN_ROOT/skills/bundled" -name 'system_prompt.md' -type f | sed "s|^$SCAN_ROOT/||" | sort
    fi
    if [[ -f "$SCAN_ROOT/skills/bundled/_shared/dispatch-lib.sh" ]]; then
        found=1
        echo "skills/bundled/_shared/dispatch-lib.sh"
    fi
    [[ $found -eq 0 ]] && echo "__NO_PERIMETER__"
    return 0
}

if [[ -n "$BODY_FILE" ]]; then
    # ── S2/S3: one body. Rules L1–L4 only; L5 is about a repository file
    #    prescribing a write, which a body is not.
    if [[ ! -f "$BODY_FILE" ]]; then
        echo "ERROR: body file not found: $BODY_FILE" >&2
        exit 2
    fi
    lint_lines "$(basename "$BODY_FILE")" "$BODY_FILE"
else
    FILES="$(prescriber_files)"
    if [[ "$FILES" == "__NO_PERIMETER__" ]]; then
        echo "ERROR: no prescriber path exists under '$SCAN_ROOT'." >&2
        echo "       A scan with nothing to scan is a vacuous pass, not a clean one." >&2
        exit 1
    fi
    while IFS= read -r rel; do
        [[ -n "$rel" ]] || continue
        lint_lines "$rel" "$SCAN_ROOT/$rel"
    done <<< "$FILES"

    if [[ -f "$LABELS_FILE" ]]; then
        lint_labels
    else
        fail "L5: $LABELS_FILE not found — the label vocabulary cannot be checked."
    fi
fi

check_stale_exceptions

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS canonical-token violation(s)."
    echo "The list and the tolerance of every reader: scripts/canonical-tokens.tsv"
    echo "Reasoning: docs/plans/2026-09-20-005-feat-2201-lint-jetons-machine-canoniques-plan.md"
    exit 1
fi

echo "Machine tokens are canonical where their reader is strict."
exit 0
