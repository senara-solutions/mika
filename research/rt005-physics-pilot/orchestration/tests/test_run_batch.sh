#!/usr/bin/env bash
# Test suite for run-batch.sh — RT-005 orchestration, brick 3/5 (mika#1890).
#
# NO TEST MAKES AN LLM CALL. `mika` is replaced by a stub, and the measurement
# channel is a temp file standing in for mika-spirit's log.
#
# The stub models the POST-mika#1727 topology: `mika ask` is an A2A client and
# spirit writes turn_usage into its own log. The stub keeps spirit minting its
# own session id (`a2a-<task>`) rather than adopting the caller's, which is the
# pre-mika#2070 shape — deliberately the pessimistic case, because the capture
# must keep working against a spirit that predates that fix.
# An earlier version of this suite modelled the pre-1727 wiring and passed 120
# assertions against a channel that captures nothing in reality.
#
# Run: bash research/rt005-physics-pilot/orchestration/tests/test_run_batch.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_BATCH="$SCRIPT_DIR/../run-batch.sh"
PREREG="$SCRIPT_DIR/../PREREGISTRATION.md"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_ne() {
    local label="$1" a="$2" b="$3"
    if [ "$a" != "$b" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label (both were '$a')"
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if printf '%s' "$haystack" | grep -qF -- "$needle"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected to contain: '$needle'"
        echo "    actual (first 200):  '$(printf '%s' "$haystack" | head -c 200)'"
    fi
}

# --- fixture -----------------------------------------------------------------

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export MIKA_AGENTS_ROOT="$TMP_ROOT/agents"
export MIKA_SPIRIT_LOG_FILE="$TMP_ROOT/spirit.log"
CALLS="$TMP_ROOT/mika-calls"
export MIKA_STUB_CALLS="$CALLS"
# Keep the suite fast: the capture retry exists for spirit's non-blocking
# appender, and the stub writes synchronously.
export RT005_CAPTURE_RETRIES=0

setup_agents() {
    for agent in mika-dev-confidence-high mika-dev-confidence-low; do
        mkdir -p "$MIKA_AGENTS_ROOT/$agent"
        printf 'log_level = "info"\n' > "$MIKA_AGENTS_ROOT/$agent/config.toml"
    done
    # The channel probe in live_preamble requires the log to already carry a
    # turn_usage line, exactly as a running spirit would.
    printf '{"target":"mika::otel","fields":{"event":"turn_usage","agent_id":"seed","session_id":"boot"}}\n' \
        > "$MIKA_SPIRIT_LOG_FILE"
}

# Stands in for `mika ask`. Records the invocation and appends a turn_usage line
# to the spirit log under a spirit-minted session id — NOT the one it was given.
make_stub() {
    local stub="$TMP_ROOT/mika"
    cat > "$stub" <<'STUB'
#!/usr/bin/env bash
agent="" session=""
while [ $# -gt 0 ]; do
    case "$1" in
        --agent)      agent="$2"; shift 2 ;;
        --session-id) session="$2"; shift 2 ;;
        *)            shift ;;
    esac
done
printf '%s %s\n' "$agent" "$session" >> "$MIKA_STUB_CALLS"
if [ "${MIKA_STUB_FAIL:-0}" = "1" ]; then echo "stub: provider error" >&2; exit 1; fi
n=$(wc -l < "$MIKA_STUB_CALLS")
if [ "${MIKA_STUB_SILENT:-0}" != "1" ]; then
    printf '{"target":"mika::otel","fields":{"event":"turn_usage","agent_id":"%s","session_id":"a2a-%s","step":0,"input_tokens":11,"output_tokens":7}}\n' \
        "$agent" "$n" >> "$MIKA_SPIRIT_LOG_FILE"
    # A second spirit session inside one slice = another conversation with this
    # agent overlapped the run; the script must refuse to attribute it.
    if [ "${MIKA_STUB_CONTAMINATE:-0}" = "1" ]; then
        printf '{"target":"mika::otel","fields":{"event":"turn_usage","agent_id":"%s","session_id":"a2a-intruder","step":0}}\n' \
            "$agent" >> "$MIKA_SPIRIT_LOG_FILE"
    fi
fi
echo "stub answer ${MIKA_STUB_TAG:-} for $session"
STUB
    chmod +x "$stub"
    export MIKA_BIN="$stub"
}

reset_calls() { : > "$CALLS"; }
call_count() { wc -l < "$CALLS" | tr -d ' '; }
fresh_dir() { local d="$TMP_ROOT/$1"; rm -rf "$d"; echo "$d"; }
fp_of() { jq -r '.design_fingerprint' "$1/manifest.json"; }

run_bb() {
    local out
    out="$("$RUN_BATCH" "$@" 2>&1)"
    LAST_RC=$?
    LAST_OUT="$out"
}

write_auth() {
    local dir="$1" batch="$2" fp="$3" mc="$4" verdict="${5-both knobs moved}" dated="${6-2026-08-29}"
    printf 'batch_id: %s\ndesign_fingerprint: %s\nmanip_check_sha256: %s\nverdict: %s\ndated: %s\n' \
        "$batch" "$fp" "$mc" "$verdict" "$dated" > "$dir/AUTHORIZATION"
}

# Writes a hand-made manip-check artifact. Real ones come from manip-check mode;
# these let the gate be probed without four stub runs per case.
write_manip_check() {
    local dir="$1" coverage="$2" fp="$3"
    mkdir -p "$dir/manip-check"
    jq -n --arg c "$coverage" --arg f "$fp" '{coverage: $c, design_fingerprint: $f}' \
        > "$dir/manip-check/manip-check.json"
    sha256sum "$dir/manip-check/manip-check.json" | cut -d' ' -f1
}

# Prepares a directory whose gate is fully satisfied.
authorize() {
    local dir="$1" batch="$2" fp mc
    fp="$(fp_of "$dir")"
    mc="$(write_manip_check "$dir" complete "$fp")"
    write_auth "$dir" "$batch" "$fp" "$mc"
}

setup_agents
make_stub

# --- dry run and expansion ---------------------------------------------------

echo "dry run and expansion"

DRY="$(fresh_dir dry)"
reset_calls
( unset MIKA_BIN; unset MIKA_AGENTS_ROOT; unset MIKA_SPIRIT_LOG_FILE
  "$RUN_BATCH" dry-run --out-dir "$DRY" --batch-id t-dry >/dev/null 2>&1 )
assert_eq "dry run succeeds with no mika, no agents root, no spirit log" "0" "$?"
assert_eq "dry run makes zero mika calls" "0" "$(call_count)"
assert_eq "dry run creates no run record" "0" "$(ls "$DRY/runs" 2>/dev/null | wc -l | tr -d ' ')"
assert_eq "dry run plans 80 runs" "80" "$(jq '.runs | length' "$DRY/plan.json")"
assert_eq "manifest records 80 runs" "80" "$(jq -r '.run_count' "$DRY/manifest.json")"
assert_eq "every cell holds exactly 20 runs" "[20]" \
    "$(jq -c '[.runs[] | .confidence + "." + .reliability] | group_by(.) | map(length) | unique' "$DRY/plan.json")"
assert_eq "every item appears 8 times" "[8]" \
    "$(jq -c '[.runs[].item_id] | group_by(.) | map(length) | unique' "$DRY/plan.json")"
assert_eq "12 runs carry a perturbed peer answer" "12" "$(jq -r '.perturbed_run_count' "$DRY/manifest.json")"
assert_eq "manifest carries the preregistration hash" \
    "$(sha256sum "$PREREG" | cut -d' ' -f1)" "$(jq -r '.preregistration_sha256' "$DRY/manifest.json")"
assert_contains "manifest carries the existence-not-magnitude disclaimer" \
    "EXISTENCE of an interaction, never its magnitude" "$(jq -r '.disclaimer' "$DRY/manifest.json")"
assert_eq "manifest records both agent-state fingerprints before the batch" "2" \
    "$(jq -r '.agent_state_before | length' "$DRY/manifest.json")"
assert_eq "prompt is byte-identical across the two confidence cells" "1" \
    "$(jq -r '[.runs[] | select(.item_id=="rt005-01" and .reliability=="degradee") | .prompt] | unique | length' "$DRY/plan.json")"
assert_eq "no prompt mentions the confidence prior" "0" \
    "$(jq -r '[.runs[].prompt] | map(select(test("0\\.95|0\\.55"))) | length' "$DRY/plan.json")"
assert_eq "perturbed runs are exactly the degradee-arm runs" "true" \
    "$(jq -r '[.runs[] | select(.perturbed) | .reliability] | unique == ["degradee"]' "$DRY/plan.json")"
assert_eq "three distinct items are perturbed" "3" \
    "$(jq -r '[.runs[] | select(.perturbed) | .item_id] | unique | length' "$DRY/plan.json")"

echo "argument validation"

for bad in 0 -1 abc ""; do
    run_bb dry-run --out-dir "$TMP_ROOT/bad" --replicates "$bad"
    assert_ne "--replicates '$bad' is rejected" "0" "$LAST_RC"
done
run_bb manip-check --out-dir "$TMP_ROOT/bad" --limit abc
assert_ne "--limit abc is rejected" "0" "$LAST_RC"
run_bb manip-check --out-dir "$TMP_ROOT/bad" --limit -1
assert_ne "--limit -1 is rejected" "0" "$LAST_RC"
run_bb dry-run --out-dir "$TMP_ROOT/bad" --peer-seed abc
assert_ne "--peer-seed abc is rejected" "0" "$LAST_RC"

echo "seeded order"

A="$(fresh_dir order-a)"; B="$(fresh_dir order-b)"; C="$(fresh_dir order-c)"
"$RUN_BATCH" dry-run --out-dir "$A" --batch-id t --seed 111 >/dev/null 2>&1
"$RUN_BATCH" dry-run --out-dir "$B" --batch-id t --seed 111 >/dev/null 2>&1
"$RUN_BATCH" dry-run --out-dir "$C" --batch-id t --seed 222 >/dev/null 2>&1
assert_eq "same ordering seed reproduces the same order" \
    "$(jq -c '.order' "$A/manifest.json")" "$(jq -c '.order' "$B/manifest.json")"
assert_ne "a different ordering seed produces a different order" \
    "$(jq -c '.order' "$A/manifest.json")" "$(jq -c '.order' "$C/manifest.json")"
assert_eq "the order is a permutation, not a different run set" \
    "$(jq -c '.order | sort' "$A/manifest.json")" "$(jq -c '.order | sort' "$C/manifest.json")"
assert_eq "ordering seed does not move peer_b's answers" \
    "$(jq -cS '.answers' "$A/peer_b.json")" "$(jq -cS '.answers' "$C/peer_b.json")"
assert_eq "ordering seed does not move the perturbed set" \
    "$(jq -cS '.perturbed_ids' "$A/peer_b.json")" "$(jq -cS '.perturbed_ids' "$C/peer_b.json")"

echo "design fingerprint"

D1="$(fresh_dir fp-1)"; D2="$(fresh_dir fp-2)"; D3="$(fresh_dir fp-3)"; D4="$(fresh_dir fp-4)"
"$RUN_BATCH" dry-run --out-dir "$D1" --batch-id t --seed 1 >/dev/null 2>&1
"$RUN_BATCH" dry-run --out-dir "$D2" --batch-id t --seed 2 >/dev/null 2>&1
"$RUN_BATCH" dry-run --out-dir "$D3" --batch-id t --seed 1 --replicates 3 >/dev/null 2>&1
"$RUN_BATCH" dry-run --out-dir "$D4" --batch-id t --seed 1 --peer-seed 99 >/dev/null 2>&1
assert_ne "fingerprint changes with the ordering seed" "$(fp_of "$D1")" "$(fp_of "$D2")"
assert_ne "fingerprint changes with the replicate count" "$(fp_of "$D1")" "$(fp_of "$D3")"
assert_ne "fingerprint changes with the peer_b seed" "$(fp_of "$D1")" "$(fp_of "$D4")"

# The fingerprint must cover the substantive design, not just item ids: a
# changed prompt or answer with unchanged ids must move it.
PATCHED="$TMP_ROOT/patched-peer.json"
jq '.items[0].prompt = "a different question entirely"' "$D1/peer_b.json" > "$PATCHED"
assert_ne "the bridge output's canonical hash moves when a prompt changes" \
    "$(jq -Sc . "$D1/peer_b.json" | sha256sum)" "$(jq -Sc . "$PATCHED" | sha256sum)"

echo "gate denials (each asserts zero mika calls)"

deny_case() {
    local label="$1" dir="$2" expect_field="$3"; shift 3
    reset_calls
    run_bb batch --out-dir "$dir" --batch-id t-gate "$@"
    assert_eq "$label — exit 3" "3" "$LAST_RC"
    assert_contains "$label — names [$expect_field]" "GATE DENIED [$expect_field]" "$LAST_OUT"
    assert_eq "$label — zero mika calls" "0" "$(call_count)"
}

G="$(fresh_dir gate)"
"$RUN_BATCH" dry-run --out-dir "$G" --batch-id t-gate >/dev/null 2>&1
FP="$(fp_of "$G")"
rm -f "$G/manifest.json"

deny_case "no manip-check artifact" "$G" "manip_check"

MC_PARTIAL="$(write_manip_check "$G" partial "$FP")"
write_auth "$G" t-gate "$FP" "$MC_PARTIAL"
deny_case "partial manip-check cannot open the gate" "$G" "manip_check.coverage"

# The evidence must come from the design about to be spent.
MC_OTHER="$(write_manip_check "$G" complete "some-other-design")"
write_auth "$G" t-gate "$FP" "$MC_OTHER"
deny_case "manip-check run under a different design" "$G" "manip_check.design_fingerprint"

MC="$(write_manip_check "$G" complete "$FP")"

rm -f "$G/AUTHORIZATION"
deny_case "absent authorization" "$G" "AUTHORIZATION"

: > "$G/AUTHORIZATION"
deny_case "empty authorization" "$G" "AUTHORIZATION"

rm -f "$G/AUTHORIZATION"; mkdir -p "$G/AUTHORIZATION"
deny_case "authorization that is a directory" "$G" "AUTHORIZATION"
rmdir "$G/AUTHORIZATION"

write_auth "$G" t-gate "$FP" "$MC"
chmod 000 "$G/AUTHORIZATION"
if [ "$(id -u)" -eq 0 ]; then
    echo "  - skipped unreadable-authorization case (running as root)"
else
    deny_case "unreadable authorization is denied, not read as empty" "$G" "AUTHORIZATION"
fi
chmod 644 "$G/AUTHORIZATION"

write_auth "$G" "some-other-batch" "$FP" "$MC"
deny_case "authorization bound to a different batch" "$G" "batch_id"

write_auth "$G" t-gate "deadbeef" "$MC"
deny_case "stale design fingerprint" "$G" "design_fingerprint"

write_auth "$G" t-gate "$FP" "deadbeef"
deny_case "authorization bound to a different manip-check" "$G" "manip_check_sha256"

write_auth "$G" t-gate "$FP" "$MC" "" "2026-08-29"
deny_case "empty operator verdict" "$G" "verdict"

write_auth "$G" t-gate "$FP" "$MC" "   " "2026-08-29"
deny_case "whitespace-only verdict" "$G" "verdict"

write_auth "$G" t-gate "$FP" "$MC" "moved" ""
deny_case "empty date" "$G" "dated"

write_auth "$G" t-gate "$FP" "$MC"
deny_case "seed changed after authorization" "$G" "manip_check.design_fingerprint" --seed 4242

# A CRLF authorization must still deny on the right field, not on a comparison
# whose two sides print identically.
printf 'batch_id: t-gate\r\ndesign_fingerprint: %s\r\nmanip_check_sha256: %s\r\nverdict: ok\r\ndated: 2026-08-29\r\n' \
    "deadbeef" "$MC" > "$G/AUTHORIZATION"
deny_case "CRLF authorization denies on the real mismatch" "$G" "design_fingerprint"

# R20: the preregistration is a gate factor, so prove it can fail. The script
# resolves both PREREGISTRATION.md and the repo root from its own location, so
# the copy has to sit at the same depth inside the repo — a /tmp copy cannot
# reach Cargo.toml and would fail for the wrong reason.
COPY="$SCRIPT_DIR/../../_nopre-test-$$"
rm -rf "$COPY"; mkdir -p "$COPY"
trap 'rm -rf "$TMP_ROOT" "$COPY"' EXIT
cp "$SCRIPT_DIR/../run-batch.sh" "$COPY/"
NP="$(fresh_dir nopre-out)"
"$COPY/run-batch.sh" dry-run --out-dir "$NP" --batch-id t-gate >/dev/null 2>&1
NP_FP="$(fp_of "$NP")"
NP_MC="$(write_manip_check "$NP" complete "$NP_FP")"
write_auth "$NP" t-gate "$NP_FP" "$NP_MC"
reset_calls
LAST_OUT="$("$COPY/run-batch.sh" batch --out-dir "$NP" --batch-id t-gate 2>&1)"; LAST_RC=$?
assert_eq "absent PREREGISTRATION.md denies — exit 3" "3" "$LAST_RC"
assert_contains "absent PREREGISTRATION.md names [preregistration]" \
    "GATE DENIED [preregistration]" "$LAST_OUT"
assert_eq "absent PREREGISTRATION.md — zero mika calls" "0" "$(call_count)"
rm -rf "$COPY"

assert_eq "a denied batch writes no manifest" "0" "$(ls "$G/manifest.json" 2>/dev/null | wc -l | tr -d ' ')"

# A denied batch must not overwrite a committed plan either.
P="$(fresh_dir denied-plan)"
"$RUN_BATCH" dry-run --out-dir "$P" --batch-id t-dp >/dev/null 2>&1
PLAN_BEFORE="$(sha256sum "$P/plan.json" | cut -d' ' -f1)"
PEER_BEFORE="$(sha256sum "$P/peer_b.json" | cut -d' ' -f1)"
run_bb batch --out-dir "$P" --batch-id t-dp --seed 4242
assert_eq "a denied batch leaves plan.json untouched" \
    "$PLAN_BEFORE" "$(sha256sum "$P/plan.json" | cut -d' ' -f1)"
assert_eq "a denied batch leaves peer_b.json untouched" \
    "$PEER_BEFORE" "$(sha256sum "$P/peer_b.json" | cut -d' ' -f1)"

echo "gate integrity"

assert_eq "the script contains no write to an AUTHORIZATION path" "0" \
    "$(grep -nE '(>|>>|tee|cp |mv )[^|]*AUTHORIZATION' "$RUN_BATCH" | grep -vE '^\s*[0-9]+:\s*#' | wc -l | tr -d ' ')"
assert_eq "the script has no --force-shaped bypass" "0" \
    "$(grep -cE '^\s*(--force|--yes|--skip-gate|--no-gate)\)' "$RUN_BATCH" | tr -d ' ')"
assert_eq "no MIKA_.*BYPASS/SKIP/FORCE env escape hatch" "0" \
    "$(grep -oE '\$\{?MIKA_[A-Z_]*(BYPASS|SKIP|FORCE)[A-Z_]*' "$RUN_BATCH" | wc -l | tr -d ' ')"
assert_eq "the script never references a token field outside comments" "0" \
    "$(grep -vE '^\s*#' "$RUN_BATCH" | grep -cE '(input_tokens|output_tokens|cache_[a-z]+_tokens)' | tr -d ' ')"
run_bb batch --out-dir "$G" --batch-id t-gate --limit 2
assert_eq "--limit is rejected in batch mode" "1" "$LAST_RC"
assert_contains "--limit rejection names the mode" "manip-check only" "$LAST_OUT"

echo "live preamble refuses to run blind"

L="$(fresh_dir live)"
"$RUN_BATCH" dry-run --out-dir "$L" --batch-id t-live >/dev/null 2>&1
authorize "$L" t-live
reset_calls
( MIKA_BIN="" PATH="/nonexistent" HOME="$TMP_ROOT/nohome" "$RUN_BATCH" batch --out-dir "$L" --batch-id t-live ) >/dev/null 2>&1
assert_ne "an unresolvable mika binary aborts" "0" "$?"
assert_eq "…making zero mika calls" "0" "$(call_count)"

mv "$MIKA_AGENTS_ROOT/mika-dev-confidence-low" "$TMP_ROOT/parked"
run_bb batch --out-dir "$L" --batch-id t-live
assert_ne "a missing confidence agent aborts" "0" "$LAST_RC"
assert_contains "…and names the provisioning command" "provision.sh" "$LAST_OUT"
mv "$TMP_ROOT/parked" "$MIKA_AGENTS_ROOT/mika-dev-confidence-low"

SAVED_LOG="$(cat "$MIKA_SPIRIT_LOG_FILE")"
: > "$MIKA_SPIRIT_LOG_FILE"
run_bb batch --out-dir "$L" --batch-id t-live
assert_ne "a spirit log with no turn_usage aborts" "0" "$LAST_RC"
assert_contains "…and names the dead measurement channel" "measurement channel is not live" "$LAST_OUT"
printf '%s\n' "$SAVED_LOG" > "$MIKA_SPIRIT_LOG_FILE"

echo "execution, capture and resume"

E="$(fresh_dir exec)"
"$RUN_BATCH" dry-run --out-dir "$E" --batch-id t-exec >/dev/null 2>&1
authorize "$E" t-exec
reset_calls
run_bb batch --out-dir "$E" --batch-id t-exec
assert_eq "a fully authorized batch runs to completion" "0" "$LAST_RC"
assert_eq "80 runs were executed" "80" "$(call_count)"
assert_eq "80 run records were written" "80" "$(ls "$E/runs" | wc -l | tr -d ' ')"
assert_eq "every record is successful" '["success"]' \
    "$(jq -s -c '[.[].status] | unique' "$E"/runs/*.json)"
assert_contains "every record carries the disclaimer text" "never its magnitude" \
    "$(jq -s -r '[.[].disclaimer] | unique | .[0]' "$E"/runs/*.json)"
assert_eq "every record carries this design's fingerprint" "[\"$(fp_of "$E")\"]" \
    "$(jq -s -c '[.[].design_fingerprint] | unique' "$E"/runs/*.json)"
assert_eq "every record is marked mode=batch" '["batch"]' \
    "$(jq -s -c '[.[].mode] | unique' "$E"/runs/*.json)"
assert_eq "every run captured its turn_usage lines" "0" \
    "$(find "$E/logs" -name '*.turn_usage.jsonl' -empty | wc -l | tr -d ' ')"
assert_eq "every record carries the spirit session it was attributed to" "0" \
    "$(jq -s -c '[.[] | select(.spirit_session_id == "" or .spirit_session_id == null)] | length' "$E"/runs/*.json)"
assert_eq "each capture holds exactly one spirit session" "80" \
    "$(for f in "$E"/logs/*.turn_usage.jsonl; do
         jq -r '.fields.session_id' "$f" | sort -u | wc -l; done | grep -c '^1$' | tr -d ' ')"
assert_eq "the correct agent was invoked for every high-confidence run" "40" \
    "$(grep -c '^mika-dev-confidence-high ' "$CALLS" | tr -d ' ')"
assert_eq "the correct agent was invoked for every low-confidence run" "40" \
    "$(grep -c '^mika-dev-confidence-low ' "$CALLS" | tr -d ' ')"
assert_eq "manifest records both agent-state fingerprints after the batch" "2" \
    "$(jq -r '.agent_state_after | length' "$E/manifest.json")"
assert_contains "manifest carries the operator verdict verbatim" \
    "both knobs moved" "$(jq -r '.operator_verdict' "$E/manifest.json")"
assert_eq "manifest carries the operator date verbatim" "2026-08-29" \
    "$(jq -r '.operator_dated' "$E/manifest.json")"

STATE_BEFORE="$(jq -c '.agent_state_before' "$E/manifest.json")"
reset_calls
run_bb batch --out-dir "$E" --batch-id t-exec
assert_eq "a resume over successful records re-executes nothing" "0" "$(call_count)"
assert_eq "a resume preserves the pre-run agent-state fingerprint" \
    "$STATE_BEFORE" "$(jq -c '.agent_state_before' "$E/manifest.json")"

# Records from another design must not be counted as this design's observations.
jq '.design_fingerprint = "a-different-design"' "$E/runs/$(jq -r '.order[0]' "$E/manifest.json").json" \
    > "$TMP_ROOT/patched.json"
mv "$TMP_ROOT/patched.json" "$E/runs/$(jq -r '.order[0]' "$E/manifest.json").json"
reset_calls
run_bb batch --out-dir "$E" --batch-id t-exec
assert_eq "a record from another design is re-executed, not reused" "1" "$(call_count)"

echo "failure handling"

F="$(fresh_dir failrun)"
"$RUN_BATCH" dry-run --out-dir "$F" --batch-id t-fail >/dev/null 2>&1
authorize "$F" t-fail
reset_calls
MIKA_STUB_FAIL=1 run_bb batch --out-dir "$F" --batch-id t-fail
assert_eq "the circuit breaker stops the batch after 3 consecutive failures" "3" "$(call_count)"
assert_contains "…and says why" "ABORTING" "$LAST_OUT"
assert_eq "the failed runs are recorded as failed" '["failed"]' \
    "$(jq -s -c '[.[].status] | unique' "$F"/runs/*.json)"
assert_contains "a failed record captures stderr" "provider error" \
    "$(jq -s -r '.[0].stderr' "$F"/runs/*.json)"

reset_calls
run_bb batch --out-dir "$F" --batch-id t-fail
assert_eq "a resume re-attempts the failed runs" "80" "$(call_count)"
assert_eq "the retry records attempt 2 for the failed ones" "3" \
    "$(jq -s -c '[.[] | select(.attempt == 2)] | length' "$F"/runs/*.json)"
assert_eq "the retry opened a fresh session id" "3" \
    "$(jq -s -r '[.[] | select(.session_id | test("-a2$"))] | length' "$F"/runs/*.json)"

S="$(fresh_dir silent)"
"$RUN_BATCH" dry-run --out-dir "$S" --batch-id t-silent >/dev/null 2>&1
authorize "$S" t-silent
reset_calls
MIKA_STUB_SILENT=1 run_bb batch --out-dir "$S" --batch-id t-silent
assert_eq "a run with zero turn_usage lines is recorded failed" '["failed"]' \
    "$(jq -s -c '[.[].status] | unique' "$S"/runs/*.json)"
assert_contains "the reason names the missing capture" "no turn_usage lines captured" \
    "$(jq -s -r '.[0].stderr' "$S"/runs/*.json)"
assert_eq "the circuit breaker also stops a silent-capture batch" "3" "$(call_count)"

CT="$(fresh_dir contaminated)"
"$RUN_BATCH" dry-run --out-dir "$CT" --batch-id t-cont >/dev/null 2>&1
authorize "$CT" t-cont
reset_calls
MIKA_STUB_CONTAMINATE=1 run_bb batch --out-dir "$CT" --batch-id t-cont
assert_eq "a slice carrying two spirit sessions is recorded contaminated" '["contaminated"]' \
    "$(jq -s -c '[.[].status] | unique' "$CT"/runs/*.json)"
assert_contains "…naming the attribution failure" "cannot attribute" \
    "$(jq -s -r '.[0].stderr' "$CT"/runs/*.json)"

echo "atomicity"

# write_atomic is extracted and exercised on its own; `die` is its only
# dependency, stubbed here so the probe reports instead of exiting the suite.
wa_probe() {
    cd "$TMP_ROOT" && bash -c '
        set -uo pipefail
        die() { printf "run-batch: %s\n" "$*" >&2; exit 1; }
        eval "$(sed -n "/^write_atomic()/,/^}/p" "$1")"
        printf "%s" "$3" > "$2"
        printf "%s" "$4" | write_atomic "$2" 2>&1 || true' _ "$RUN_BATCH" "$1" "$2" "$3"
}
assert_contains "write_atomic refuses an empty producer" "refusing to write an empty file" \
    "$(wa_probe wa.json 'seed' '')"
assert_eq "…leaving the destination intact after an empty producer" "seed" "$(cat "$TMP_ROOT/wa.json")"
assert_contains "write_atomic refuses malformed JSON" "refusing to write malformed JSON" \
    "$(wa_probe wb.json '{"a":1}' 'not json')"
assert_eq "…leaving the destination intact after malformed JSON" '{"a":1}' "$(cat "$TMP_ROOT/wb.json")"
assert_eq "write_atomic leaves no stray temp files" "0" \
    "$(find "$TMP_ROOT" -maxdepth 1 -name 'w*.json.*' | wc -l | tr -d ' ')"

echo "manip-check"

M="$(fresh_dir manip)"
reset_calls
run_bb manip-check --out-dir "$M" --batch-id t-manip
assert_eq "a full manip-check runs the four cells" "4" "$(call_count)"
assert_eq "a full manip-check is marked complete" "complete" \
    "$(jq -r '.coverage' "$M/manip-check/manip-check.json")"
assert_eq "it binds itself to the design it was run under" "$(fp_of "$M")" \
    "$(jq -r '.design_fingerprint' "$M/manip-check/manip-check.json")"
assert_eq "it covers all four cells" "4" \
    "$(jq -r '.cells_covered | length' "$M/manip-check/manip-check.json")"
assert_eq "it runs on an item peer_b actually perturbs" "true" \
    "$(jq -r --slurpfile p "$M/peer_b.json" \
        '.item_id as $i | ($p[0].perturbed_ids.degradee | index($i)) != null' \
        "$M/manip-check/manip-check.json")"
assert_eq "it records four paired observations" "4" \
    "$(jq -r '.observations | length' "$M/manip-check/manip-check.json")"
assert_eq "observations are binary, never a magnitude" '["boolean"]' \
    "$(jq -c '[.observations[].outputs_identical | type] | unique' "$M/manip-check/manip-check.json")"
assert_eq "it carries raw turn_usage per run (R11)" "true" \
    "$(jq -r '[.runs[].turn_usage | length] | all(. > 0)' "$M/manip-check/manip-check.json")"
assert_contains "the manip-check artifact carries the disclaimer" "never its magnitude" \
    "$(jq -r '.disclaimer' "$M/manip-check/manip-check.json")"
assert_eq "a manip-check writes no AUTHORIZATION" "0" \
    "$(ls "$M/AUTHORIZATION" 2>/dev/null | wc -l | tr -d ' ')"
# Pre-authorization runs must not silently satisfy 4 of the 80.
assert_eq "manip-check records live under manip-check/, not runs/" "4" \
    "$(ls "$M/manip-check/runs" | wc -l | tr -d ' ')"
assert_eq "…and the batch's runs/ is untouched" "0" \
    "$(ls "$M/runs" 2>/dev/null | wc -l | tr -d ' ')"
assert_eq "manip-check records are marked mode=manip-check" '["manip-check"]' \
    "$(jq -s -c '[.[].mode] | unique' "$M"/manip-check/runs/*.json)"

# The trap PREREGISTRATION.md names: a check whose runs all failed must not
# read as evidence, and must not open the gate.
MF="$(fresh_dir manip-failed)"
reset_calls
MIKA_STUB_FAIL=1 run_bb manip-check --out-dir "$MF" --batch-id t-mf
assert_eq "a manip-check whose runs all failed is marked partial" "partial" \
    "$(jq -r '.coverage' "$MF/manip-check/manip-check.json")"
assert_eq "…and reports nothing as comparable" '[false]' \
    "$(jq -c '[.observations[].comparable] | unique' "$MF/manip-check/manip-check.json")"
assert_eq "…and claims no identical outputs" '[false]' \
    "$(jq -c '[.observations[].outputs_identical] | unique' "$MF/manip-check/manip-check.json")"
MF_MC="$(sha256sum "$MF/manip-check/manip-check.json" | cut -d' ' -f1)"
write_auth "$MF" t-mf "$(fp_of "$MF")" "$MF_MC"
reset_calls
run_bb batch --out-dir "$MF" --batch-id t-mf
assert_eq "a failed manip-check cannot open the gate — exit 3" "3" "$LAST_RC"
assert_eq "…and made zero mika calls" "0" "$(call_count)"

ML="$(fresh_dir manip-limited)"
reset_calls
run_bb manip-check --out-dir "$ML" --batch-id t-manip-l --limit 2
assert_eq "a capped manip-check runs only the cap" "2" "$(call_count)"
assert_eq "a capped manip-check is marked partial" "partial" \
    "$(jq -r '.coverage' "$ML/manip-check/manip-check.json")"
write_auth "$ML" t-manip-l "$(fp_of "$ML")" \
    "$(sha256sum "$ML/manip-check/manip-check.json" | cut -d' ' -f1)"
reset_calls
run_bb batch --out-dir "$ML" --batch-id t-manip-l
assert_eq "a capped manip-check cannot open the gate — exit 3" "3" "$LAST_RC"
assert_contains "the denial names coverage" "GATE DENIED [manip_check.coverage]" "$LAST_OUT"
assert_eq "and made zero further mika calls" "0" "$(call_count)"

echo "pre-specified contrasts and per-cell dilution (operator decision 2026-08-29)"

assert_contains "manifest names the labelled arm as the primary contrast" \
    "labelled arm" "$(jq -r '.prespecified_contrasts.primary' "$DRY/manifest.json")"
assert_contains "manifest names the realised perturbation as secondary" \
    "realised perturbation" "$(jq -r '.prespecified_contrasts.secondary' "$DRY/manifest.json")"
assert_contains "manifest records that the contrasts predate the data" \
    "pre-registered before data" "$(jq -r '.prespecified_contrasts.registered' "$DRY/manifest.json")"
assert_contains "manifest carries the both-contrasts reporting rule" \
    "both contrasts are reported" "$(jq -r '.prespecified_contrasts.reporting_rule' "$DRY/manifest.json")"
assert_eq "manifest reports the realised perturbation per cell" \
    '{"high.degradee":6,"high.fiable":0,"low.degradee":6,"low.fiable":0}' \
    "$(jq -cS '.realised_perturbation_by_cell | map_values(.perturbed)' "$DRY/manifest.json")"
assert_eq "each cell is reported over its 20 runs" "[20]" \
    "$(jq -c '[.realised_perturbation_by_cell[].runs] | unique' "$DRY/manifest.json")"
assert_eq "the manip-check artifact reports the per-cell dilution" \
    '{"high.degradee":6,"high.fiable":0,"low.degradee":6,"low.fiable":0}' \
    "$(jq -cS '.realised_perturbation_by_cell | map_values(.perturbed)' "$M/manip-check/manip-check.json")"
# The prose must track --replicates rather than hardcoding the R=2 numbers.
assert_contains "contrast prose says 6 and 14 at R=2" "6 manipulated runs" \
    "$(jq -r '.prespecified_contrasts.secondary' "$DRY/manifest.json")"
assert_contains "contrast prose says 9 at R=3" "9 manipulated runs" \
    "$(jq -r '.prespecified_contrasts.secondary' "$D3/manifest.json")"
assert_contains "contrast prose says 30 vs 30 at R=3" "30 vs 30" \
    "$(jq -r '.prespecified_contrasts.primary' "$D3/manifest.json")"

echo "scale-up bookkeeping"

X="$(fresh_dir extends)"
"$RUN_BATCH" dry-run --out-dir "$X" --batch-id t-ext --replicates 3 --extends-batch t-exec >/dev/null 2>&1
assert_eq "an extension batch records what it extends" "t-exec" \
    "$(jq -r '.extends_batch' "$X/manifest.json")"
assert_eq "an ordinary batch records no extension" "null" \
    "$(jq -r '.extends_batch' "$DRY/manifest.json")"

echo "preregistration content"

PRE="$(cat "$PREREG")"
assert_contains "preregistration states the R=2 to R=3 scaling step" "R=2 → R=3" "$PRE"
assert_contains "preregistration states the 10 to 15 item step" "10 → 15" "$PRE"
assert_contains "preregistration states the claim boundary" \
    "It cannot support a claim about how large it is." "$PRE"
assert_contains "preregistration states the dilution" "6 carry a wrong peer answer" "$PRE"
assert_contains "preregistration fixes the primary contrast" "Primary contrast" "$PRE"
assert_contains "preregistration records the operator settlement before data" \
    "Settled by the operator, 2026-08-29, before any data existed." "$PRE"
assert_contains "preregistration requires both contrasts to be reported" \
    "Reporting only one is a protocol violation, whichever one it is." "$PRE"
assert_contains "preregistration requires the per-cell realised perturbation rate" \
    "realised perturbation rate per cell" "$PRE"
assert_contains "preregistration says the scale-up is a new batch" "extends_batch" "$PRE"
assert_contains "preregistration forbids a gate factor that cannot fail" \
    "can never register a failure" "$PRE"

echo
echo "passed: $PASS   failed: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1
