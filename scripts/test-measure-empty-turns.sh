#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/measure-empty-turns (mika#1910).
#
# WHY THIS FILE EXISTS, AND WHAT IT FOUND ON ITS FIRST RUN. `measure-empty-turns`
# shipped without a test. Its classification stage ended in `map(fromjson)` over
# a slurped file that already held parsed objects, so the very first invocation
# died with `only strings can be parsed` and exit 2 — "nothing measured". The
# script could not classify a single line, and nothing said so, because nothing
# had ever run it. A geste reproductible that has never been reproduced is a
# geste nobody has checked.
#
# THE ASYMMETRY THIS BATTERY IS BUILT ON. A negative-only battery is satisfied by
# an analyzer that never says `empty_response`, and a positive-only battery by
# one that always does. So the positive control (§2) and the four negative
# controls (§3) are both required, and the negatives are asserted SEPARATELY —
# one `trace_id` per confusion — rather than through a fixture that neutralises
# them together. A conjunction of fail-safe terms is not proven by invalidating
# all of them at once (the mika#2277 lesson).
#
# THE FIXTURES ARE FROZEN AND SYNTHETIC, DELIBERATELY. They are not sampled from
# `$MIKA_SPIRIT_LOG_FILE`: a suite reading the live corpus would change verdict
# as the corpus grows, and could not run on a machine that has never dispatched.
# Fidelity comes from the PRODUCER — every field below is what `emit_turn_usage`
# writes through `tracing_subscriber`'s JSON layer with `flatten_event(true)`,
# including the `?`-sigil Debug string form of `response_chars`
# (`"Some(240)"` / `"Some(0)"` / `"None"`), which is the one shape a naive
# analyzer expecting a JSON number would read as "unmeasured" on EVERY line —
# reporting zero occurrences of the class on precisely the population the ticket
# is about.
#
# Run: make test-measure-empty-turns   (or: bash scripts/test-measure-empty-turns.sh)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MEASURE_PATH="$REPO_ROOT/scripts/measure-empty-turns"
FIXTURES="$REPO_ROOT/crates/mika-agent/tests/fixtures/mika1910"
CORPUS="$FIXTURES/turn_usage_corpus.jsonl"
WIRE="$FIXTURES/wire_forms.jsonl"

# Scratch stays INSIDE the worktree (`target/` is gitignored): the dispatch
# permission-policy refuses writes outside it, and a suite that cannot write is
# a suite that reports nothing.
WORK="$REPO_ROOT/target/mika1910-test"
rm -rf "$WORK"
mkdir -p "$WORK"

# Invoked through `bash` rather than by path, deliberately: the exec bit is a
# property of the committed tree, not of whatever checkout this runs in, and a
# worktree restored without it would hide every result below behind one opaque
# "permission denied". §1 asserts the bit where it is actually contracted.
MEASURE() { bash "$MEASURE_PATH" "$@"; }

PASS=0
FAIL=0

ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
ko()  { FAIL=$((FAIL + 1)); printf '  FAIL %s\n     expected: %s\n     actual:   %s\n' "$1" "$2" "$3"; }
eq()  { if [[ "$2" == "$3" ]]; then ok "$1"; else ko "$1" "$2" "$3"; fi; }

# Class of one trace_id in a turns file.
classe_of() { jq -r --arg t "$2" 'select(.trace_id == $t) | .classe' "$1"; }
# One aggregate field.
sum_of()    { jq -r "$2" "$1"; }

printf '\n== mika#1910 — measure-empty-turns ==\n\n'

# ---------------------------------------------------------------------------
printf -- '-- 0. the corpus and the script are where this suite says they are\n'
# ---------------------------------------------------------------------------
[[ -f "$MEASURE_PATH" ]] && ok "measure-empty-turns exists" \
    || ko "measure-empty-turns exists" "a file at $MEASURE_PATH" "absent"
[[ -f "$CORPUS" ]] && ok "frozen corpus exists" \
    || ko "frozen corpus exists" "a file at $CORPUS" "absent"

# The exec bit is contracted in the git index, not on this filesystem.
MODE="$(cd "$REPO_ROOT" && git ls-files -s scripts/measure-empty-turns | awk '{print $1}')"
eq "the committed mode is executable (100755)" "100755" "$MODE"

# ---------------------------------------------------------------------------
printf -- '\n-- 1. the run succeeds at all (the defect this suite found)\n'
# ---------------------------------------------------------------------------
MEASURE --log-file "$CORPUS" --agent mika-dev \
        --out "$WORK/turns.jsonl" --summary "$WORK/summary.json" 2>"$WORK/stderr.txt"
RC=$?
eq "exit 0 on a readable corpus" "0" "$RC"
if [[ "$RC" -ne 0 ]]; then
    printf '     stderr: %s\n' "$(cat "$WORK/stderr.txt")"
    printf '\n  %d passed, %d FAILED — aborting, nothing else is measurable\n\n' "$PASS" "$FAIL"
    exit 1
fi
LINES="$(grep -c . "$WORK/turns.jsonl" || true)"
eq "the run classified something (anti-vacuity of the run itself)" "8" "$LINES"

# ---------------------------------------------------------------------------
printf -- '\n-- 2. anti-vacuity of the PREDICATE, in both directions (VC §3)\n'
# ---------------------------------------------------------------------------
# The positive direction is load-bearing: an analyzer that could never say
# `empty_response` would pass every negative control below and be worth nothing.
#
# `output_tokens: 800` on this fixture is NOT decorative. It pins the frontier
# with N3 — same `response_chars: 0`, same `output_tokens > 0`, DIFFERENT
# `stop_reason` — so a predicate that swallowed rule 3 into rule 4 would stay
# green without it. It also anchors the `strip_internal_tags` false positive
# (§6) on an executed case rather than on a note.
eq "a continuation turn at chars=0 / success / EndTurn / output>0 IS the class" \
   "empty_response" "$(classe_of "$WORK/turns.jsonl" t1)"
eq "a continuation turn at chars=240 is produced" \
   "produced" "$(classe_of "$WORK/turns.jsonl" t2)"

# ---------------------------------------------------------------------------
printf -- '\n-- 3. the three confusions, each with its own control (VC §4)\n'
# ---------------------------------------------------------------------------
# N1 (R5) — the test whose absence would make the measurement WRONG without
# making it RED. `null` means "not measured"; counting it as empty would put
# every pre-deploy line, every error arm and every pre-mika#1910 continuation
# row into the class, at a false-positive rate of 100% on the population of
# interest.
eq "N1 — chars null is undetermined, never empty_response" \
   "undetermined" "$(classe_of "$WORK/turns.jsonl" t3)"

# N2 (R6) — mika#2357 is a glm-5.3 population with status=error and
# input_tokens=0. Counting it here would file it under a glm-5.2 ticket.
eq "N2 — status=error is error, never empty_response" \
   "error" "$(classe_of "$WORK/turns.jsonl" t4)"

# N3 (R7) — mika#1665: text empty + output cap reached + output_tokens > 0 is a
# reasoning-budget exhaustion (fixed by a setting), not a model regression. The
# rule has ONE definition site, `calibration::failure::classify_failure`; the
# Rust side pins the agreement of its two consumers.
eq "N3 — MaxTokens + output>0 + chars=0 is reasoning_budget_exhausted" \
   "reasoning_budget_exhausted" "$(classe_of "$WORK/turns.jsonl" t5)"

# The `output_tokens > 0` term of rule 3 is load-bearing: without it, every
# MaxTokens turn at chars=0 would leave the class.
eq "N3b — MaxTokens WITHOUT output>0 falls through to the class" \
   "empty_response" "$(bash "$MEASURE_PATH" --log-file "$WIRE" --agent mika-dev \
       --out "$WORK/wire.jsonl" --summary "$WORK/wire-summary.json" 2>/dev/null; \
       classe_of "$WORK/wire.jsonl" w5)"

# ---------------------------------------------------------------------------
printf -- '\n-- 4. N4 — the unit is the TURN, not the call\n'
# ---------------------------------------------------------------------------
# An in-loop line (step != u32::MAX) at chars=0 does not make its trace_id a
# turn of the class: a turn emitting only tool calls at step 3 is nominal. It
# must be absent from the population AND present in the denominator.
eq "an in-loop line at chars=0 is not in the population" \
   "" "$(classe_of "$WORK/turns.jsonl" t6)"
eq "…but its trace_id still counts in the denominator" \
   "9" "$(sum_of "$WORK/summary.json" '.total_trace_ids_scanned')"

# ---------------------------------------------------------------------------
printf -- '\n-- 5. N5 — the key is trace_id, never session_id (VC §6)\n'
# ---------------------------------------------------------------------------
# (a) Multi-turn session. Two DIFFERENT trace_ids sharing one session_id, each
#     with its own continuation line, are TWO turns of the class. This is the
#     control that refuses the "at most one per session_id" invariant: an
#     analyzer deduplicating by session would count one — and would under-count
#     exactly the sessions where the class shows up most.
eq "N5a — two trace_ids in one session are two turns (first)" \
   "empty_response" "$(classe_of "$WORK/turns.jsonl" t7)"
eq "N5a — two trace_ids in one session are two turns (second)" \
   "empty_response" "$(classe_of "$WORK/turns.jsonl" t8)"
eq "N5a — and the class count reflects all three" \
   "3" "$(sum_of "$WORK/summary.json" '.by_class.empty_response')"

# (b) Real duplicate (rule 0). Two continuation lines under ONE trace_id are
#     `undetermined` plus a dedicated counter — never two `empty_response`
#     (which would inflate the class 2x on exactly the population of interest),
#     never a silent pick of the first or the last (which would absorb the
#     anomaly into a healthy-looking number).
eq "N5b — a duplicated continuation line is undetermined" \
   "undetermined" "$(classe_of "$WORK/turns.jsonl" t9)"
eq "N5b — and it is counted under its own name" \
   "1" "$(sum_of "$WORK/summary.json" '.duplicate_continuation')"

# (c) Denominator counts DISTINCT trace_ids, not lines. Without it a class
#     count has no base — and it is what the D6 criterion consumes.
eq "N5c — continuation turns are counted per turn, not per line" \
   "8" "$(sum_of "$WORK/summary.json" '.continuation_turns')"

# ---------------------------------------------------------------------------
printf -- '\n-- 6. the model is REPORTED, never supposed (AC3)\n'
# ---------------------------------------------------------------------------
# mika#1950's discipline, and the one that made the axis-A premise fall. The
# repo cannot say which model mika-dev runs; only the data can.
eq "every model present in the corpus is reported" \
   "anthropic/claude-sonnet-4-6 z-ai/glm-5.2 z-ai/glm-5.3" \
   "$(sum_of "$WORK/summary.json" '.models_seen | join(" ")')"
eq "the GLM half is counted apart" \
   "7" "$(sum_of "$WORK/summary.json" '.glm_continuation_turns')"

# The README's isolation query for the `strip_internal_tags` false positive must
# actually select something — a documented query nobody can run is a note.
eq "the README query isolating the strip_internal_tags population selects" \
   "t1" "$(jq -r 'select(.classe == "empty_response" and .stop_reason == "EndTurn" and .output_tokens > 0) | .trace_id' \
            "$WORK/turns.jsonl" | head -1)"

# ---------------------------------------------------------------------------
printf -- '\n-- 7. fail-safe: an unreadable signal is never a satisfied term\n'
# ---------------------------------------------------------------------------
eq "a non-JSON line is counted, not classified" \
   "1" "$(sum_of "$WORK/summary.json" '.unparseable_lines')"
eq "an unreadable chars value is undetermined, never empty_response" \
   "undetermined" "$(classe_of "$WORK/wire.jsonl" w3)"
eq "an ABSENT chars field is undetermined (the pre-deploy population)" \
   "undetermined" "$(classe_of "$WORK/wire.jsonl" w4)"
# Graceful degradation: a bare JSON number is accepted too, so a future
# formatter change degrades to correctness rather than to silence.
eq "a bare JSON number is read (formatter-change tolerance, 0)" \
   "empty_response" "$(classe_of "$WORK/wire.jsonl" w1)"
eq "a bare JSON number is read (formatter-change tolerance, >0)" \
   "produced" "$(classe_of "$WORK/wire.jsonl" w2)"

# ---------------------------------------------------------------------------
printf -- '\n-- 8. the --agent filter, and read-only-ness\n'
# ---------------------------------------------------------------------------
eq "a line of another agent is excluded entirely" \
   "" "$(classe_of "$WORK/turns.jsonl" tq1)"

cp "$CORPUS" "$WORK/corpus.before"
MEASURE --log-file "$CORPUS" --agent mika-dev --out "$WORK/ro.jsonl" --summary "$WORK/ro.json" >/dev/null 2>&1
if cmp -s "$CORPUS" "$WORK/corpus.before"; then
    ok "the corpus is not modified (read-only)"
else
    ko "the corpus is not modified (read-only)" "byte-identical" "changed"
fi

# ---------------------------------------------------------------------------
printf -- '\n-- 9. "nothing measured" exits 2, never 0\n'
# ---------------------------------------------------------------------------
# A silently inert scan reads exactly like a scan that found nothing to report,
# and that ambiguity is its own defect class (mika#2205). Zero continuation
# turns on a READ corpus is a RESULT (exit 0); an unread corpus is not.
MEASURE --log-file "$WORK/does-not-exist.jsonl" >/dev/null 2>&1
eq "an absent log file exits 2" "2" "$?"

MEASURE --log-file "$CORPUS" --agent no-such-agent >/dev/null 2>&1
eq "an --agent filter matching nothing exits 2" "2" "$?"

# ---------------------------------------------------------------------------
printf -- '\n-- 10. the D6 criterion is a READOUT, never a verdict\n'
# ---------------------------------------------------------------------------
# The script reports the two numbers; the halt that reads them lives in
# docs/eval/mika-1910/README.md §6. A script that decided here would let a
# count taken too early read as "class absent" — the most dangerous false green
# of this work.
eq "the criterion carries its declared day floor" \
   "14" "$(sum_of "$WORK/summary.json" '.d6_criterion.days_required')"
eq "the criterion carries its derived turn floor" \
   "20" "$(sum_of "$WORK/summary.json" '.d6_criterion.glm_continuation_turns_required')"
eq "a one-day corpus of 7 GLM turns does not meet it" \
   "false" "$(sum_of "$WORK/summary.json" '.d6_criterion.met')"

# ---------------------------------------------------------------------------
printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
