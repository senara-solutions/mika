#!/usr/bin/env bash
# Reject `tokio::select!` patterns inside the agent loop body.
#
# Background: mika#848 fixes a deadline-during-LLM-call bug by checking the
# turn deadline at the top of each `run_loop` iteration in
# `crates/mika-agent/src/agent.rs`. The guarantee that this check fires before
# any new step starts depends on the loop body NOT shadowing iteration semantics
# with `tokio::select!`. A `select!` inside the loop could:
#
#   - Interleave the deadline check with other futures, missing the next-step
#     boundary the architect specified.
#   - Drop in-flight DB writes (tool-call summaries, message saves) when one
#     of the select branches resolves first — re-introducing the silent-drop
#     failure mode this fix removed.
#
# If you have a legitimate need for `tokio::select!` inside `run_loop`, that's a
# scope item that requires re-reading the plan at
# `docs/plans/2026-04-27-010-fix-agent-total-timeout-cancels-in-flight-llm-plan.md`
# and the related solution doc, then making a deliberate change to the deadline
# enforcement model — not a quiet addition.

set -euo pipefail

AGENT_FILE="crates/mika-agent/src/agent.rs"

if [ ! -f "$AGENT_FILE" ]; then
    echo "error: $AGENT_FILE not found (run from repo root)" >&2
    exit 2
fi

# Locate the line range for `async fn run_loop` body. The function signature
# spans multiple lines; we anchor on the literal `async fn run_loop(` and grab
# from there to the next top-level `}` (depth tracked via brace counting).
start=$(grep -n '^async fn run_loop(' "$AGENT_FILE" | head -1 | cut -d: -f1 || true)
if [ -z "${start:-}" ]; then
    echo "error: could not locate 'async fn run_loop(' in $AGENT_FILE" >&2
    exit 2
fi

# awk: starting at $start, track brace depth from the first `{` on the signature
# line forward; emit the line range from start to the matching `}`.
end=$(awk -v s="$start" '
    NR < s { next }
    {
        for (i = 1; i <= length($0); i++) {
            c = substr($0, i, 1)
            if (c == "{") {
                depth++
                seen = 1
            } else if (c == "}") {
                depth--
                if (seen && depth == 0) {
                    print NR
                    exit
                }
            }
        }
    }
' "$AGENT_FILE")

if [ -z "${end:-}" ]; then
    echo "error: could not find closing brace for run_loop body" >&2
    exit 2
fi

# Search for tokio::select! within the body. Reject any match.
hits=$(sed -n "${start},${end}p" "$AGENT_FILE" | grep -n 'tokio::select!' || true)

if [ -n "$hits" ]; then
    echo "error: 'tokio::select!' found inside run_loop body (lines relative to function start):" >&2
    echo "$hits" >&2
    echo "" >&2
    echo "See docs/plans/2026-04-27-010-fix-agent-total-timeout-cancels-in-flight-llm-plan.md" >&2
    echo "and docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md for why this is rejected." >&2
    exit 1
fi

echo "ok: no tokio::select! inside run_loop body"
