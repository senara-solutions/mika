#!/usr/bin/env bash
# CI lint: a sandboxed handler does not read a `MIKA_*` variable (mika#2532 T4).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE RULE, IN ONE SENTENCE
#
# No `${MIKA_…}` parameter expansion may appear in `skills/bundled/*/handlers/*.sh`.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY IT IS A CLASS AND NOT A TYPO
#
# Every one of those handlers is spawned by `spawn_long_running_exec`, which
# calls `sandboxed_pilot_env`: `env_clear()` followed by a copy of a POSITIVE
# allowlist whose very first test returns false for any key starting with
# `MIKA_`, reinforced by a `debug_assert`. So a `${MIKA_X:-default}` in a
# handler ALWAYS takes the right-hand branch — it is a dead branch, and it gives
# the handler the appearance of honouring an operator setting it cannot read.
#
# Measured in mika#2532: four handlers read `${MIKA_PLATFORM_DIR:-…}` on six
# lines, while `qa-review`'s prompt prescribed a worktree path composed from the
# same variable. Nothing expanded it on either side.
#
# The repair is never to widen the sandbox allowlist — that would pierce an
# anti-secret-leak guard for a path convenience. It is to RELAY the value
# explicitly after the sandbox, under an unprefixed name, the way
# `inject_pilot_transcript_env`, `inject_dispatch_worktree_env`,
# `inject_rescue_verify_env`, `inject_arch_ask_retry_env`,
# `inject_pilot_dispatch_env` and `inject_platform_dir_env` already do.
#
# WHEN THIS FIRES ON A NEW SITE, ADD A RELAY. Do not add a line to an allowlist
# — there is none, deliberately (mika#2201: a handler that needs a `MIKA_*`
# needs a relay, not a dispensation).
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY A SOURCE SCAN AND NOT A TEST
#
# A dead branch makes NO decision wrong. Every behavioural assertion stays green
# while the setting silently stops working — which is precisely how six sites
# survived unnoticed. Only a scan can see it.
#
# ─────────────────────────────────────────────────────────────────────────────
# POPULATION, AND THE RESIDUE THIS SCAN DELIBERATELY DOES NOT COVER
#
# `skills/bundled/*/handlers/*.sh` — the set mika#2532 enumerated and brought to
# zero. `_shared/dispatch-lib.sh` is OUT, and that is a decision rather than an
# oversight: it carries four reads of the same class, of which three are NOT
# relayed today —
#
#     MIKA_PILOT_SANDBOX          (containment kill-switch; dead = fail-closed)
#     MIKA_PILOT_EGRESS_LOG_DIR   (log sink; its default is the right one)
#     MIKA_HOME                   (its default is the right one)
#     MIKA_PLATFORM_DIR           (relayed since mika#2532, still read prefixed)
#
# Each needs its own relay-or-delete judgement, and the fourth changes sub-repo
# resolution on the loop's central dispatch path for every dispatch — which
# deserves its own verification rather than a ride-along on a diagnostic ticket.
# FOLLOW-UP. Naming them here is what stops them being rediscovered as new.
#
# Usage: check-sandboxed-handler-env.sh [scan-root]
#   The optional scan root lets the anti-vacuity harness
#   (scripts/test-check-sandboxed-handler-env.sh) point the scan at a fixture
#   tree, on the mika#2103 model.
#
# Exit 0 if clean, 1 with actionable errors otherwise.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCAN_ROOT="${1:-$REPO_ROOT}"
HANDLER_GLOB_ROOT="$SCAN_ROOT/skills/bundled"

if [[ ! -d "$HANDLER_GLOB_ROOT" ]]; then
    echo "check-sandboxed-handler-env: no skills/bundled under $SCAN_ROOT" >&2
    exit 1
fi

mapfile -t HANDLERS < <(find "$HANDLER_GLOB_ROOT" -mindepth 3 -maxdepth 3 \
    -path '*/handlers/*.sh' -type f | sort)

# Anti-vacuity: a scan with an empty population reads exactly like a clean tree
# (mika#2205). Refuse rather than pass.
if [[ ${#HANDLERS[@]} -eq 0 ]]; then
    echo "check-sandboxed-handler-env: FAIL — no handler found under $HANDLER_GLOB_ROOT." >&2
    echo "  The scan has nothing to look at, which is not the same as a clean tree." >&2
    echo "  A directory rename or an extension change would produce exactly this." >&2
    exit 1
fi

OFFENDERS=()

for handler in "${HANDLERS[@]}"; do
    rel="${handler#"$SCAN_ROOT"/}"
    # Comments are stripped BEFORE the examination (the mika#2496 motif): a
    # handler must stay free to explain in prose why it no longer reads the
    # variable. Without this the four repaired handlers would be permanently
    # red on their own explanation, and the scan would get disarmed.
    while IFS= read -r hit; do
        OFFENDERS+=("$rel:$hit")
    done < <(
        grep -n '\${MIKA_' "$handler" 2>/dev/null \
            | grep -v '^[0-9]*:[[:space:]]*#' \
            || true
    )
done

if [[ ${#OFFENDERS[@]} -gt 0 ]]; then
    echo "check-sandboxed-handler-env: FAIL (mika#2532)" >&2
    echo >&2
    echo "  A sandboxed handler reads a MIKA_* variable. That read is a DEAD" >&2
    echo "  BRANCH: sandboxed_pilot_env rebuilds the child environment from a" >&2
    echo "  positive allowlist that refuses every MIKA_* name, so the default" >&2
    echo "  is always taken and the setting silently does nothing." >&2
    echo >&2
    for off in "${OFFENDERS[@]}"; do
        echo "    $off" >&2
    done
    echo >&2
    echo "  RESOLUTION: relay the value from the executor under an unprefixed" >&2
    echo "  name (see inject_platform_dir_env in crates/mika-agent/src/skills/" >&2
    echo "  executor.rs) and read that name here. Do NOT add the variable to" >&2
    echo "  SANDBOX_ENV_CORE_ALLOWLIST — that allowlist is an anti-secret-leak" >&2
    echo "  guard, and a debug_assert exists to refuse exactly that gesture." >&2
    exit 1
fi

echo "check-sandboxed-handler-env: OK — ${#HANDLERS[@]} handlers scanned, 0 dead MIKA_* reads"
