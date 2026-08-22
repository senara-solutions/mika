#!/usr/bin/env bash
# CI lint (mika#1796 — voice testimony lane non-transit invariant, Phase 3):
# the `skills/bundled/voice-livekit-agents/testimony/**` Python subtree may
# NEVER import a cloud STT/TTS SDK or a raw network primitive. Any hit fails
# the build.
#
# Discipline analog: `scripts/verify-egress-uniqueness.sh` (mika#1807 AC4) —
# same shape, same doctrine (construis l'incapacité). Also cite:
# `scripts/check-byte-slices.sh` (#764) and `scripts/check-loop-select.sh`
# (#848) for the structural-not-prompt precedent.
#
# No-op-until-scaffold-lands semantics
# ------------------------------------
# The testimony subtree is created by mika#1787 (LiveKit Agents Python
# scaffold). Until that lands, this script exits 0 as a no-op — the gate
# is armed but has nothing to check. The instant the directory appears,
# the gate becomes enforcing without a separate wiring step; this
# guarantees no interleaving-order bug can slip cloud STT/TTS into
# testimony/** between merges.
#
# Exit codes:
#   0 — clean, or no-op (directory absent)
#   1 — violation(s) found
#
# Escape hatch (per line):
#   Append `# voice-non-transit: safe` and cite the ticket that justifies
#   the exception (e.g., a LOCAL LiveKit control socket that happens to
#   use `httpx` for a loopback endpoint). The escape MUST cite a ticket
#   and MUST NOT be used to justify a real cloud call.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TESTIMONY_ROOT="$REPO_ROOT/skills/bundled/voice-livekit-agents/testimony"

# Cloud-provider import signatures + raw network primitives. Any of these
# appearing under testimony/**/*.py is a discipline violation.
#
# Scope discipline (mirror of egress-uniqueness):
#   - Cloud SDKs / API hosts: real network egress on the testimony path.
#   - Raw network primitives: any HTTP client on the testimony path is
#     suspect — even if it targets a loopback URL, a subsequent config
#     edit or code change could point it at WAN. The type-level Rust
#     surface plus the config validator handle the intentional local
#     endpoints; the CI lint rejects the SDK import class entirely.
BANNED_IMPORTS=(
    "deepgram"                    # cloud STT
    "elevenlabs"                  # cloud TTS
    "azure.cognitiveservices"     # Azure Speech
    "google.cloud.speech"         # Google STT
    "google.cloud.texttospeech"   # Google TTS
    "openai"                      # cloud STT (Whisper API) — LOCAL-ONLY
                                  # Whisper uses `faster_whisper` / `whisper_cpp`
    "boto3"                       # AWS SDK — catches Transcribe / Polly
    "aiohttp"                     # raw network primitive
    "httpx"                       # raw network primitive
    "requests"                    # raw network primitive
    "urllib.request"              # raw network primitive
)

# No-op-until-scaffold-lands (see header comment): the gate is armed but
# has nothing to check until mika#1787 lands the testimony directory.
if [ ! -d "$TESTIMONY_ROOT" ]; then
    echo "voice-non-transit-lint: testimony/ path not present yet (waiting on #1787). No-op."
    exit 0
fi

VIOLATIONS=0
for pattern in "${BANNED_IMPORTS[@]}"; do
    # Escape dots in the pattern for the extended regex.
    escaped="${pattern//./\\.}"
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        echo "ERROR: banned import '$pattern' in testimony path: $line"
        VIOLATIONS=$((VIOLATIONS + 1))
    done < <(grep -rEn "^[[:space:]]*(from|import)[[:space:]]+${escaped}" "$TESTIMONY_ROOT" \
        --include='*.py' \
        | grep -v '# voice-non-transit: safe' \
        || true)
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "::error::testimony lane non-transit invariant violated ($VIOLATIONS hits)."
    echo "The testimony/** subtree MUST import LOCAL-only STT/TTS (faster_whisper, whisper_cpp, piper, silero, coqui)."
    echo "For any legitimate exception (e.g., a LOCAL LiveKit control socket), append '# voice-non-transit: safe' to the line and cite the ticket that justifies it."
    exit 1
fi

echo "voice-non-transit-lint: clean."
