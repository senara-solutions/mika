#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/measure-pilot-cycle-emptiness (mika#1950).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# THE ONE ASSERTION THIS FILE EXISTS FOR is that the *constant* is never counted
# as the *event*. 2349 of the 2352 sessions in the corpus carry the line
#
#     [guardrails] maxTurns=200 stallThreshold=5 emptyResponseThreshold=5 …
#
# so a script that greps `empty` reports ~2349 "silent sessions" — a number that
# is wrong, plausible, and carried by the authority of a measurement. That is the
# exact defect class mika#1950 exists not to reproduce, and it is the reason the
# anchor is derived from the producer rather than reverse-engineered from a log.
#
# THE ASYMMETRY. A negative-only battery is satisfied by a script that never says
# `true`, and a positive-only battery by one that always does. So every leurre
# below is paired with the literal producer line, copied byte-for-byte from
# claude-pilot `ui.py::log_guardrail` including its ANSI CSI sequences. Neither
# half is sufficient; both are required.
#
# THE THREE LEURRES ARE MEASURED, NOT IMAGINED (plan F1). Each one was counted on
# the real corpus on 2026-09-20:
#   - `emptyResponseThreshold=5`  — the config line, 2349 sessions
#   - `EmptyResponse`             — case variant of the same constant
#   - `empty_response_result()`   — a Rust symbol read or edited *by* a pilot,
#                                   i.e. session CONTENT, 3 sessions
# A frontier drawn around invented leurres would prove nothing about this corpus.
#
# THE FIXTURES ARE SYNTHETIC AND THAT IS DELIBERATE. They are not sampled from
# /var/log/claude-pilot: a suite that reads the live corpus would change verdict
# as the corpus grows, and could not run on a machine that has never dispatched.
# Fidelity comes from the producer — every marker below is the literal f-string
# of claude_pilot/ui.py — not from provenance.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MEASURE_PATH="$REPO_ROOT/scripts/measure-pilot-cycle-emptiness"

# Invoked through `bash` rather than by path, deliberately. The exec bit is a
# property of the *committed tree*, not of whatever checkout this suite runs in
# — a worktree restored without it would turn every assertion below into the
# same opaque "exited non-zero", hiding twenty real results behind one
# permission. The bit is therefore asserted once, explicitly, against the git
# index (section 0), where it is actually contracted.
MEASURE() { bash "$MEASURE_PATH" "$@"; }

PASS=0
FAIL=0

TMPROOT="$(mktemp -d)"
cleanup() { rm -rf "$TMPROOT"; }
trap cleanup EXIT

LOGDIR="$TMPROOT/logs"
mkdir -p "$LOGDIR"

# -- ANSI, written once ------------------------------------------------------
#
# The producer colours every marker. A fixture set written in plain text would
# pass against a script that never strips CSI, and the live corpus would then
# match nothing. So the escapes are real here.

E=$'\033'
DIM="$E[2m"
RESET="$E[0m"
BOLD="$E[1m"
ORANGE="$E[38;5;208m"
RED="$E[31m"

ok() { PASS=$((PASS + 1)); printf 'ok   — %s\n' "$1"; }
ko() { FAIL=$((FAIL + 1)); printf 'FAIL — %s\n' "$1"; }

# assert_field <fixture-basename> <jq-filter> <expected> <label>
assert_field() {
    local task="$1" filter="$2" expected="$3" label="$4"
    local out actual
    out="$(MEASURE --log-dir "$LOGDIR" 2>/dev/null)" || {
        ko "$label (script exited non-zero)"
        return
    }
    actual="$(printf '%s\n' "$out" \
        | jq -r --arg t "$task" "select(.task_id == \$t) | $filter" 2>/dev/null)"
    if [[ "$actual" == "$expected" ]]; then
        ok "$label"
    else
        ko "$label — expected '$expected', got '$actual'"
    fi
}

reset_logs() { rm -f "$LOGDIR"/*.stderr; }

# ui.py:27-29. `task_str` is "" when task_id is None, so BOTH forms below are
# real and both are in the corpus. The short form alone was the fixture this
# suite first shipped with, and it was green while the script rendered all 2353
# real sessions `undetermined` — a frontier drawn from a truncated observation
# instead of from the producer, i.e. plan F1's defect committed by its own test.
init_line() { printf '%s[init]%s Session %s, model %s\n' "$DIM" "$RESET" "${1:0:8}" "$2"; }
init_line_with_task() {
    printf '%s[init]%s Session %s, model %s, task %s\n' "$DIM" "$RESET" "${1:0:8}" "$2" "$1"
}

# =============================================================================
# 0. The script is shipped runnable
# =============================================================================
#
# Asserted against the git index rather than the filesystem: the index is what
# a fresh clone receives. README §4 tells an operator to run the script by path,
# and a 100644 blob would make that instruction false on every machine but the
# author's.

MODE="$(cd "$REPO_ROOT" && git ls-files -s scripts/measure-pilot-cycle-emptiness | awk '{print $1}')"
if [[ "$MODE" == "100755" ]]; then
    ok 'the script is committed executable (100755)'
else
    ko "the script is committed as ${MODE:-<untracked>}, not 100755"
fi

if [[ "$(head -n1 "$MEASURE_PATH")" == '#!/usr/bin/env bash' ]]; then
    ok 'the script carries a bash shebang'
else
    ko 'the script does not carry the expected bash shebang'
fi

# =============================================================================
# 1. The event, and the three leurres that are not it
# =============================================================================

# --- positive control: the literal producer line ----------------------------
#
# claude_pilot/ui.py:113
#   _log(f"\n{ORANGE}[guardrail]{RESET} {BOLD}{type_}{RESET}: {detail}")
# with type_="empty_response" from guardrails.py:845.

reset_logs
{
    init_line "aaaaaaaa-0000-0000-0000-000000000001" "z-ai/glm-5.2"
    printf '\n%s[guardrail]%s %sempty_response%s: 5 consecutive trivial responses (<10 chars)\n' \
        "$ORANGE" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/aaaaaaaa-0000-0000-0000-000000000001.stderr"
assert_field "aaaaaaaa-0000-0000-0000-000000000001" '.empty_event' 'true' \
    'positive control — the literal producer line sets empty_event'
assert_field "aaaaaaaa-0000-0000-0000-000000000001" '.guardrail_aborts | join(",")' 'empty_response' \
    'positive control — the abort type is captured'

# --- leurre 1: the config constant (2349 sessions) --------------------------

reset_logs
{
    init_line "bbbbbbbb-0000-0000-0000-000000000002" "claude-opus-5"
    printf '%s[guardrails]%s maxTurns=200 stallThreshold=5 emptyResponseThreshold=5 idleTimeout=300.0s\n' \
        "$E[2m" "$RESET"
} >"$LOGDIR/bbbbbbbb-0000-0000-0000-000000000002.stderr"
assert_field "bbbbbbbb-0000-0000-0000-000000000002" '.empty_event' 'false' \
    'leurre 1 — emptyResponseThreshold=5 is the constant, not the event'
assert_field "bbbbbbbb-0000-0000-0000-000000000002" '.guardrail_aborts | length' '0' \
    'leurre 1 — the [guardrails] config line is not an abort'

# --- leurre 2: case variant of the same constant ----------------------------

reset_logs
{
    init_line "cccccccc-0000-0000-0000-000000000003" "claude-opus-5"
    printf '%s[tool]%s %sRead%s: src/config.rs — EmptyResponse threshold documented here\n' \
        "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/cccccccc-0000-0000-0000-000000000003.stderr"
assert_field "cccccccc-0000-0000-0000-000000000003" '.empty_event' 'false' \
    'leurre 2 — EmptyResponse (case variant) is not the event'

# --- leurre 3: a Rust symbol in session CONTENT (3 sessions) ----------------

reset_logs
{
    init_line "dddddddd-0000-0000-0000-000000000004" "claude-opus-5"
    printf '%s[tool]%s %sEdit%s: crates/mika-agent/src/llm.rs — fn empty_response_result() -> Result<()>\n' \
        "$DIM" "$RESET" "$BOLD" "$RESET"
    printf '%sempty_response_result() is called from two sites%s\n' "$DIM" "$RESET"
} >"$LOGDIR/dddddddd-0000-0000-0000-000000000004.stderr"
assert_field "dddddddd-0000-0000-0000-000000000004" '.empty_event' 'false' \
    'leurre 3 — empty_response_result() in session content is not the event'

# --- the anchor is not satisfied mid-line -----------------------------------
#
# A pilot that reads THIS repository can print the anchor itself as content.
# Without the line anchor the script would count its own documentation.

reset_logs
{
    init_line "eeeeeeee-0000-0000-0000-000000000005" "claude-opus-5"
    printf '%s[tool]%s %sRead%s: docs/eval/mika-1950/README.md — the anchor is `[guardrail] empty_response:` after stripping\n' \
        "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/eeeeeeee-0000-0000-0000-000000000005.stderr"
assert_field "eeeeeeee-0000-0000-0000-000000000005" '.empty_event' 'false' \
    'the anchor is line-anchored — quoted in content it does not count'

# =============================================================================
# 2. Verdict anti-vacuity, in all three directions
# =============================================================================

# --- produced ---------------------------------------------------------------

reset_logs
{
    init_line "11111111-0000-0000-0000-000000000011" "claude-opus-5"
    for _ in $(seq 33); do
        printf '%s[tool:request]%s %sBash%s: cargo test -p mika-agent\n' "$DIM" "$RESET" "$BOLD" "$RESET"
    done
} >"$LOGDIR/11111111-0000-0000-0000-000000000011.stderr"
assert_field "11111111-0000-0000-0000-000000000011" '.verdict' 'produced' \
    'anti-vacuity — 33 tool calls render produced'
assert_field "11111111-0000-0000-0000-000000000011" '.tool_calls' '33' \
    'anti-vacuity — the 33 are counted, not merely detected'

# --- empty ------------------------------------------------------------------

reset_logs
{
    init_line "22222222-0000-0000-0000-000000000022" "z-ai/glm-5.2"
    printf '%s[done]%s Success | 1 turns | $0.01 | 3s\n' "$E[32m" "$RESET"
} >"$LOGDIR/22222222-0000-0000-0000-000000000022.stderr"
assert_field "22222222-0000-0000-0000-000000000022" '.verdict' 'empty' \
    'anti-vacuity — a started session with zero tool calls renders empty'

# --- undetermined: truncated (no [init] line) -------------------------------
#
# This is the fail-safe of plan D2/U1. A log whose header never arrived is a log
# we cannot read; it is never `produced` and never `empty`.

reset_logs
printf '%s[tool:request]%s %sBash%s: ls\n' "$DIM" "$RESET" "$BOLD" "$RESET" \
    >"$LOGDIR/33333333-0000-0000-0000-000000000033.stderr"
assert_field "33333333-0000-0000-0000-000000000033" '.verdict' 'undetermined' \
    'fail-safe — a truncated log (no [init]) renders undetermined'
assert_field "33333333-0000-0000-0000-000000000033" '.model' 'null' \
    'fail-safe — an unreadable model is null, never a guess'

# --- undetermined: empty file -----------------------------------------------

reset_logs
: >"$LOGDIR/44444444-0000-0000-0000-000000000044.stderr"
assert_field "44444444-0000-0000-0000-000000000044" '.verdict' 'undetermined' \
    'fail-safe — an empty file renders undetermined'

# =============================================================================
# 3. The discriminants: model (R5) and policy:deny (R6)
# =============================================================================

reset_logs
{
    init_line "55555555-0000-0000-0000-000000000055" "claude-opus-4-8[1m]"
    printf '%s[policy:deny]%s %sBash%s: cd /elsewhere\n' "$RED" "$RESET" "$BOLD" "$RESET"
    printf '%s[policy:deny]%s %sWrite%s: /tmp/pr-body.md\n' "$RED" "$RESET" "$BOLD" "$RESET"
    printf '\n%s[error]%s error_during_execution: [ede_diagnostic] stop_reason=tool_use\n' "$RED" "$RESET"
} >"$LOGDIR/55555555-0000-0000-0000-000000000055.stderr"
assert_field "55555555-0000-0000-0000-000000000055" '.model' 'claude-opus-4-8[1m]' \
    'R5 discriminant — the model is read from [init], bracket suffix included'
assert_field "55555555-0000-0000-0000-000000000055" '.policy_denies' '2' \
    'R6 discriminant — policy:deny lines are counted'
assert_field "55555555-0000-0000-0000-000000000055" '.error_during_execution' 'true' \
    'R6 discriminant — error_during_execution is detected'

# The form that actually dominates the corpus: `, task <uuid>` trails the model.
reset_logs
{
    init_line_with_task "5c1e3486-c69f-452e-9938-3dda725fc865" "claude-opus-5[1m]"
    printf '%s[tool]%s %sBash%s: true\n' "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/5c1e3486-c69f-452e-9938-3dda725fc865.stderr"
assert_field "5c1e3486-c69f-452e-9938-3dda725fc865" '.model' 'claude-opus-5[1m]' \
    'R5 discriminant — the trailing ", task <uuid>" is not swallowed into the model'
assert_field "5c1e3486-c69f-452e-9938-3dda725fc865" '.verdict' 'produced' \
    'R5 discriminant — the dominant [init] form does not fail-safe to undetermined'

# =============================================================================
# 4. An unknown guardrail type is recorded as-is, never binned
# =============================================================================
#
# `error_max_turns` is NOT in guardrails.py::_abort's Literal[...]: it reaches
# log_guardrail from agent.py:842 as an SDK termination subtype. A script that
# only knew _abort's list would drop it silently — and the plan's own v1 misread
# exactly those sessions as empty_response.

reset_logs
{
    init_line "66666666-0000-0000-0000-000000000066" "claude-opus-5"
    printf '\n%s[guardrail]%s %serror_max_turns%s: SDK limit reached after 200 turns\n' \
        "$ORANGE" "$RESET" "$BOLD" "$RESET"
    printf '\n%s[guardrail]%s %ssome_future_type%s: a type this script has never seen\n' \
        "$ORANGE" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/66666666-0000-0000-0000-000000000066.stderr"
assert_field "66666666-0000-0000-0000-000000000066" '.guardrail_aborts | join(",")' \
    'error_max_turns,some_future_type' \
    'unknown guardrail types are recorded verbatim, not binned or dropped'
assert_field "66666666-0000-0000-0000-000000000066" '.empty_event' 'false' \
    'error_max_turns is NOT empty_response — the v1 misreading, pinned'

# =============================================================================
# 4b. The line stamp (cpp#168) — a dated format change in a third-party repo
# =============================================================================
#
# claude-pilot logger.py::_LineStamper prefixes EVERY line with
# `[YYYY-MM-DDTHH:MM:SS.mmmZ] `, applied to the already-coloured message. Both
# regimes are in the corpus: unstamped before ~2026-09-10, stamped after.
#
# This is not a hypothetical. Shipped without step 2 of `strip`, the script
# rendered 231 real sessions — the whole of September — `undetermined`, because
# their `[init]` header no longer started the line. The fail-safe held (no false
# value was emitted) but the newest and most relevant slice of the corpus went
# dark. That is the frontier this section pins.

STAMP='[2026-09-18T21:42:41.491Z] '

reset_logs
{
    printf '%s%s[init]%s Session aaaabbbb, model claude-opus-5[1m], task aaaabbbb-0000-0000-0000-0000000000aa\n' \
        "$STAMP" "$DIM" "$RESET"
    printf '%s%s[tool:request]%s %sBash%s: cargo test\n' "$STAMP" "$DIM" "$RESET" "$BOLD" "$RESET"
    printf '%s%s[policy:deny]%s %sWrite%s: /tmp/x.md\n' "$STAMP" "$RED" "$RESET" "$BOLD" "$RESET"
    printf '%s\n' "$STAMP"
    printf '%s%s[guardrail]%s %sempty_response%s: 5 consecutive trivial responses (<10 chars)\n' \
        "$STAMP" "$ORANGE" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/aaaabbbb-0000-0000-0000-0000000000aa.stderr"
assert_field "aaaabbbb-0000-0000-0000-0000000000aa" '.model' 'claude-opus-5[1m]' \
    'cpp#168 stamp — the model is still read through the line stamp'
assert_field "aaaabbbb-0000-0000-0000-0000000000aa" '.verdict' 'produced' \
    'cpp#168 stamp — a stamped working session is not lost to undetermined'
assert_field "aaaabbbb-0000-0000-0000-0000000000aa" '.tool_calls' '1' \
    'cpp#168 stamp — tool calls are still counted'
assert_field "aaaabbbb-0000-0000-0000-0000000000aa" '.policy_denies' '1' \
    'cpp#168 stamp — policy:deny is still counted'
assert_field "aaaabbbb-0000-0000-0000-0000000000aa" '.empty_event' 'true' \
    'cpp#168 stamp — the event anchor still holds through the stamp'

# And the stamp-stripping must not become a way in for content. A bracketed
# token that merely looks date-ish is not a stamp.
reset_logs
{
    init_line "bbbbaaaa-0000-0000-0000-0000000000bb" "claude-opus-5"
    printf '%s[tool]%s %sRead%s: log.txt — [2026-09-18] [guardrail] empty_response: quoted in a file\n' \
        "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/bbbbaaaa-0000-0000-0000-0000000000bb.stderr"
assert_field "bbbbaaaa-0000-0000-0000-0000000000bb" '.empty_event' 'false' \
    'cpp#168 stamp — a date-ish bracketed token is not a stamp and opens no door'

# =============================================================================
# 5. Read-only, and it is asserted rather than asserted-in-prose
# =============================================================================

reset_logs
{
    init_line "77777777-0000-0000-0000-000000000077" "claude-opus-5"
    printf '%s[tool]%s %sRead%s: a.rs\n' "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/77777777-0000-0000-0000-000000000077.stderr"

BEFORE="$(find "$LOGDIR" -type f -printf '%p %s\n' | sort)"
MEASURE --log-dir "$LOGDIR" >/dev/null 2>&1
AFTER="$(find "$LOGDIR" -type f -printf '%p %s\n' | sort)"
if [[ "$BEFORE" == "$AFTER" ]]; then
    ok 'read-only — the log directory is byte-identical after a run'
else
    ko 'read-only — the log directory changed'
fi

# =============================================================================
# 6. The output is machine-decidable (AC6) and self-describing
# =============================================================================

reset_logs
{
    init_line "88888888-0000-0000-0000-000000000088" "z-ai/glm-5.3"
    printf '%s[tool]%s %sBash%s: true\n' "$DIM" "$RESET" "$BOLD" "$RESET"
} >"$LOGDIR/88888888-0000-0000-0000-000000000088.stderr"

if MEASURE --log-dir "$LOGDIR" 2>/dev/null | jq -e 'has("task_id") and has("date") and has("model") and has("tool_calls") and has("policy_denies") and has("empty_event") and has("guardrail_aborts") and has("error_during_execution") and has("verdict")' >/dev/null; then
    ok 'every line carries the nine contracted fields'
else
    ko 'a contracted field is missing from the output'
fi

if MEASURE --log-dir "$LOGDIR" 2>/dev/null | jq -e '.date | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}$")' >/dev/null; then
    ok 'date is an ISO calendar day (plan F2: calendar, not regime)'
else
    ko 'date is not an ISO calendar day'
fi

# --- --out writes where it is told, and stdout stays clean ------------------

OUTFILE="$TMPROOT/measurement.jsonl"
MEASURE --log-dir "$LOGDIR" --out "$OUTFILE" >/dev/null 2>&1
if [[ -s "$OUTFILE" ]] && jq -e . "$OUTFILE" >/dev/null 2>&1; then
    ok '--out writes valid JSONL to the named file'
else
    ko '--out did not produce valid JSONL'
fi

# --- --limit takes the N most recent ----------------------------------------

reset_logs
for i in 1 2 3 4 5; do
    init_line "9999999$i-0000-0000-0000-00000000009$i" "claude-opus-5" \
        >"$LOGDIR/9999999$i-0000-0000-0000-00000000009$i.stderr"
    touch -d "2026-09-0$i 12:00:00" "$LOGDIR/9999999$i-0000-0000-0000-00000000009$i.stderr"
done
COUNT="$(MEASURE --log-dir "$LOGDIR" --limit 2 2>/dev/null | wc -l)"
if [[ "$COUNT" == "2" ]]; then
    ok '--limit N returns exactly N sessions'
else
    ko "--limit N returned $COUNT sessions, expected 2"
fi
NEWEST="$(MEASURE --log-dir "$LOGDIR" --limit 1 2>/dev/null | jq -r '.date')"
if [[ "$NEWEST" == "2026-09-05" ]]; then
    ok '--limit takes the most RECENT sessions (the 102/120 cross-check needs this)'
else
    ko "--limit took the wrong end: got $NEWEST, expected 2026-09-05"
fi

# =============================================================================
# 7. An absent log directory is reported, never silently empty
# =============================================================================
#
# mika#2205's rule: a silently inert scan reads exactly like a scan that found
# nothing to do. An exit 0 with no output here would make "no corpus on this
# host" indistinguishable from "no session was ever empty".

if MEASURE --log-dir "$TMPROOT/does-not-exist" >/dev/null 2>&1; then
    ko 'an absent log directory exited 0 — a silent scan reads as a healthy one'
else
    ok 'an absent log directory exits non-zero rather than reading as empty'
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
