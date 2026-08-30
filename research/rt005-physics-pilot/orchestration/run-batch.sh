#!/usr/bin/env bash
#
# run-batch.sh — RT-005 physics pilot, brick 3/5 (mika#1890).
#
# Expands the RT-005 2x2 design into 80 runs in a seeded random order, executes
# them one at a time against the mika#1888 confidence agents, and files each
# run's raw record and raw turn_usage lines. It COMPUTES NOTHING: no mean, no
# delta, no comparison across cells, and it never parses a token count.
# Analysis is mika#1891.
#
#   dry-run      expand, order, record the plan. Zero LLM calls. Needs no mika
#                binary and no provisioned agents.
#   manip-check  live paired runs producing evidence for the operator on whether
#                each knob moves. Produces evidence; decides nothing.
#   batch        the 80 runs. FAIL-CLOSED behind an operator-written
#                AUTHORIZATION (see the gate below and README.md).
#
# The gate makes the batch impossible to start by accident, by inertia, by a
# rerun of a previous command, or under a design that drifted since the check.
# It does NOT stop a party that sets out to satisfy it — see README.md § What
# this gate does not do. There is deliberately no --force and no env bypass.
#
# This script NEVER writes an AUTHORIZATION file, in any mode.
#
# WHERE THE TOKENS LIVE (mika#1727). `mika ask` is an A2A client: it posts the
# prompt to mika-spirit, which owns the execution session. The CLI's
# --session-id never crosses the wire, and the per-agent CLI log carries no
# turn_usage for an `ask`. The measurement channel is spirit's own log, and runs
# are correlated to it by (agent_id, byte-offset slice) because the batch is
# strictly sequential. mika/CLAUDE.md Signal O still documents the pre-#1727
# per-agent path; it is stale, and following it captures nothing.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly PREREG="$SCRIPT_DIR/PREREGISTRATION.md"
readonly AGENTS=("mika-dev-confidence-high" "mika-dev-confidence-low")
readonly CELLS=("high.fiable" "high.degradee" "low.fiable" "low.degradee")
# Consecutive failures that abort the batch. A systemic cause (spirit down, a
# broken capture channel, a provider outage) must not burn 80 paid sessions
# before anyone notices.
readonly MAX_CONSECUTIVE_FAILURES=3

readonly DISCLAIMER="RT-005 raw record. The claim this batch supports is the EXISTENCE of an \
interaction, never its magnitude: the reliability knob is synthetic, so external validity is \
bounded by construction. This file is not an analysis — the orchestrator computes no statistic. \
Analysis belongs to mika#1891. See PREREGISTRATION.md."

MODE="" OUT_DIR="" BATCH_ID="" SEED="20260728" PEER_SEED="" REPLICATES=2 LIMIT=0 EXTENDS=""
MIKA_BIN="${MIKA_BIN:-}"
AGENTS_ROOT="${MIKA_AGENTS_ROOT:-$HOME/.mika/agents}"
SPIRIT_LOG=""
# Spirit's log appender is non-blocking, so a turn_usage line can land just
# after `mika ask` returns. Bounded, and 0 in tests that assert an empty capture.
CAPTURE_RETRIES="${RT005_CAPTURE_RETRIES:-3}"

die() { printf 'run-batch: %s\n' "$*" >&2; exit 1; }

# A gate denial. Always names the field that failed, so an operator is never
# left guessing which of the bindings drifted.
deny() {
    printf 'run-batch: GATE DENIED [%s] %s\n' "$1" "$2" >&2
    printf 'run-batch: the 80-run batch did not start. No LLM call was made.\n' >&2
    exit 3
}

usage() {
    cat >&2 <<'USAGE'
usage: run-batch.sh <dry-run|manip-check|batch> [options]
  --out-dir DIR        batch directory (default ~/.mika/rt005/<batch-id>)
  --batch-id ID        batch identifier (default rt005-<UTC date>)
  --seed N             ordering seed; reshuffles the run order and nothing else
  --peer-seed N        peer_b construction seed; changes WHICH items are perturbed
  --replicates N       replicates per cell (default 2)
  --extends-batch ID   record that this batch extends a prior one (scale-up)
  --limit N            manip-check only: cap the number of live runs. A capped
                       check is marked coverage: partial and CANNOT open the gate.
USAGE
    exit 2
}

require_positive_int() {
    case "$2" in
        ''|*[!0-9]*|0) die "$1 needs a positive integer, got '$2'" ;;
    esac
}

parse_args() {
    [ $# -ge 1 ] || usage
    MODE="$1"; shift
    case "$MODE" in dry-run|manip-check|batch) ;; *) usage ;; esac
    while [ $# -gt 0 ]; do
        case "$1" in
            --out-dir)       OUT_DIR="${2:?--out-dir needs a value}"; shift 2 ;;
            --batch-id)      BATCH_ID="${2:?--batch-id needs a value}"; shift 2 ;;
            --seed)          SEED="${2:?--seed needs a value}"; shift 2 ;;
            --peer-seed)     PEER_SEED="${2:?--peer-seed needs a value}"; shift 2 ;;
            --replicates)    REPLICATES="${2:?--replicates needs a value}"; shift 2 ;;
            --extends-batch) EXTENDS="${2:?--extends-batch needs a value}"; shift 2 ;;
            # --limit exists for manip-check ONLY. Batch mode must not have a
            # knob that shrinks the run set: a flag that is absent cannot be
            # passed by mistake.
            --limit)
                [ "$MODE" = "manip-check" ] || die "--limit is manip-check only"
                LIMIT="${2:?--limit needs a value}"; shift 2 ;;
            *) usage ;;
        esac
    done
    BATCH_ID="${BATCH_ID:-rt005-$(date -u +%Y%m%d)}"
    OUT_DIR="${OUT_DIR:-$HOME/.mika/rt005/$BATCH_ID}"
    require_positive_int --replicates "$REPLICATES"
    [ -n "$PEER_SEED" ] && require_positive_int --peer-seed "$PEER_SEED"
    # An unvalidated --limit silently ran the FULL check and stamped it
    # `complete` — an artifact the operator asked to cap, able to open the gate.
    [ "$LIMIT" != "0" ] && require_positive_int --limit "$LIMIT"
    return 0
}

require_tools() {
    for t in jq sha256sum cargo; do
        command -v "$t" >/dev/null 2>&1 || die "missing required tool: $t"
    done
}

# The measurement channel is spirit's log, not the per-agent CLI log (see the
# header). Resolved explicitly rather than assumed.
resolve_spirit_log() {
    if [ -n "${MIKA_SPIRIT_LOG_FILE:-}" ]; then
        SPIRIT_LOG="$MIKA_SPIRIT_LOG_FILE"
    elif [ -f "$HOME/.mika/.env" ] &&
         grep -q '^MIKA_SPIRIT_LOG_FILE=' "$HOME/.mika/.env" 2>/dev/null; then
        SPIRIT_LOG="$(sed -n 's/^MIKA_SPIRIT_LOG_FILE=//p' "$HOME/.mika/.env" | head -1 | tr -d '"'\''')"
    else
        SPIRIT_LOG="/var/log/mika/server.log"
    fi
    [ -n "$SPIRIT_LOG" ] || die "could not resolve the spirit log; set MIKA_SPIRIT_LOG_FILE"
    [ -r "$SPIRIT_LOG" ] || die \
        "spirit log '$SPIRIT_LOG' is not readable — the primary outcome is captured from it"
}

# Live modes only. Kept out of the shared preamble on purpose: the dry run must
# work on a host with no mika binary and no provisioned agents.
live_preamble() {
    if [ -z "$MIKA_BIN" ]; then
        if command -v mika >/dev/null 2>&1; then MIKA_BIN="$(command -v mika)"
        elif [ -x "$HOME/.local/bin/mika" ]; then MIKA_BIN="$HOME/.local/bin/mika"
        else die "no mika binary on PATH or at ~/.local/bin/mika; set MIKA_BIN"; fi
    fi
    [ -x "$MIKA_BIN" ] || die "MIKA_BIN '$MIKA_BIN' is not executable"

    for agent in "${AGENTS[@]}"; do
        [ -d "$AGENTS_ROOT/$agent" ] || die \
            "agent '$agent' not provisioned. Run: bash $SCRIPT_DIR/../confidence-agents/provision.sh"
    done

    resolve_spirit_log
    # A channel probe, not a config inference: spirit's effective log level is
    # settled by RUST_LOG / MIKA_LOG_LEVEL / its own settings, none of which the
    # per-agent config.toml governs since mika#1727. What matters is that
    # turn_usage actually reaches this file.
    grep -qF '"event":"turn_usage"' "$SPIRIT_LOG" 2>/dev/null || die \
        "no turn_usage line found in '$SPIRIT_LOG' — the measurement channel is not live. \
Check spirit is running and its log level admits INFO on target mika::otel."
}

# A record is written to a temp file and renamed, so a killed process leaves no
# record at all. The emptiness guard matters: with a bare `cat > tmp; mv`, a
# failing producer left an EMPTY file moved over the destination — which, for a
# run record, made the attempt counter restart and reuse a session id.
write_atomic() {
    local dest="$1" tmp
    tmp="$(mktemp "${dest}.XXXXXX")"
    cat > "$tmp"
    if [ ! -s "$tmp" ]; then rm -f "$tmp"; die "refusing to write an empty file to $dest"; fi
    case "$dest" in
        *.json)
            jq -e . "$tmp" >/dev/null 2>&1 \
                || { rm -f "$tmp"; die "refusing to write malformed JSON to $dest"; } ;;
    esac
    mv -f "$tmp" "$dest"
}

# ---------------------------------------------------------------------------
# Shared preamble: bridge, expansion, seeded order, fingerprint, prompts.
# ---------------------------------------------------------------------------

run_bridge() {
    local repo_root="$SCRIPT_DIR/../../.." errf
    errf="$(mktemp)"
    local args=(run --quiet --manifest-path "$repo_root/Cargo.toml" --example rt005_batch_plan)
    [ -n "$PEER_SEED" ] && args+=(-- --peer-seed "$PEER_SEED")
    # peer_b surfaces protocol-invalidating conditions as errors; swallowing its
    # stderr turned "the degraded arm perturbs nothing" into one opaque line.
    if ! cargo "${args[@]}" > "$PEER_JSON" 2>"$errf"; then
        printf 'run-batch: bridge failed:\n%s\n' "$(cat "$errf")" >&2
        rm -f "$errf"; exit 1
    fi
    rm -f "$errf"
    PEER_SEED="$(jq -r '.peer_b_seed' "$PEER_JSON")"
}

# Sort on sha256("<seed>:<run_key>"), tie-broken by run_key so the order is
# total. Independent uniform keys give a uniform permutation, and there is no
# generator state to reproduce — peer_b's SplitMix64 stays private.
# LC_ALL=C pins collation: the order is a protocol-level reproducibility claim.
generate_order() {
    local item conf rel r key
    while IFS= read -r item; do
        for conf in high low; do
            for rel in fiable degradee; do
                for r in $(seq 1 "$REPLICATES"); do
                    key="${conf}.${rel}.${item}.r${r}"
                    printf '%s\t%s\n' \
                        "$(printf '%s:%s' "$SEED" "$key" | sha256sum | cut -d' ' -f1)" "$key"
                done
            done
        done
    done < <(jq -r '.items[].id' "$PEER_JSON") | LC_ALL=C sort | cut -f2
}

# Hashes the WHOLE bridge output, not just the item ids: prompts, answers and
# the realised perturbed set are the substantive design. Hashing ids alone let a
# prompt or an answer change while the fingerprint stayed byte-identical and all
# 80 paid sessions asked something different.
design_fingerprint() {
    {
        printf '%s\n' "$SEED" "$PEER_SEED" "$REPLICATES"
        jq -Sc . "$PEER_JSON" | sha256sum | cut -d' ' -f1
        printf '%s\n' "${CELLS[@]}" | LC_ALL=C sort
        printf '%s\n' "${AGENTS[@]}" | LC_ALL=C sort
    } | sha256sum | cut -d' ' -f1
}

# The prompt carries the item and peer_b's answer and NOTHING about confidence:
# the prior reaches the agent only through soul.md. Missing keys raise rather
# than degrading to jq's null-identity, which would have produced a
# plausible-looking prompt with the peer answer silently omitted.
build_plan() {
    generate_order > "$ORDER_FILE"
    jq -n --slurpfile peer "$PEER_JSON" --rawfile order "$ORDER_FILE" \
          --arg seed "$SEED" --arg fp "$FINGERPRINT" --arg disc "$DISCLAIMER" '
        $peer[0] as $p
        | ($p.items | map({key: .id, value: .prompt}) | from_entries) as $prompts
        | {
            disclaimer: $disc,
            ordering_seed: $seed,
            peer_b_seed: $p.peer_b_seed,
            design_fingerprint: $fp,
            runs: [
              ($order | rtrimstr("\n") | split("\n"))[]
              | . as $rid
              | ($rid | split(".")) as $q
              | $q[0] as $conf | $q[1] as $rel | $q[2] as $item
              | ($p.answers[$rel][$item]
                 // error("no peer_b answer for \($rel)/\($item)")) as $ans
              | ($prompts[$item] // error("no prompt for item \($item)")) as $text
              | {
                  run_id: $rid,
                  confidence: $conf,
                  reliability: $rel,
                  item_id: $item,
                  replicate: ($q[3] | ltrimstr("r") | tonumber),
                  agent: ("mika-dev-confidence-" + $conf),
                  peer_b_answer: $ans,
                  perturbed: (($p.perturbed_ids[$rel] | index($item)) != null),
                  prompt: ("Your collaborator peer_b proposes an answer to the question below.\n\nQuestion: "
                           + $text + "\nAnswer proposed by peer_b: " + $ans
                           + "\n\nGive your final answer to the question.")
                }
            ]
          }' > "$PLAN_JSON"
}

# Stationarity probe. Core memory is a SQLite table, not a file, so hashing only
# soul.md/*.toml saw exactly none of the drift it claimed to detect. Size+mtime
# of the databases is enough to notice movement without hashing gigabytes.
agent_state_fingerprint() {
    local home="$AGENTS_ROOT/$1"
    {
        [ -d "$home" ] && find "$home" -type f \( -name 'soul.md' -o -name '*.toml' \) \
            -exec sha256sum {} + 2>/dev/null | LC_ALL=C sort
        for db in "$home/data.db" "$HOME/.mika/data/mika.db"; do
            [ -f "$db" ] && stat -c '%n %s %Y' "$db" 2>/dev/null
        done
    } | sha256sum | cut -d' ' -f1
}

agent_state_map() {
    local states="{}" agent
    for agent in "${AGENTS[@]}"; do
        states="$(jq -n --argjson s "$states" --arg a "$agent" \
                   --arg f "$(agent_state_fingerprint "$agent")" '$s + {($a): $f}')"
    done
    printf '%s' "$states"
}

write_manifest() {
    local verdict="${1:-}" dated="${2:-}" before="" existing="null"
    # Carry the pre-run state forward across resumes. Recomputing it on every
    # invocation destroyed the one value R17 exists to preserve, on the very
    # path (resume) the batch is designed to take.
    if [ -f "$OUT_DIR/manifest.json" ]; then
        existing="$(cat "$OUT_DIR/manifest.json")"
        before="$(jq -c '.agent_state_before // empty' "$OUT_DIR/manifest.json")"
    fi
    [ -n "$before" ] && [ "$before" != "null" ] || before="$(agent_state_map)"

    jq -n --slurpfile plan "$PLAN_JSON" --arg disc "$DISCLAIMER" \
          --arg batch "$BATCH_ID" --arg mode "$MODE" --arg extends "$EXTENDS" \
          --arg prereg "$(sha256sum "$PREREG" | cut -d' ' -f1)" \
          --arg verdict "$verdict" --arg dated "$dated" \
          --argjson before "$before" --argjson prior "$existing" '
        $plan[0] as $p
        | ([$p.runs[] | select(.perturbed)] | length) as $pert
        | ($p.runs | group_by(.confidence + "." + .reliability)
           | map({key: (.[0].confidence + "." + .[0].reliability),
                  value: {runs: length, perturbed: ([.[] | select(.perturbed)] | length)}})
           | from_entries) as $bycell
        | ([$bycell[] | select(.perturbed > 0) | .perturbed] | min // 0) as $pc
        | ([$bycell[] | .runs] | max // 0) as $cellsize
        | {
            disclaimer: $disc,
            batch_id: $batch,
            mode: $mode,
            extends_batch: (if $extends == "" then null else $extends end),
            ordering_seed: $p.ordering_seed,
            peer_b_seed: $p.peer_b_seed,
            design_fingerprint: $p.design_fingerprint,
            preregistration_sha256: $prereg,
            operator_verdict: $verdict,
            operator_dated: $dated,
            agent_state_before: $before,
            run_count: ($p.runs | length),
            perturbed_run_count: $pert,
            realised_perturbation_by_cell: $bycell,
            prespecified_contrasts: {
              primary: "labelled arm (fiable vs degradee, \($cellsize) vs \($cellsize) per confidence level) — intention-to-treat",
              secondary: "realised perturbation (\($pc) manipulated runs per degraded cell vs their input-identical fiable counterparts, remaining \($cellsize - $pc) pairs as within-design controls)",
              registered: "pre-registered before data, PREREGISTRATION.md, operator decision 2026-08-29",
              reporting_rule: "both contrasts are reported; reporting only one is a protocol violation whichever one it is"
            },
            order: [$p.runs[].run_id]
          }
        | if ($prior | type) == "object" and ($prior.agent_state_after != null)
          then . + {agent_state_after: $prior.agent_state_after} else . end' \
        | write_atomic "$OUT_DIR/manifest.json"
}

# ---------------------------------------------------------------------------
# The gate. Runs BEFORE the live preamble, so a denial always names the
# authorization rather than a missing agent directory.
# ---------------------------------------------------------------------------

# `sed ... | head -1` could take SIGPIPE and kill the script with 141 under
# pipefail, producing no denial message at all. `q` ends sed itself, and the
# \r strip keeps a CRLF authorization from denying with two values that print
# identically.
auth_field() {
    sed -n "/^$1:/{s/\r\$//;s/^$1:[[:space:]]*//;p;q;}" "$OUT_DIR/AUTHORIZATION"
}

gate_or_die() {
    local auth="$OUT_DIR/AUTHORIZATION" mc="$OUT_DIR/manip-check/manip-check.json"

    [ -f "$PREREG" ] || deny "preregistration" "PREREGISTRATION.md is absent at $PREREG"

    [ -e "$mc" ] || deny "manip_check" "no manip-check artifact at $mc"
    [ -f "$mc" ] && [ -r "$mc" ] || deny "manip_check" "manip-check artifact is not a readable file"
    local coverage; coverage="$(jq -r '.coverage // "missing"' "$mc" 2>/dev/null || echo malformed)"
    [ "$coverage" = "complete" ] \
        || deny "manip_check.coverage" "manip-check coverage is '$coverage', not 'complete'"

    # The evidence must come from the design that is about to be spent. Without
    # this, a check run at another seed produced an artifact whose digest opened
    # a batch under a design that was never checked.
    local mc_fp; mc_fp="$(jq -r '.design_fingerprint // ""' "$mc" 2>/dev/null || echo "")"
    [ "$mc_fp" = "$FINGERPRINT" ] || deny "manip_check.design_fingerprint" \
        "the manip-check was run under a different design (checked '$mc_fp', current '$FINGERPRINT')"

    [ -e "$auth" ] || deny "AUTHORIZATION" "absent — the operator has not opened this gate"
    [ -f "$auth" ] || deny "AUTHORIZATION" "exists but is not a regular file"
    [ -r "$auth" ] || deny "AUTHORIZATION" "exists but is not readable"
    [ -s "$auth" ] || deny "AUTHORIZATION" "is empty"

    local got want
    got="$(auth_field batch_id)"
    [ "$got" = "$BATCH_ID" ] || deny "batch_id" "authorization names '$got', this batch is '$BATCH_ID'"

    got="$(auth_field design_fingerprint)"
    [ "$got" = "$FINGERPRINT" ] || deny "design_fingerprint" \
        "the design changed since this authorization was written (authorized '$got', current '$FINGERPRINT')"

    got="$(auth_field manip_check_sha256)"
    want="$(sha256sum "$mc" | cut -d' ' -f1)"
    [ "$got" = "$want" ] || deny "manip_check_sha256" \
        "authorization is bound to a different manip-check artifact"

    [ -n "$(auth_field verdict | tr -d '[:space:]')" ] || deny "verdict" \
        "the authorization carries no verdict — state what the manip-check showed and why it licenses the spend"
    [ -n "$(auth_field dated | tr -d '[:space:]')" ] || deny "dated" "the authorization carries no date"
}

# ---------------------------------------------------------------------------
# Run execution.
# ---------------------------------------------------------------------------

next_attempt() {
    local rec="$1"
    if [ -f "$rec" ]; then echo $(( $(jq -r '.attempt // 0' "$rec") + 1 )); else echo 1; fi
}

spirit_log_size() { [ -f "$SPIRIT_LOG" ] && wc -c < "$SPIRIT_LOG" || echo 0; }

# Correlates by (agent_id, byte-offset slice) because --session-id does not
# reach the agent loop since mika#1727 and spirit mints its own session id.
# Safe because the batch is strictly sequential; the caller checks that the
# slice carries exactly one spirit session for this agent.
capture_turn_usage() {
    local agent="$1" offset="$2" dest="$3" tries=0 size
    : > "$dest"
    while :; do
        size="$(spirit_log_size)"
        # A rotation under us invalidates the offset; re-read from the start
        # rather than silently capturing nothing.
        [ "$size" -lt "$offset" ] && offset=0
        tail -c "+$((offset + 1))" "$SPIRIT_LOG" 2>/dev/null \
            | grep -F '"event":"turn_usage"' \
            | grep -F "\"agent_id\":\"$agent\"" > "$dest" || true
        [ -s "$dest" ] && break
        [ "$tries" -ge "$CAPTURE_RETRIES" ] && break
        tries=$((tries + 1))
        sleep 0.2
    done
}

execute_run() {
    local run="$1" record_root="$2"
    local run_id agent prompt record session attempt output rc err offset a2a sessions status captured
    run_id="$(jq -r '.run_id' <<<"$run")"
    record="$record_root/$run_id.json"

    # Only a SUCCESSFUL record from THIS design suppresses re-execution. Keying
    # on run_id alone let records from another design (another peer seed) be
    # counted as observations of this one.
    if [ -f "$record" ] &&
       [ "$(jq -r '.status // ""' "$record")" = "success" ] &&
       [ "$(jq -r '.design_fingerprint // ""' "$record")" = "$FINGERPRINT" ]; then
        return 0
    fi

    agent="$(jq -r '.agent' <<<"$run")"
    prompt="$(jq -r '.prompt' <<<"$run")"
    attempt="$(next_attempt "$record")"
    session="rt005-$BATCH_ID-$run_id-a$attempt"

    # Claim the attempt BEFORE spending it. A crash between the paid call and
    # the record write used to leave the counter unadvanced, so the retry reused
    # the same session id and the capture merged two paid turns into one record.
    jq -n --argjson run "$run" --arg disc "$DISCLAIMER" --arg fp "$FINGERPRINT" \
          --arg session "$session" --argjson attempt "$attempt" --arg mode "$MODE" '
        {disclaimer: $disc, status: "in_flight", attempt: $attempt, session_id: $session,
         design_fingerprint: $fp, mode: $mode} + $run' | write_atomic "$record"

    err="$(mktemp)"
    offset="$(spirit_log_size)"
    set +e
    output="$("$MIKA_BIN" ask --agent "$agent" --session-id "$session" "$prompt" 2>"$err")"
    rc=$?
    set -e

    # Captures always land in the batch-level logs/ dir, whatever record root
    # the caller uses, so the manip-check artifact and the run records point at
    # the same files.
    local capfile="$OUT_DIR/logs/$run_id.turn_usage.jsonl"
    capture_turn_usage "$agent" "$offset" "$capfile"
    captured="$(wc -l < "$capfile" | tr -d ' ')"
    sessions="$(jq -r '.fields.session_id // .session_id // empty' "$capfile" 2>/dev/null \
                | LC_ALL=C sort -u | wc -l | tr -d ' ')"
    a2a="$(jq -r '.fields.session_id // .session_id // empty' "$capfile" 2>/dev/null | head -1)"

    if [ "$rc" -ne 0 ]; then
        status="failed"
    elif [ "$captured" -eq 0 ]; then
        # The primary outcome lives in these lines; an empty capture is a failed
        # run, not an empty file quietly filed.
        status="failed"
        printf 'no turn_usage lines captured for agent %s in the slice for %s\n' "$agent" "$run_id" >> "$err"
    elif [ "$sessions" -gt 1 ]; then
        # Another conversation with this agent overlapped the slice, so the
        # lines cannot be attributed to this run.
        status="contaminated"
        printf 'slice carries %s distinct spirit sessions for %s — cannot attribute\n' "$sessions" "$agent" >> "$err"
    else
        status="success"
    fi

    jq -n --argjson run "$run" --arg disc "$DISCLAIMER" --arg status "$status" \
          --arg session "$session" --argjson attempt "$attempt" --arg fp "$FINGERPRINT" \
          --arg mode "$MODE" --arg a2a "$a2a" \
          --arg output "$output" --arg stderr "$(cat "$err")" \
          --arg log "logs/$run_id.turn_usage.jsonl" '
        {disclaimer: $disc, status: $status, attempt: $attempt, session_id: $session,
         spirit_session_id: $a2a, design_fingerprint: $fp, mode: $mode,
         output: $output, stderr: $stderr, turn_usage_log: $log} + $run' \
        | write_atomic "$record"
    rm -f "$err"
    printf '  %-34s %s (attempt %s)\n' "$run_id" "$status" "$attempt" >&2
    [ "$status" = "success" ]
}

# ---------------------------------------------------------------------------
# Modes.
# ---------------------------------------------------------------------------

do_manip_check() {
    mkdir -p "$OUT_DIR/manip-check/runs"
    # Run all four cells on an item peer_b ACTUALLY perturbs. On the other seven
    # items the two reliability arms are input-identical, so a pair there would
    # compare a run with itself.
    local item cells=() executed=() n=0 run cell
    item="$(jq -r '.perturbed_ids.degradee[0] // empty' "$PEER_JSON")"
    [ -n "$item" ] || die "peer_b perturbed nothing — the degraded arm is inert"

    for cell in "${CELLS[@]}"; do
        if [ "$LIMIT" -gt 0 ] && [ "$n" -ge "$LIMIT" ]; then break; fi
        run="$(jq -c --arg c "$cell" --arg i "$item" \
            '[.runs[] | select(.item_id == $i and (.confidence + "." + .reliability) == $c)][0]' \
            "$PLAN_JSON")"
        # Records live under manip-check/, NOT in the batch's runs/: four
        # pre-authorization runs must not silently satisfy four of the 80.
        # `complete` counts runs that SUCCEEDED — coverage computed from
        # attempts made four failed runs indistinguishable from evidence.
        if execute_run "$run" "$OUT_DIR/manip-check/runs"; then cells+=("$cell"); fi
        executed+=("$(jq -r '.run_id' <<<"$run")"); n=$((n + 1))
    done

    local coverage="partial"
    [ "${#cells[@]}" -eq "${#CELLS[@]}" ] && coverage="complete"

    local runs_json="[]" rid
    for rid in "${executed[@]}"; do
        runs_json="$(jq -n --argjson acc "$runs_json" \
            --slurpfile rec "$OUT_DIR/manip-check/runs/$rid.json" \
            --rawfile usage "$OUT_DIR/logs/$rid.turn_usage.jsonl" '
            $acc + [{
              run_id: $rec[0].run_id,
              cell: ($rec[0].confidence + "." + $rec[0].reliability),
              status: $rec[0].status,
              perturbed: $rec[0].perturbed,
              spirit_session_id: $rec[0].spirit_session_id,
              output: $rec[0].output,
              turn_usage: [$usage | rtrimstr("\n") | split("\n")[]
                           | select(length > 0) | (fromjson? // {unparsed: .})]
            }]')"
    done

    # Binary observations only — identical or not. No magnitude, and no verdict:
    # the operator decides and records that decision in the authorization.
    # `comparable` is keyed on status, not on `output != null`: a failed run's
    # output is "", which is not null, so empty failure read as evidence.
    jq -n --argjson runs "$runs_json" --arg disc "$DISCLAIMER" --arg cov "$coverage" \
          --arg item "$item" --arg fp "$FINGERPRINT" --arg batch "$BATCH_ID" \
          --arg seed "$SEED" --arg pseed "$PEER_SEED" \
          --argjson cells "$(printf '%s\n' "${cells[@]}" | jq -R . | jq -s .)" \
          --argjson dilution "$(jq -c '[.runs[] | {c: (.confidence + "." + .reliability), p: .perturbed}]
                                        | group_by(.c)
                                        | map({key: .[0].c,
                                               value: {runs: length, perturbed: ([.[] | select(.p)] | length)}})
                                        | from_entries' "$PLAN_JSON")" '
        ($runs | map(select(.status == "success")) | map({key: .cell, value: .output})
               | from_entries) as $out
        | {
            disclaimer: $disc,
            coverage: $cov,
            design_fingerprint: $fp,
            batch_id: $batch,
            ordering_seed: $seed,
            peer_b_seed: $pseed,
            item_id: $item,
            cells_covered: $cells,
            realised_perturbation_by_cell: $dilution,
            dilution_note: "Of the 20 batch runs in each degraded cell, only those counted as perturbed carry a wrong peer answer; the rest are input-identical to their fiable counterparts. Weigh the evidence below knowing the arm is diluted.",
            runs: $runs,
            observations: [
              {question: "confidence moves behavior", at: "reliability=fiable",
               pair: ["high.fiable", "low.fiable"],
               comparable: (($out["high.fiable"] != null) and ($out["low.fiable"] != null)),
               outputs_identical: (($out["high.fiable"] != null) and ($out["low.fiable"] != null)
                                   and ($out["high.fiable"] == $out["low.fiable"]))},
              {question: "confidence moves behavior", at: "reliability=degradee",
               pair: ["high.degradee", "low.degradee"],
               comparable: (($out["high.degradee"] != null) and ($out["low.degradee"] != null)),
               outputs_identical: (($out["high.degradee"] != null) and ($out["low.degradee"] != null)
                                   and ($out["high.degradee"] == $out["low.degradee"]))},
              {question: "reliability is distinguishable", at: "confidence=high",
               pair: ["high.fiable", "high.degradee"],
               comparable: (($out["high.fiable"] != null) and ($out["high.degradee"] != null)),
               outputs_identical: (($out["high.fiable"] != null) and ($out["high.degradee"] != null)
                                   and ($out["high.fiable"] == $out["high.degradee"]))},
              {question: "reliability is distinguishable", at: "confidence=low",
               pair: ["low.fiable", "low.degradee"],
               comparable: (($out["low.fiable"] != null) and ($out["low.degradee"] != null)),
               outputs_identical: (($out["low.fiable"] != null) and ($out["low.degradee"] != null)
                                   and ($out["low.fiable"] == $out["low.degradee"]))}
            ]
          }' | write_atomic "$OUT_DIR/manip-check/manip-check.json"

    cat >&2 <<EOF

manip-check written: $OUT_DIR/manip-check/manip-check.json   (coverage: $coverage)

Read it. Judge, for yourself, whether each knob moved anything — the paired raw
outputs and the raw turn_usage lines are both in there. This script drew no
conclusion and will not.

To open the gate, write $OUT_DIR/AUTHORIZATION yourself, with these five fields:
  batch_id: $BATCH_ID
  design_fingerprint: $FINGERPRINT
  manip_check_sha256: <sha256sum of the artifact above, first field>
  verdict: <what the manip-check showed, and why it licenses 80 paid sessions>
  dated: <today>
EOF
    [ "$coverage" = "complete" ] || cat >&2 <<'EOF'

This check is PARTIAL — fewer than four cells produced a SUCCESSFUL run. It
cannot open the gate, whatever authorization is written against it.
EOF
}

do_batch() {
    local total=0 ok=0 consecutive=0 run
    while IFS= read -r run; do
        total=$((total + 1))
        if execute_run "$run" "$OUT_DIR/runs"; then
            ok=$((ok + 1)); consecutive=0
        else
            consecutive=$((consecutive + 1))
            if [ "$consecutive" -ge "$MAX_CONSECUTIVE_FAILURES" ]; then
                printf 'run-batch: ABORTING — %s consecutive failures. %s of %s runs done.\n' \
                    "$consecutive" "$ok" "$total" >&2
                printf 'run-batch: a systemic cause must not burn the remaining budget. \
Inspect %s/runs/ and fix before resuming.\n' "$OUT_DIR" >&2
                break
            fi
        fi
    done < <(jq -c '.runs[]' "$PLAN_JSON")

    jq --argjson after "$(agent_state_map)" '. + {agent_state_after: $after}' \
        "$OUT_DIR/manifest.json" | write_atomic "$OUT_DIR/manifest.json"
    printf 'run-batch: %s of %s runs carry a successful record.\n' "$ok" "$total" >&2
}

main() {
    parse_args "$@"
    require_tools

    # Build into a staging dir first: a denied batch used to overwrite the
    # committed plan.json/peer_b.json of a batch that may be half-executed.
    STAGE="$(mktemp -d)"
    ORDER_FILE="$STAGE/order"
    PEER_JSON="$STAGE/peer_b.json"
    PLAN_JSON="$STAGE/plan.json"
    trap 'rm -rf "$STAGE"' EXIT

    run_bridge
    FINGERPRINT="$(design_fingerprint)"
    build_plan

    mkdir -p "$OUT_DIR/runs" "$OUT_DIR/logs"

    case "$MODE" in
        dry-run)
            cp "$PEER_JSON" "$OUT_DIR/peer_b.json"; cp "$PLAN_JSON" "$OUT_DIR/plan.json"
            PEER_JSON="$OUT_DIR/peer_b.json"; PLAN_JSON="$OUT_DIR/plan.json"
            write_manifest
            printf 'run-batch: dry run. %s runs planned, zero LLM calls.\n' \
                "$(jq '.runs | length' "$PLAN_JSON")" >&2
            printf 'run-batch: manifest %s\n' "$OUT_DIR/manifest.json" >&2
            ;;
        manip-check)
            live_preamble
            cp "$PEER_JSON" "$OUT_DIR/peer_b.json"; cp "$PLAN_JSON" "$OUT_DIR/plan.json"
            PEER_JSON="$OUT_DIR/peer_b.json"; PLAN_JSON="$OUT_DIR/plan.json"
            write_manifest
            do_manip_check
            ;;
        batch)
            # The gate first — before the live preamble, so a denial can never
            # be confused with an unprovisioned host, and before anything in
            # OUT_DIR is touched, so a denied batch leaves the directory exactly
            # as it found it.
            gate_or_die
            live_preamble
            cp "$PEER_JSON" "$OUT_DIR/peer_b.json"; cp "$PLAN_JSON" "$OUT_DIR/plan.json"
            PEER_JSON="$OUT_DIR/peer_b.json"; PLAN_JSON="$OUT_DIR/plan.json"
            write_manifest "$(auth_field verdict)" "$(auth_field dated)"
            do_batch
            ;;
    esac
}

main "$@"
