#!/usr/bin/env bash
#
# provision.sh — RT-005 physics pilot, brick 2/5.
#
# Idempotently assembles two derived agent dirs under ~/.mika/agents/:
#   mika-dev-confidence-high  (prior 0.95 on peer_b reliability)
#   mika-dev-confidence-low   (prior 0.55 on peer_b reliability)
#
# The two agents are byte-identical except for the confidence prior block
# in soul.md (and the display `name`/`emoji`, a documented metadata
# exception — see README.md). No engine changes, no `mika agents create`
# dependency: `config.toml` presence is the only key `mika ask` needs to
# resolve an agent (crates/mika-cli/src/init.rs, ensure_initialized_for_agent).
#
# Re-running overwrites the three generated files deterministically.
# Teardown: rm -rf ~/.mika/agents/mika-dev-confidence-{high,low}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED="$SCRIPT_DIR/shared"
AGENTS_ROOT="${MIKA_AGENTS_ROOT:-$HOME/.mika/agents}"

emoji_for() { case "$1" in high) printf '▲' ;; low) printf '▼' ;; esac; }
name_for()  { case "$1" in high) printf 'Mika Dev Confidence High' ;; low) printf 'Mika Dev Confidence Low' ;; esac; }

assemble() {
  local level="$1"
  local home="$AGENTS_ROOT/mika-dev-confidence-$level"
  mkdir -p "$home"

  # config.toml — identical for both agents.
  cp "$SHARED/config.toml" "$home/config.toml"

  # identity.toml — substitute per-level display name/emoji; rest identical.
  sed -e "s/__EMOJI__/$(emoji_for "$level")/" \
      -e "s/__NAME__/$(name_for "$level")/" \
      "$SHARED/identity.toml" > "$home/identity.toml"

  # soul.md — shared base + the ONE differing confidence prior block.
  cat "$SHARED/soul-base.md" "$SCRIPT_DIR/prior-$level.md" > "$home/soul.md"

  echo "  provisioned $home"
}

echo "Provisioning confidence agents under $AGENTS_ROOT ..."
assemble high
assemble low

# --- AC1 self-assertion: the two assembled soul.md files must differ ONLY
# inside the confidence prior (every changed line mentions 0.95 or 0.55). ---
HIGH_SOUL="$AGENTS_ROOT/mika-dev-confidence-high/soul.md"
LOW_SOUL="$AGENTS_ROOT/mika-dev-confidence-low/soul.md"

diff_out="$(diff "$HIGH_SOUL" "$LOW_SOUL" || true)"
changed="$(printf '%s\n' "$diff_out" | grep -E '^[<>]' || true)"

if [ -z "$changed" ]; then
  echo "AC1 FAIL: the two soul.md files are identical — the prior block did not differ." >&2
  exit 1
fi

offending="$(printf '%s\n' "$changed" | grep -Ev '0\.(95|55)' || true)"
if [ -n "$offending" ]; then
  echo "AC1 FAIL: soul.md differs OUTSIDE the confidence prior block:" >&2
  printf '%s\n' "$offending" >&2
  exit 1
fi

echo "AC1 OK: soul.md files differ only in the confidence prior (0.95 vs 0.55)."
echo
echo "Invoke the agents (AC3):"
echo "  mika ask --agent mika-dev-confidence-high \"<probe>\""
echo "  mika ask --agent mika-dev-confidence-low  \"<probe>\""
