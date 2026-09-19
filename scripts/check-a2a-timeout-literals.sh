#!/usr/bin/env bash
# CI lint: on the a2a path, a call's budget comes from a resolver — never from a
# literal written at the site that bounds it (mika#2309).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THE PREDICATE IS ON THE *SITE*, NOT ON THE VALUE
#
# The founding ticket asked for "every live duration literal >= 60 s on the a2a
# path must be an env-derived DEFAULT". Measured against the code, that rule
# fires on things that are not budgets at all: cache TTLs
# (`check_suite_dedup.rs` ENTRY_TTL = 600), a GitHub-imposed JWT lifetime
# (`github_app.rs` JWT_LIFETIME = 540), audit-dedup horizons (86 400), re-emit
# cadences (3 600). Demanding that a JWT lifetime be single-sourced on an env
# var means nothing — so the guard gets disarmed, or its allowlist becomes the
# junk drawer in which the regression it exists to catch passes unnoticed.
# **That is the failure mode the ticket exists to prevent, reached by the
# remedy.** (Plan § M1.)
#
# So the rule here is not "60 is suspicious". It is: *a call on the a2a path
# whose budget comes from a literal instead of a resolver*. A TTL is not a call
# budget and is never in scope.
#
# DO NOT "TIGHTEN" THIS BACK TOWARDS THE NAIVE PREDICATE. The reasoning and the
# measured table live in
# docs/plans/2026-09-18-004-feat-2309-prevol-d-en-garde-ci-plan.md § M1.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THE EXPRESSION IS EVALUATED, NOT MATCHED
#
# A `from_secs\([0-9]+\)` pattern misses every composed value, and the bypass is
# already written in this repo seven times (`from_secs(24 * 60 * 60)`,
# `from_secs(30 * 60)`, `from_secs(5 * 60)`, …). A guard you evade by typing
# `10 * 60` instead of `600` is worse than no guard: it advertises a coverage it
# does not have. Rule A2 therefore *evaluates* the expression. (Plan § M2.)
#
# ─────────────────────────────────────────────────────────────────────────────
# THE TWO RULES
#
#   A1 — no duration literal at a bounding site.
#        Inside the perimeter, the argument of `.timeout(`, `.connect_timeout(`,
#        `with_timeout(` or `tokio::time::timeout(` may not be a *literal*
#        duration. The predicate is on what `from_secs` receives, not on the
#        presence of `from_secs`: `from_secs(600)` and `from_secs(10 * 60)` are
#        refused; `from_secs(SOME_CONST)`, `from_secs(budget.http_timeout_secs())`
#        and an argument that is not a `Duration::from_*` at all are admitted.
#        This distinction is load-bearing: `openai.rs` and `ollama.rs` both write
#        `.timeout(Duration::from_secs(budget.http_timeout_secs()))`, which is
#        exactly the single-sourcing this guard protects. A rule reading "no
#        inline Duration::from_* at a bounding site" would accuse them on the day
#        it was born. (Plan § Fire-Disposition, surface 1.)
#
#   A2 — a `const … : Duration` >= 60 s must be an env default.
#        Inside the perimeter, any `const NAME: Duration = Duration::from_secs(E)`
#        whose *evaluated* E is >= THRESHOLD_SECS must either be named `DEFAULT_*`
#        and live in a file that also declares its env var (`*_ENV: &str = "MIKA_…"`
#        or an `env::var("MIKA_…")`), or appear in the allowlist with a reason.
#
# THRESHOLD, and what it does not buy. 60 s, taken from the ticket. A 59 s
# literal written at a bounding site would pass A2 — but it does not pass A1,
# which never looks at the value. The threshold protects nothing on its own; it
# bounds A2's population.
#
# ALLOWLIST: `scripts/a2a-timeout-allowlist.txt`, compared **in both
# directions** (an entry matching no site fails the build, exactly like an
# unlisted site). An allowlist nobody prunes is the junk drawer of § M1; the
# two-way comparison is what stops it becoming one. Shipped empty, and measured
# empty on 2026-09-18.
#
# NOT COVERED, deliberately (plan § refusals, point 8): `from_millis`,
# `from_secs_f64`, `from_mins`. Zero occurrences in the workspace, so covering
# them would be speculative. A `const … : Duration` built by any constructor
# other than `from_secs` is therefore not *silently* admitted — it fails with an
# explicit "extend this script" message, so the day one appears it is routed
# here instead of slipping through.
#
# Usage: check-a2a-timeout-literals.sh [scan-root]
#   The optional scan root lets the anti-vacuity harness
#   (scripts/test-check-a2a-timeout-literals.sh) point the guard at a fixture
#   tree, on the mika#2103 model.
#
# Exit 0 if clean, 1 with actionable errors otherwise.

set -euo pipefail

# ── Threshold for rule A2, in seconds. See the header for what it does not buy.
THRESHOLD_SECS=60

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$REPO_ROOT}"

# ── The perimeter, declared and closed.
#
# The pre-flight script named seven crates, two of which (`mika-llm`, `mika-core`)
# do not exist, while the five real ones are the entire workspace minus
# `mika-os`. A perimeter that subtracts nothing discriminates nothing — which is
# the direct cause of § M1. These are the files that actually carry an a2a
# exchange or the LLM budget cascade behind it. (Plan § M3, AC8.)
PERIMETER=(
    "crates/mika-a2a/src"                      # the client and its transport
    "crates/mika-cli/src/remote_ask.rs"        # A2aClient call site
    "crates/mika-agent/src/tools/a2a_call.rs"  # A2aClient call site
    "crates/mika-common/src/llm"               # the rail carrying the cascade
)

ALLOWLIST_FILE="$SCAN_ROOT/scripts/a2a-timeout-allowlist.txt"

VIOLATIONS=0

fail() {
    echo "ERROR: $*"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# ── Collect the perimeter's .rs files, relative to SCAN_ROOT.
#
# A perimeter entry that does not exist is skipped (a fixture tree carries only
# one of them), but a run where *nothing* exists is a vacuous pass and is
# refused: a guard that silently scans an empty set is the decoration mika#2103
# is about.
collect_files() {
    local found=0 entry
    for entry in "${PERIMETER[@]}"; do
        local abs="$SCAN_ROOT/$entry"
        if [[ -d "$abs" ]]; then
            found=1
            find "$abs" -name '*.rs' -type f | sed "s|^$SCAN_ROOT/||"
        elif [[ -f "$abs" ]]; then
            found=1
            echo "$entry"
        fi
    done
    if [[ $found -eq 0 ]]; then
        echo "__NO_PERIMETER__"
    fi
}

# ── Emit `path:lineno:content` for every line outside a `#[cfg(test)]` block.
#
# Test code legitimately writes `A2aClient::with_timeout(…, Duration::from_secs(2))`
# — that is a test asserting the override works, not a production budget. The
# pre-flight excluded `#[cfg(test)]` and `tests/` for the same reason.
#
# Brace-depth tracking (rather than "ignore everything after the first
# `#[cfg(test)]`") so a `#[cfg(test)]` in the middle of a file does not blind the
# rest of it. An attribute with no block at all (`#[cfg(test)] mod tests;`) ends
# its skip at the `;`.
emit_live_lines() {
    local rel="$1"
    awk -v rel="$rel" '
        skipping {
            line = $0
            o = gsub(/\{/, "{", line)
            c = gsub(/\}/, "}", line)
            if (o > 0) started = 1
            depth += o - c
            if (started && depth <= 0) skipping = 0
            else if (!started && index($0, ";") > 0) skipping = 0
            next
        }
        /^[[:space:]]*#\[cfg\(test\)\]/ { skipping = 1; depth = 0; started = 0; next }
        { printf "%s:%d:%s\n", rel, FNR, $0 }
    ' "$SCAN_ROOT/$rel"
}

# ── Evaluate a purely arithmetic seconds expression.
#
# Prints the value on success. On anything it cannot evaluate it prints nothing
# and returns 1, so the caller fails *explicitly* rather than ignoring the site
# (AC3): a guard that silently skips what it cannot parse is a guard with a hole
# shaped like whatever it failed to read.
eval_secs() {
    local expr="$1"
    # Rust digit separators are not shell arithmetic.
    local normalized="${expr//_/}"
    if [[ ! "$normalized" =~ ^[0-9\ \*]+$ ]]; then
        return 1
    fi
    # A leading zero is octal to the shell and decimal to Rust. Refuse rather
    # than answer a different number than the compiler sees.
    if [[ "$normalized" =~ (^|[^0-9])0[0-9] ]]; then
        return 1
    fi
    echo $((normalized))
}

# ── Extract the first `from_secs(` argument of a line (text up to its `)`).
from_secs_arg() {
    local line="$1"
    [[ "$line" == *"from_secs("* ]] || return 1
    local tail="${line#*from_secs(}"
    echo "${tail%%)*}"
}

FILES="$(collect_files)"
if [[ "$FILES" == "__NO_PERIMETER__" ]]; then
    echo "ERROR: no perimeter path exists under '$SCAN_ROOT'."
    echo "       A scan with nothing to scan is a vacuous pass, not a clean one."
    exit 1
fi

# ── Rule A1 ──────────────────────────────────────────────────────────────────
#
# Known gap, stated rather than discovered: this is a line-oriented source scan,
# so a bounding site split across lines escapes it, as does a fifth bounding
# helper nobody has written yet. A2 catches the sub-case where the budget is a
# const >= 60 s. (Plan § Risque 2.)
while IFS= read -r rel; do
    [[ -n "$rel" ]] || continue
    while IFS= read -r entry; do
        [[ -n "$entry" ]] || continue
        local_path="${entry%%:*}"
        rest="${entry#*:}"
        lineno="${rest%%:*}"
        content="${rest#*:}"

        case "$content" in
            *".timeout("*|*".connect_timeout("*|*"with_timeout("*|*"tokio::time::timeout("*) ;;
            *) continue ;;
        esac

        arg="$(from_secs_arg "$content")" || continue
        # Only a *literal* duration is refused. An identifier or a function call
        # is the single-sourced shape this guard exists to protect.
        if value="$(eval_secs "$arg")"; then
            fail "A1: literal duration (${value}s) at a bounding site — $local_path:$lineno"
            echo "       $(echo "$content" | sed 's/^[[:space:]]*//')"
            echo "       A budget on the a2a path comes from a resolver, not from a literal."
            echo "       Read it from a resolver (e.g. mika_a2a::client::resolve_send_timeout(),"
            echo "       LlmTimeoutBudget::http_timeout_secs()) or from a DEFAULT_* const that"
            echo "       declares its MIKA_* env var."
        fi
    done < <(emit_live_lines "$rel")
done <<< "$FILES"

# ── Rule A2 ──────────────────────────────────────────────────────────────────
#
# Raw violations are collected first so the allowlist can be compared in both
# directions below.
A2_VIOLATIONS=()

while IFS= read -r rel; do
    [[ -n "$rel" ]] || continue
    while IFS= read -r entry; do
        [[ -n "$entry" ]] || continue
        local_path="${entry%%:*}"
        rest="${entry#*:}"
        lineno="${rest%%:*}"
        content="${rest#*:}"

        [[ "$content" =~ const[[:space:]]+([A-Za-z0-9_]+)[[:space:]]*:[[:space:]]*Duration[[:space:]]*= ]] || continue
        name="${BASH_REMATCH[1]}"

        # Any Duration constructor other than from_secs is out of this script's
        # reach today (zero occurrences). Refuse explicitly rather than admit it
        # in silence — see the header.
        if [[ "$content" != *"from_secs("* ]]; then
            fail "A2: const '$name' builds a Duration by a constructor this guard does not read — $local_path:$lineno"
            echo "       $(echo "$content" | sed 's/^[[:space:]]*//')"
            echo "       Only Duration::from_secs(...) is covered (mika#2309, plan § refusals point 8)."
            echo "       Extend this script's A2 rule rather than leaving the site unread."
            continue
        fi

        arg="$(from_secs_arg "$content")"
        if ! value="$(eval_secs "$arg")"; then
            fail "A2: const '$name' has a duration expression this guard cannot evaluate — $local_path:$lineno"
            echo "       expression: '$arg'"
            echo "       A2 evaluates the expression so '10 * 60' and '600' are the same thing."
            echo "       Only digits, '_', '*' and spaces are evaluable. Simplify it, or extend"
            echo "       eval_secs() — do not leave it unread (AC3)."
            continue
        fi

        (( value >= THRESHOLD_SECS )) || continue

        # Compliant shape: DEFAULT_* whose file also declares the env var that
        # overrides it. `client.rs` is the model: DEFAULT_TIMEOUT alongside
        # TIMEOUT_ENV = "MIKA_A2A_TIMEOUT_SECS".
        declares_env=1
        grep -qE '(_ENV[A-Z_]*:[[:space:]]*&.?str[[:space:]]*=[[:space:]]*"MIKA_|env::var\("MIKA_)' \
            "$SCAN_ROOT/$local_path" || declares_env=0

        if [[ "$name" == DEFAULT_* && $declares_env -eq 1 ]]; then
            continue
        fi

        A2_VIOLATIONS+=("$local_path:$name:$lineno:$value")
    done < <(emit_live_lines "$rel")
done <<< "$FILES"

# ── Allowlist, compared in both directions ───────────────────────────────────
#
# Modelled on check-dispatch-seats-declared.sh: a stale entry fails the build
# just as an unlisted site does. One-way comparison is how an allowlist turns
# into the junk drawer of § M1.
ALLOW_ENTRIES=()
if [[ -f "$ALLOWLIST_FILE" ]]; then
    while IFS= read -r raw; do
        line="${raw%%#*}"
        line="$(echo "$line" | tr -d '[:space:]')"
        [[ -n "$line" ]] && ALLOW_ENTRIES+=("$line")
    done < "$ALLOWLIST_FILE"
fi

matched_allow=()
for viol in "${A2_VIOLATIONS[@]:-}"; do
    [[ -n "$viol" ]] || continue
    vpath="${viol%%:*}"
    vrest="${viol#*:}"
    vname="${vrest%%:*}"
    vrest2="${vrest#*:}"
    vline="${vrest2%%:*}"
    vvalue="${vrest2#*:}"
    key="$vpath:$vname"

    allowed=0
    for entry in "${ALLOW_ENTRIES[@]:-}"; do
        if [[ "$entry" == "$key" ]]; then
            allowed=1
            matched_allow+=("$entry")
        fi
    done

    if [[ $allowed -eq 0 ]]; then
        fail "A2: const '$vname' (${vvalue}s >= ${THRESHOLD_SECS}s) is neither an env default nor allowlisted — $vpath:$vline"
        echo "       Name it DEFAULT_* and declare its MIKA_* env var in the same file,"
        echo "       or add '$key  # <why>' to scripts/a2a-timeout-allowlist.txt."
    fi
done

for entry in "${ALLOW_ENTRIES[@]:-}"; do
    [[ -n "$entry" ]] || continue
    still_matches=0
    for m in "${matched_allow[@]:-}"; do
        [[ "$m" == "$entry" ]] && still_matches=1
    done
    if [[ $still_matches -eq 0 ]]; then
        fail "allowlist entry '$entry' matches no A2 violation — remove it."
        echo "       An allowlist is compared in both directions on purpose: an entry that"
        echo "       outlives its site is how the list becomes a junk drawer (mika#2309 § M1)."
    fi
done

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS a2a timeout-budget violation(s)."
    echo "Reasoning and the measured tables: docs/plans/2026-09-18-004-feat-2309-prevol-d-en-garde-ci-plan.md"
    exit 1
fi

echo "No hardcoded timeout budgets on the a2a path."
exit 0
