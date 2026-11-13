#!/bin/bash
# Test suite for scripts/audit-egress-no-log.sh (mika#2788).
#
# THE PROPERTY UNDER TEST IS A LOCKSTEP, not a list. `audit-egress-no-log.sh`
# carries a CLOSED list of `search_*` event names and nothing mechanical tied it
# to its producer, `crates/mika-gateway/src/`. mika#2407 added three legitimate
# startup events; the audit's list stayed at two; for a month the audit printed
# `LEAK (Layer 1): unexpected search_* event names` on a sanctioned event, which
# blocked the mika#1806 milestone close and taught its operator to ignore a rc=1.
# A closed list with no link to its producer does not drift by one notch — it
# drifts by everything added since.
#
# So the comparison here runs in BOTH directions, on the explicit model of
# `scripts/check-dispatch-seats-declared.sh` (cited in CLAUDE.md § Labels):
#
#   * a name emitted by the gateway source and absent from the audit's union
#     → red, at the commit that adds it rather than on the field weeks later;
#   * a name declared in the union that no site emits any more → red too. A
#     closed list carrying dead names is what teaches a reader to stop reading
#     it, and without this direction the list would simply start lying the other
#     way round.
#
# The interface between this guard and the audit is the audit's `--known-events`
# MODE, not a `sed` of its array literals: sourcing the script is excluded (it is
# `set -euo pipefail` and runs on read), and re-parsing its arrays would be a
# second reader that drifts from the first — this ticket's own defect class. One
# reader.
#
# Producer scope is ALL of `crates/mika-gateway/src/**/*.rs`, `#[cfg(test)]`
# modules included. Deliberately fail-closed: an `event = "search_…"` written in
# a test is either a real name to declare or a name to rename.
#
# Run: bash scripts/test-audit-egress-no-log.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AUDIT="$REPO_ROOT/scripts/audit-egress-no-log.sh"
GATEWAY_SRC="$REPO_ROOT/crates/mika-gateway/src"

PASS=0
FAIL=0
TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2788-audit-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# An operator workstation carrying these must not change a test verdict. Every
# one of them steers the audit's own decisions: the expected target, the stale
# suppression knob, the TTL, the fallback log path and the log level banner.
unset MIKA_EGRESS_L2_TARGET MIKA_AUDIT_SUPPRESS_L2_WARN \
      MIKA_EGRESS_L2_ATTESTATION_TTL_DAYS MIKA_EGRESS_L2_ATTESTATION_FILE \
      MIKA_GATEWAY_LOG_FILE MIKA_GATEWAY_DB MIKA_SPIRIT_LOG_FILE 2>/dev/null || true

# ------------------------------------------------------------------
# Assertions — herestrings, never `printf | grep -q`
# ------------------------------------------------------------------

assert_true() {
    local label="$1" ok="$2" detail="${3:-}"
    if [ "$ok" = "1" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        [ -n "$detail" ] && echo "    $detail"
    fi
}

assert_exit() {
    local label="$1" expected="$2" result="$3"
    local actual="${result%%|*}"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        echo "    expected exit: $expected"
        echo "    actual exit:   $actual"
        echo "    output: ${result#*|}"
    fi
}

# Herestring, NOT `printf … | grep -q`: under `pipefail` the pipe form is a
# SIGPIPE trap (grep -q exits at the first match, printf takes 141, pipefail
# promotes it, and a PRESENT needle reads as absent). Documented at
# `scripts/test-verify-egress-no-log.sh:64-68` and linted by
# `scripts/verify-no-sigpipe-grep.sh`.
assert_mentions() {
    local label="$1" needle="$2" result="$3"
    if grep -q -- "$needle" <<< "${result#*|}"; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label -- output does not mention '$needle'"
        echo "    output: ${result#*|}"
    fi
}

assert_absent() {
    local label="$1" needle="$2" result="$3"
    if grep -q -- "$needle" <<< "${result#*|}"; then
        FAIL=$((FAIL + 1)); echo "  XX $label -- output DOES mention '$needle'"
        echo "    output: ${result#*|}"
    else
        PASS=$((PASS + 1)); echo "  ok $label"
    fi
}

# ------------------------------------------------------------------
# Runners
# ------------------------------------------------------------------

# Run the audit against fixture paths; echo "<exit>|<combined output>".
# $4, when given, replaces PATH for that run (used to hide `jq`).
run_audit() {
    local log="$1" db="$2" att="$3" path_override="${4:-$PATH}" out rc=0
    out=$(
        MIKA_GATEWAY_LOG_FILE="$log" \
        MIKA_GATEWAY_DB="$db" \
        MIKA_EGRESS_L2_ATTESTATION_FILE="$att" \
        PATH="$path_override" \
        bash "$AUDIT" 2>&1
    ) || rc=$?
    printf '%s|%s' "$rc" "$out"
}

run_audit_args() {
    local out rc=0
    out=$(bash "$AUDIT" "$@" 2>&1) || rc=$?
    printf '%s|%s' "$rc" "$out"
}

# A PATH under which `command -v jq` fails and NOTHING ELSE changes, so the
# audit's `grep -F` fallback is genuinely visited. Without this the fallback
# predicate stays unexecuted, which is the exact condition under which the two
# field-check predicates drift apart.
#
# Stripping the jq-bearing directories is the only mechanism that works: bash's
# PATH search skips a non-executable or dangling `jq` shim and goes on to find
# the real one, so shadowing cannot hide it. But a bare strip is not a regime —
# it is a different experiment. On a distribution where `jq` lives in
# `/usr/bin`, removing that directory also takes `grep`, `sed`, `sort` and
# `date`, and the audit dies at 127 having reached no branch at all. Measured:
# green on a Gentoo workstation, red on ubuntu-22.04 (mika#2788, CI run
# 39171808290) — the harness said the fallback was pinned when nothing had run.
#
# So each stripped directory is REPLACED, in its own position, by a shim that
# symlinks its whole contents except `jq`. The regime then differs from the
# nominal one in exactly one tool. Both halves of that claim are asserted at
# the call site rather than trusted.
# $1: the PATH value to transform (default: the live PATH).
# $2: the shim directory's basename, so a caller can build an independent one.
nojq_path() {
    local source_path="${1:-$PATH}" shim="$TMPROOT/${2:-nojq-bin}"
    local dir entry out="" replaced=0
    local -a path_dirs links
    mkdir -p "$shim"
    IFS=':' read -ra path_dirs <<< "$source_path"
    for dir in ${path_dirs[@]+"${path_dirs[@]}"}; do
        [ -n "$dir" ] || continue
        if [ -x "$dir/jq" ]; then
            links=()
            for entry in "$dir"/*; do
                [ "${entry##*/}" = "jq" ] && continue
                [ -e "$entry" ] || continue
                links+=("$entry")
            done
            # -f because a merged-usr system lists the same directory twice
            # (/bin and /usr/bin): the second pass must overwrite, not fail.
            [ "${#links[@]}" -gt 0 ] && ln -sf "${links[@]}" "$shim/" 2>/dev/null
            if [ "$replaced" = "0" ]; then
                out+="${out:+:}$shim"
                replaced=1
            fi
            continue
        fi
        out+="${out:+:}$dir"
    done
    printf '%s' "$out"
}

# ------------------------------------------------------------------
# The lockstep itself — the two directions, with a diagnostic per direction
# ------------------------------------------------------------------

# Every `search_*` event name a Rust source tree emits. The pattern is the
# tracing field form (`event = "search…"`), so a `contains(&"search_requested"
# .to_string())` in a test assertion is correctly not a producer.
emitted_names() {
    local dir="$1"
    grep -rhoE 'event[[:space:]]*=[[:space:]]*"search[A-Za-z0-9_]*"' \
        --include='*.rs' "$dir" 2>/dev/null \
        | sed -E 's/.*"(search[A-Za-z0-9_]*)".*/\1/' \
        | sort -u \
        | grep -v '^$' \
        || true
}

# The audit's declared union, read through its own `--known-events` mode.
declared_names() {
    bash "$AUDIT" --known-events 2>/dev/null | cut -f1 | sort -u | grep -v '^$' || true
}

# Compare both ways. Echo "<rc>|<diagnostic>" so the assertions above apply
# unchanged. Each direction prints a diagnostic that ONLY that direction
# produces — a generic "declaration mismatch" would be emitted by any failure of
# the guard and would pin nothing.
check_declaration_lockstep() {
    local dir="$1" emitted declared undeclared dead out="" rc=0
    emitted=$(emitted_names "$dir")
    declared=$(declared_names)

    undeclared=$(comm -23 <(printf '%s\n' "$emitted" | grep -v '^$' || true) \
                          <(printf '%s\n' "$declared" | grep -v '^$' || true))
    dead=$(comm -13 <(printf '%s\n' "$emitted" | grep -v '^$' || true) \
                    <(printf '%s\n' "$declared" | grep -v '^$' || true))

    if [ -n "$undeclared" ]; then
        rc=1
        out+="UNDECLARED: emitted by $dir, unknown to the audit:"$'\n'
        out+="$(printf '%s\n' "$undeclared" | sed 's/^/    /')"$'\n'
        out+="  Declare each in AUDIT_FLOW_EVENTS (per-request, Q4 field shape)"$'\n'
        out+="  or AUDIT_CONFIG_EVENTS (per-startup, operator surface) of"$'\n'
        out+="  scripts/audit-egress-no-log.sh. Until then the runtime audit"$'\n'
        out+="  reports a sanctioned event as a LEAK (mika#2788)."$'\n'
    fi

    if [ -n "$dead" ]; then
        rc=1
        out+="STALE: declared by the audit, emitted by no site under $dir:"$'\n'
        out+="$(printf '%s\n' "$dead" | sed 's/^/    /')"$'\n'
        out+="  Remove the entry, or restore the emission. A closed list that"$'\n'
        out+="  carries dead names is what teaches a reader to stop reading it."$'\n'
    fi

    [ "$rc" = "0" ] && out+="OK: declared union and emitted names agree, both ways."$'\n'
    printf '%s|%s' "$rc" "$out"
}

# ============================================================================
echo ""
echo "A1 — the declared union matches the real tree, in both directions"
echo "------------------------------------------------------------------"
# The guard lands ARMED AND GREEN on the real tree: the same flight that adds
# this guard declares the three names mika#2407 left out, so there is nothing to
# exempt. No exemption table is created — an empty one would give the next
# reader somewhere to drop a name instead of declaring it, which is this
# ticket's defect one notch lower.
R=$(check_declaration_lockstep "$GATEWAY_SRC")
assert_exit "A1 real tree: lockstep holds" "0" "$R"
assert_mentions "A1 says both ways agree" "both ways" "$R"

# ============================================================================
echo ""
echo "A2 — ANTI-VACUITY: the real tree hands the guard at least five names"
echo "---------------------------------------------------------------------"
# A grep that finds nothing — module renamed, emission form changed — would be
# green having ignored everything. That is mika#2205, and it is why A3 and A4
# exist side by side. The count is on the PRODUCER side on purpose: the declared
# side is read through `--known-events` and would stay at five whatever the tree
# did.
real_emitted=$(emitted_names "$GATEWAY_SRC" | wc -l | tr -d ' ')
assert_true "A2 at least 5 emitted search_* names found in the tree" \
    "$([ "${real_emitted:-0}" -ge 5 ] && echo 1 || echo 0)" \
    "emitted_names() returned $real_emitted name(s), expected >= 5.
  If the gateway genuinely stopped emitting events, A4 is the assertion that
  should fire. A count below 5 here means the SCAN went blind — check the
  emission pattern and the include glob before touching the declared list."

real_declared=$(declared_names | wc -l | tr -d ' ')
assert_true "A2 --known-events hands back the five declared names" \
    "$([ "${real_declared:-0}" -eq 5 ] && echo 1 || echo 0)" \
    "declared_names() returned $real_declared name(s), expected exactly 5"

# ============================================================================
echo ""
echo "A3 — direction 1: an emitted name the audit does not declare is red"
echo "--------------------------------------------------------------------"
# SEEN RED against a mutated fixture, not assumed red. A lock validated by
# construction is not validated
# (docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md).
D="$TMPROOT/tree-with-undeclared"
mkdir -p "$D"
cp -r "$GATEWAY_SRC"/. "$D/"
cat > "$D/mika2788_zorglub.rs" <<'RS'
// Fixture only — a sixth event name no audit list declares.
fn emits_an_undeclared_event() {
    info!(
        event = "search_zorglub",
        "an event name the audit's closed list has never heard of"
    );
}
RS
R=$(check_declaration_lockstep "$D")
assert_exit "A3 undeclared emitted name: rc=1" "1" "$R"
assert_mentions "A3 names the offending event" "search_zorglub" "$R"
assert_mentions "A3 diagnostic is the UNDECLARED branch" "UNDECLARED" "$R"
assert_mentions "A3 names AUDIT_FLOW_EVENTS as a remedy" "AUDIT_FLOW_EVENTS" "$R"
assert_mentions "A3 names AUDIT_CONFIG_EVENTS as a remedy" "AUDIT_CONFIG_EVENTS" "$R"
# The branch must not also claim the other direction: a fixture that merely ADDS
# a name leaves every declared name still emitted.
assert_absent "A3 does not also report a stale entry" "STALE" "$R"

# ============================================================================
echo ""
echo "A4 — direction 2: a declared name no site emits any more is red"
echo "----------------------------------------------------------------"
# The self-cleaning half. The fixture is the gateway source MINUS the flow
# module: copying only `src/*.rs` leaves `main.rs` and `settings.rs` (the three
# config events) and drops `egress_search/` (the two flow events), so
# `search_requested` and `search_egress` become declared entries nothing emits.
D="$TMPROOT/tree-without-egress-search"
mkdir -p "$D"
cp "$GATEWAY_SRC"/*.rs "$D/"
R=$(check_declaration_lockstep "$D")
assert_exit "A4 declared-but-unemitted name: rc=1" "1" "$R"
assert_mentions "A4 diagnostic is the STALE branch" "STALE" "$R"
assert_mentions "A4 names search_requested as stale" "search_requested" "$R"
assert_mentions "A4 names search_egress as stale" "search_egress" "$R"
assert_mentions "A4 says a dead name stops being read" "stop reading it" "$R"
assert_absent "A4 does not also report an undeclared name" "UNDECLARED" "$R"

# ============================================================================
echo ""
echo "A5 / A6 — a config event is accepted by NAME and audited by FIELD"
echo "-------------------------------------------------------------------"
# The ticket's test case 4, both halves. The audit reaches rc=0 only when
# Layer 2 is attested and Layer 3 skips, so the harness drives three paths: the
# log fixture, a non-existent DB (Layer 3 SKIP) and an attestation written by
# the producer itself — `--attest`, never a hand-written JSON fixture that would
# drift from the record format.
ATT="$TMPROOT/state/egress-l2-attestation.json"
att_rc=0
MIKA_EGRESS_L2_ATTESTATION_FILE="$ATT" bash "$AUDIT" --attest \
    --target mika2788-harness --by harness >/dev/null 2>&1 || att_rc=$?
assert_true "A5 attestation written by the audit's own --attest path" \
    "$([ "$att_rc" = "0" ] && [ -f "$ATT" ] && echo 1 || echo 0)" \
    "--attest exited $att_rc; record present: $([ -f "$ATT" ] && echo yes || echo no)"

NO_DB="$TMPROOT/there-is-no-db-here.sqlite"

CLEAN_LOG="$TMPROOT/clean.log"
cat > "$CLEAN_LOG" <<'LOG'
{"timestamp":"2026-10-18T05:00:00Z","level":"INFO","event":"search_upstream_resolved","upstream":"none","upstream_source":"unset","api_key_present":false,"required":false,"required_source":"default","endpoint_is_default":true}
{"timestamp":"2026-10-18T05:00:01Z","level":"INFO","event":"search_requested","upstream":"brave","outcome":"ok"}
{"timestamp":"2026-10-18T05:00:01Z","level":"INFO","event":"search_egress","upstream":"brave","status":"200"}
LOG

R=$(run_audit "$CLEAN_LOG" "$NO_DB" "$ATT")
assert_exit "A5 clean log with a config event: rc=0" "0" "$R"
assert_mentions "A5 says only declared names appear" "only declared event names" "$R"
assert_mentions "A5 counts the config event" "search_upstream_resolved events: 1" "$R"
assert_absent "A5 reports no leak" "LEAK (Layer 1)" "$R"

# Same log, one forbidden field on the CONFIG line. Before mika#2788 the
# field check selected only the two flow events, so a `url` on
# `search_upstream_resolved` went past unseen — the hole this closes.
LEAKY_LOG="$TMPROOT/config-event-with-url.log"
cat > "$LEAKY_LOG" <<'LOG'
{"timestamp":"2026-10-18T05:00:00Z","level":"INFO","event":"search_upstream_resolved","upstream":"brave","url":"https://api.search.brave.com/res/v1/web/search?q=SECRET","api_key_present":true}
{"timestamp":"2026-10-18T05:00:01Z","level":"INFO","event":"search_requested","upstream":"brave","outcome":"ok"}
LOG

# Both regimes. `jq` is the default path in CI; the fallback must be visited
# too, or the two predicates drift exactly where nothing looks.
R=$(run_audit "$LEAKY_LOG" "$NO_DB" "$ATT")
assert_exit "A6 (jq regime) forbidden url on a config event: rc=1" "1" "$R"
assert_mentions "A6 (jq regime) names the forbidden field" '"url"' "$R"
assert_mentions "A6 (jq regime) reports a Layer 1 leak" "LEAK (Layer 1)" "$R"

NOJQ_PATH=$(nojq_path)
# The regime is asserted BEFORE it is used, in both directions. A PATH that
# still resolved `jq` would make the rc=1 below pass while testing the branch it
# was meant to avoid; a PATH that lost `grep` would make the audit die at 127
# having reached no branch at all — and the second is what CI actually did.
assert_true "A6 the no-jq regime hides jq" \
    "$( ( PATH="$NOJQ_PATH"; command -v jq >/dev/null 2>&1 ) && echo 0 || echo 1)" \
    "jq still resolves under the stripped PATH, so the fallback branch is not the one being exercised"
nojq_toolchain_intact=1
for needed_tool in grep sed sort date head cat wc tr mkdir dirname mv; do
    ( PATH="$NOJQ_PATH"; command -v "$needed_tool" >/dev/null 2>&1 ) \
        || nojq_toolchain_intact="missing:$needed_tool"
done
assert_true "A6 the no-jq regime keeps every other tool the audit needs" \
    "$nojq_toolchain_intact" \
    "the shim lost a tool ($nojq_toolchain_intact): the audit would die at 127
  instead of reaching its fallback, and the assertions below would be
  measuring an abort rather than a branch."

# The two assertions above pass on a host where SOME jq-free directory happens
# to carry grep — which is this workstation (/bin is a real directory holding
# grep and no jq) and is why the bare strip looked correct here. On a merged-usr
# host every directory carrying grep carries jq too, both PATH entries are
# stripped together, and the strip leaves nothing at all: ubuntu-22.04, where
# the first attempt at this harness went red having measured no branch. So the
# regime is also built from a PATH reduced to a jq-bearing directory ALONE —
# the merged-usr shape — and that is a property of the shim, not of the host.
jq_bearing_dir=$(dirname "$(command -v jq)")
MERGED_USR_PATH=$(nojq_path "$jq_bearing_dir" "nojq-merged-usr")
# The property is "the shim preserves everything the stripped directory HELD",
# never "the shim provides tool X": which of the audit's tools live in jq's
# directory is a packaging decision of the distribution (here /usr/bin carries
# grep and sed but not date, which lives in /bin). Asserting the former is a
# property of the shim; asserting the latter would be a property of the host.
merged_usr_intact=1
merged_usr_checked=0
for needed_tool in grep sed sort date head cat wc tr mkdir dirname mv; do
    [ -x "$jq_bearing_dir/$needed_tool" ] || continue
    merged_usr_checked=$((merged_usr_checked + 1))
    ( PATH="$MERGED_USR_PATH"; command -v "$needed_tool" >/dev/null 2>&1 ) \
        || merged_usr_intact="dropped:$needed_tool"
done
assert_true "A6 anti-vacuity — jq's directory holds some of the audit's tools" \
    "$([ "$merged_usr_checked" -ge 2 ] && echo 1 || echo 0)" \
    "only $merged_usr_checked of the audit's tools live in $jq_bearing_dir, so the
  assertion below would be checking nothing. Name the tools the audit really
  uses before concluding the shim is sound."
assert_true "A6 the regime survives a merged-usr layout (jq's own dir, alone)" \
    "$merged_usr_intact" \
    "built from $jq_bearing_dir alone, the shim $merged_usr_intact — a tool that
  directory DID hold no longer resolves. This is the ubuntu-22.04 shape, where
  every directory carrying grep carries jq too. A bare PATH strip fails it by
  construction; if this is red, the shim stopped replacing what it removes."
assert_true "A6 the merged-usr regime still hides jq" \
    "$( ( PATH="$MERGED_USR_PATH"; command -v jq >/dev/null 2>&1 ) && echo 0 || echo 1)" \
    "jq resolves under $MERGED_USR_PATH — the shim copied the one file it must not"

R=$(run_audit "$LEAKY_LOG" "$NO_DB" "$ATT" "$NOJQ_PATH")
assert_exit "A6 (grep fallback) forbidden url on a config event: rc=1" "1" "$R"
assert_mentions "A6 (grep fallback) names the forbidden field" '"url"' "$R"
# Anti-vacuity on the regime itself: a PATH that still resolved `jq` would make
# the assertion above pass while testing the branch it was meant to avoid.
assert_mentions "A6 (grep fallback) really took the fallback branch" \
    "jq not installed" "$R"

# ============================================================================
echo ""
echo "A7 — a null count prints once, not twice"
echo "------------------------------------------"
# `grep -c` writes `0` and THEN exits 1, so the old `|| echo 0` appended a
# second `0` and the line read `search_requested events: 0 0`.
NOFLOW_LOG="$TMPROOT/config-only.log"
cat > "$NOFLOW_LOG" <<'LOG'
{"timestamp":"2026-10-18T05:00:00Z","level":"INFO","event":"search_upstream_resolved","upstream":"none","api_key_present":false}
LOG
R=$(run_audit "$NOFLOW_LOG" "$NO_DB" "$ATT")
assert_exit "A7 config-only log: rc=0" "0" "$R"
assert_mentions "A7 the null count line ends at the single zero" \
    "search_requested events: 0$" "$R"
assert_absent "A7 no doubled zero anywhere" "events: 0 0" "$R"

# ============================================================================
echo ""
echo "A8 — ONE alternation site, consumed by both field-check branches"
echo "------------------------------------------------------------------"
# STRUCTURAL (source scan, not an audit run). The audit has two implementations
# of the field check — `jq` when present, `grep -F` otherwise. If each builds
# its own line selection they drift; that is lesson V13 of
# `scripts/test-verify-egress-no-log.sh`, where "both parsers call sanitize()"
# was true globally and false per function. So: one construction site, and no
# `search_requested`/`search_egress` literal left in any Layer 1b predicate.
alt_defs=$(grep -c '^readonly KNOWN_EVENT_ALTERNATION=' "$AUDIT")
assert_true "A8 exactly one KNOWN_EVENT_ALTERNATION construction site" \
    "$([ "$alt_defs" -eq 1 ] && echo 1 || echo 0)" \
    "found $alt_defs site(s), expected exactly 1"

layer1b=$(sed -n '/^# Layer 1b/,/^# Layer 2/p' "$AUDIT")
# Anti-vacuity: an empty slice would make every assertion below pass while
# measuring nothing (mika#2205).
layer1b_lines=$(printf '%s\n' "$layer1b" | wc -l | tr -d ' ')
assert_true "A8 anti-vacuity — the Layer 1b slice is non-trivial" \
    "$([ "${layer1b_lines:-0}" -ge 30 ] && echo 1 || echo 0)" \
    "the slice is $layer1b_lines line(s); the section header comments must have moved"

jq_branch=$(sed -n '/if command -v jq/,/^    else$/p' <<< "$layer1b")
fallback_branch=$(sed -n '/^    else$/,/^    fi$/p' <<< "$layer1b")
jq_uses=$(grep -c 'KNOWN_EVENT_ALTERNATION' <<< "$jq_branch")
fb_uses=$(grep -c 'KNOWN_EVENT_ALTERNATION' <<< "$fallback_branch")
assert_true "A8 the jq branch consumes the shared alternation" \
    "$([ "${jq_uses:-0}" -ge 1 ] && echo 1 || echo 0)" \
    "jq branch references it $jq_uses time(s), expected >= 1"
assert_true "A8 the grep fallback consumes the shared alternation" \
    "$([ "${fb_uses:-0}" -ge 1 ] && echo 1 || echo 0)" \
    "fallback branch references it $fb_uses time(s), expected >= 1"

stray=$(grep -n 'search_requested\|search_egress' <<< "$layer1b" || true)
assert_true "A8 no flow-event literal survives in a Layer 1b predicate" \
    "$([ -z "$stray" ] && echo 1 || echo 0)" \
    "literal occurrence(s) inside the Layer 1b section:
$stray
  -> read the name from AUDIT_FLOW_EVENTS / KNOWN_EVENT_ALTERNATION instead. A
     literal here is a second list that drifts from the declared one."

# ============================================================================
echo ""
echo "A9 — this harness is wired into CI and into the Makefile"
echo "----------------------------------------------------------"
# A guard nothing invokes reads exactly like a clean tree
# (docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md
# — the document that cites THIS audit script as an uninvoked guard). Precedent
# for a harness asserting its own wiring: N16 of
# `scripts/test-verify-egress-manifest.sh`.
#
# The predicate is PER JOB / PER TARGET, not per file: a mention anywhere in
# `ci.yml` says nothing about the job that is supposed to run this, which is
# exactly the error a bare `grep -q` would make.
HARNESS_REF="scripts/test-audit-egress-no-log.sh"

ci_job=$(sed -n '/^  egress-no-log-lint:/,/^  [a-z][a-z0-9-]*:$/p' \
    "$REPO_ROOT/.github/workflows/ci.yml")
assert_true "A9 anti-vacuity — the egress-no-log-lint job block was found" \
    "$(grep -q 'scripts/verify-egress-no-log.sh' <<< "$ci_job" && echo 1 || echo 0)" \
    "the slice does not even contain the job's first step; the job may have been
  renamed. Without a block, the assertion below would measure nothing."
assert_true "A9 the egress-no-log-lint job runs this harness" \
    "$(grep -q "$HARNESS_REF" <<< "$ci_job" && echo 1 || echo 0)" \
    "add a step to the \`egress-no-log-lint\` job of .github/workflows/ci.yml:
      - name: Pin the egress audit's declared search_* event list (mika#2788)
        run: bash $HARNESS_REF"

mk_verify=$(sed -n '/^verify-egress-no-log:/,/^$/p' "$REPO_ROOT/Makefile")
assert_true "A9 anti-vacuity — the verify-egress-no-log target block was found" \
    "$(grep -q 'scripts/verify-egress-no-log.sh' <<< "$mk_verify" && echo 1 || echo 0)" \
    "the slice does not contain the target's first recipe line"
assert_true "A9 the verify-egress-no-log target runs this harness" \
    "$(grep -q "$HARNESS_REF" <<< "$mk_verify" && echo 1 || echo 0)" \
    "add \`@bash $HARNESS_REF\` to the \`verify-egress-no-log\` target of the Makefile"

mk_test=$(sed -n '/^test:/,/^$/p' "$REPO_ROOT/Makefile")
assert_true "A9 anti-vacuity — the test target block was found" \
    "$(grep -q 'scripts/test-verify-egress-no-log.sh' <<< "$mk_test" && echo 1 || echo 0)" \
    "the slice does not contain the sibling harness the \`test\` target already runs"
assert_true "A9 the test target runs this harness" \
    "$(grep -q "$HARNESS_REF" <<< "$mk_test" && echo 1 || echo 0)" \
    "add \`@bash $HARNESS_REF\` to the \`test\` target of the Makefile"

# ============================================================================
echo ""
echo "A10 — --known-events is a mode, and it runs no layer"
echo "------------------------------------------------------"
R=$(run_audit_args --known-events)
assert_exit "A10 --known-events: exit 0" "0" "$R"
assert_mentions "A10 prints name<TAB>class" "^search_upstream_resolved	config$" "$R"
assert_mentions "A10 classes a flow event as flow" "^search_requested	flow$" "$R"
# It must short-circuit BEFORE any layer: a mode that also ran the audit would
# make the CI guard depend on the host's log file and DB.
assert_absent "A10 runs no layer" "Layer 1a" "$R"
assert_absent "A10 reads no attestation" "Layer 2" "$R"

R=$(run_audit_args -h)
assert_exit "A10 --help: exit 0" "0" "$R"
assert_mentions "A10 the help names --known-events" "known-events" "$R"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
