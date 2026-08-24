---
issue: senara-solutions/mika#1796
type: invariant
target: crates/mika-gateway + skills/bundled + CI
milestone: senara-solutions/mika#1785 (Mika Voice — Phase 2 testimony lane)
sibling: senara-solutions/mika#1793 (Whisper local FR variant selection)
authored: 2026-08-22
---

# invariant(voice,p2.5): non-transit build-time gate — cloud STT/TTS interdits sur testimony path, vérifié par construction

## Prime line (WHY — non-negotiable)

**Construis l'incapacité, ne promets pas la retenue.** The testimony lane's
non-transit property must be a build-time invariant, not a runtime toggle.
"Verify by construction" means: on any commit that lands on `main`, the
testimony code path *cannot* reference a cloud STT/TTS provider — the
compiler and CI refuse to produce such a build.

Same doctrine that seats:
- `mika#1807` egress-uniqueness lint (`scripts/verify-egress-uniqueness.sh`)
  — a single authorized substrate for search egress, any other reference
  fails the build.
- `mika#848` `loop-select-lint` — reject `tokio::select!` inside `run_loop`
  by grepping the AST at CI time.
- `mika#764` `byte-slice-lint` — reject unsafe `&str` byte-slicing patterns
  that panic on multi-byte UTF-8.

The pattern is proven and this ticket adopts it verbatim.

## Body-vs-code reconciliation (checkpoint findings)

The issue body and the parent milestone (#1785 § Phases) name two implementation
surfaces. Reconcile before the architect:

| Surface | Body claim | Milestone reality | Resolution in this plan |
|---|---|---|---|
| Rust orchestrator | "Module dans `crates/mika-gateway/`" + "Rust marker types + trait bounds" | Gateway crate hosts LiveKit **room/token/routing** orchestration (not the STT/TTS pipeline itself) | Rust gate = typed API separating conversation vs testimony rooms; cargo-deny ban on cloud STT/TTS crates being pulled into the gateway crate |
| Rust module layout | "Séparation stricte des modules : `voice::conversation::{stt,tts}` (cloud OK) vs `voice::testimony::{stt,tts}` (local only)" | — | Plan supersedes with **type-level lane markers** (`ConversationLane` / `TestimonyLane` + sealed `VoiceLane` trait). Enforcement is stronger than pure namespace separation: the type system rejects illegal wiring regardless of import path (a namespace-only split could be defeated by a single mis-scoped `use`). Body intent ("no conversion possible testimony→conversation") is honored more strongly, not weakened. Filesystem layout still uses per-concern files (`lane.rs`, `provider.rs`, `room.rs`) for clarity. |
| Python pipeline | (implicit, absent from body) | Milestone P1.2 (#1787) wires STT/TTS in a **LiveKit Agents SDK Python scaffold** | Python gate = module boundary (`testimony/` vs `conversation/`, matching body's directory intuition) + custom CI lint that greps for cloud-provider imports inside the testimony path |
| Runtime egress audit (body §Scope #4) | "iptables/nftables rules on gateway box" | Deployment-layer, not build-time | Not in this plan's build-time contract — carved out to a follow-up (see § Out of scope). Body explicitly tags this "défense en profondeur" (defense-in-depth), signalling it as supplementary to the build-time gate this ticket owns. |

**No plan-vs-body divergence in the AC axis** — every AC (1–5) is honored, with
the runtime-egress AC scoped down to "documented boundary + follow-up ticket
filed" (see § AC mapping).

## Scope — what this plan ships

Three defenses composed. Each is a build-time gate; if any fails, `cargo build`,
`make verify-bundled-skills`, or CI fails.

### Phase 1 — Rust orchestrator: typed lane separation

**Where:** `crates/mika-gateway/src/voice/` (new module).

**Types:**

```rust
// Compile-time zero-sized markers — non-convertible
pub struct ConversationLane;
pub struct TestimonyLane;

pub trait VoiceLane: sealed::Sealed {}
impl VoiceLane for ConversationLane {}
impl VoiceLane for TestimonyLane {}

// Sealed trait prevents downstream crates adding new lanes.
mod sealed { pub trait Sealed {} impl Sealed for super::ConversationLane {} impl Sealed for super::TestimonyLane {} }

// STT/TTS provider capabilities are lane-scoped by type.
pub trait CloudStt { const LANE: ConversationLane; /* ... */ }
pub trait LocalStt { const LANE: TestimonyLane; /* ... */ }
pub trait CloudTts { const LANE: ConversationLane; /* ... */ }
pub trait LocalTts { const LANE: TestimonyLane; /* ... */ }

// Room orchestration is generic over the lane and only accepts the matching provider.
pub struct VoiceRoom<L: VoiceLane, S, T> { _lane: PhantomData<L>, stt: S, tts: T, /* ... */ }

impl<S: CloudStt, T: CloudTts> VoiceRoom<ConversationLane, S, T> { /* conversation ctor */ }
impl<S: LocalStt, T: LocalTts> VoiceRoom<TestimonyLane, S, T> { /* testimony ctor */ }
```

**Compile-fail test** (`crates/mika-gateway/tests/voice_lane_compile_fail/`):

Using [`trybuild`](https://docs.rs/trybuild) — the standard pattern for asserting
compile errors. A `.rs` fixture attempts `VoiceRoom::<TestimonyLane, DeepgramStt, ElevenLabsTts>::new(…)`;
the expected `.stderr` names the trait-bound error. `cargo test --test voice_lane_compile_fail`
runs it; a regression that removes the type-level lane separation makes the fixture *compile*,
failing the trybuild assertion.

**Config validation** (`crates/mika-gateway/src/voice/config.rs`):

Startup check rejects any `testimony.stt` or `testimony.tts` config value whose
`endpoint` field resolves outside `127.0.0.1` / `::1` / LAN RFC1918 ranges.
Fails-closed: parse error → gateway refuses to start (not warn+continue).

### Phase 2 — cargo-deny bans in the gateway crate

**Where:** `mika/deny.toml`, `[bans]` section.

Add explicit denies for cloud-STT/TTS crate identifiers **within the gateway crate's
dependency subgraph** (the crates most likely to be pulled in by a testimony-lane
wiring mistake):

```toml
[bans]
multiple-versions = "warn"
wildcards = "allow"
# mika#1796 — non-transit testimony lane: cloud STT/TTS providers may
# never be pulled into mika-gateway. If a legitimate future use adds
# one (conversation lane only), it MUST be pulled into a separate crate
# never touched by the testimony path, and this list updated with a
# comment naming that ticket.
deny = [
  { name = "elevenlabs" },       # cloud TTS — conversation lane only
  { name = "elevenlabs-rs" },
  { name = "deepgram" },         # cloud STT — conversation lane only
  { name = "deepgram-rs" },
  { name = "azure-speech" },
  { name = "google-speech" },
  { name = "aws-sdk-transcribe" },
  { name = "aws-sdk-polly" },
]
```

**Note on scope:** cargo-deny bans apply workspace-wide by default. That's the
desired behavior here — the Rust workspace already treats the gateway crate as
the sole voice-surface owner; if a downstream extraction ever needs cloud STT
in a separate crate, the ticket adds the crate name to an allowlist with a
comment naming the reason.

CI already runs `cargo deny check` as part of the standard security jobs; no
new workflow file needed.

### Phase 3 — Python-side non-transit gate (LiveKit Agents SDK scaffold)

**Where:** the future `skills/bundled/voice-livekit-agents/` (created in #1787),
enforced by a new CI lint script `scripts/verify-voice-non-transit.sh` and a
verify job in `.github/workflows/ci.yml`.

**Module boundary convention (P1.2 lands, this plan enforces):**

```
skills/bundled/voice-livekit-agents/
├── conversation/          # cloud STT/TTS wiring OK
│   ├── stt.py             # imports deepgram, openai, google.cloud.speech, …
│   └── tts.py             # imports elevenlabs, azure.cognitiveservices.speech, …
└── testimony/             # LOCAL only, non-transit
    ├── stt.py             # imports faster_whisper OR whisper_cpp (local)
    └── tts.py             # imports piper, coqui, or silero (local)
```

**Verify script** (`mika/scripts/verify-voice-non-transit.sh`) — modeled verbatim on
`scripts/verify-egress-uniqueness.sh`:

```bash
#!/usr/bin/env bash
# mika#1796 — build-time invariant: the testimony lane's
# skills/bundled/voice-livekit-agents/testimony/** subtree may not import
# any cloud STT/TTS SDK. Any hit fails CI.

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TESTIMONY_ROOT="$REPO_ROOT/skills/bundled/voice-livekit-agents/testimony"

# Cloud-provider import signatures. Any of these appearing under
# testimony/** is a discipline violation.
BANNED_IMPORTS=(
  "deepgram"                  # cloud STT
  "elevenlabs"                # cloud TTS
  "azure.cognitiveservices"   # Azure Speech
  "google.cloud.speech"       # Google STT
  "google.cloud.texttospeech" # Google TTS
  "openai"                    # cloud STT (Whisper API) — LOCAL-ONLY Whisper uses `faster_whisper` / `whisper_cpp`
  "boto3"                     # AWS SDK — catches Transcribe/Polly
  "aiohttp"                   # network primitive; any HTTP client is suspect on testimony path
  "httpx"                     # network primitive
  "requests"                  # network primitive
  "urllib.request"            # network primitive
)

# Directory does not exist yet — the gate is a no-op until P1.2 (#1787) lands
# the LiveKit Agents scaffold. Once it exists, the gate becomes enforcing.
if [ ! -d "$TESTIMONY_ROOT" ]; then
  echo "voice-non-transit-lint: testimony/ path not present yet (waiting on #1787). No-op."
  exit 0
fi

VIOLATIONS=0
for pattern in "${BANNED_IMPORTS[@]}"; do
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    echo "ERROR: banned import '$pattern' in testimony path: $line"
    VIOLATIONS=$((VIOLATIONS + 1))
  done < <(grep -rEn "^\s*(from|import)\s+${pattern//./\\.}" "$TESTIMONY_ROOT" \
    --include='*.py' \
    | grep -v '# voice-non-transit: safe' \
    || true)
done

if [ "$VIOLATIONS" -gt 0 ]; then
  echo ""
  echo "::error::testimony lane non-transit invariant violated ($VIOLATIONS hits)."
  echo "The testimony/** subtree must import LOCAL-only STT/TTS (faster_whisper, whisper_cpp, piper, silero, coqui)."
  echo "For any legitimate exception (e.g. LOCAL LiveKit control socket), append '# voice-non-transit: safe' to the line and cite this ticket."
  exit 1
fi

echo "voice-non-transit-lint: clean."
```

**CI wiring:** add to `mika/.github/workflows/ci.yml` next to `egress-uniqueness-lint`:

```yaml
voice-non-transit-lint:
  name: Voice Non-Transit Lint
  runs-on: ubuntu-22.04
  steps:
    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
    - name: Enforce testimony-lane non-transit (mika#1796)
      run: bash scripts/verify-voice-non-transit.sh
```

**No-op-until-scaffold-lands semantics:** the script is added *now* (this ticket's
job) so the gate is armed before P1.2. It exits 0 when the testimony subtree is
absent — the gate becomes enforcing the moment #1787 creates the directory. This
guarantees no interleaving-order bug can slip cloud STT/TTS into testimony/**
between merges.

### Phase 4 — Doctrine documentation

**Where:** `mika/docs/voice-non-transit-invariant.md` (new).

Contents:
1. **Why the invariant exists** — Prime line quoted verbatim.
2. **How each layer enforces it** — three gates (Rust types, cargo-deny, CI lint)
   with the failure mode each catches.
3. **How to add a new lane provider** — the four-step recipe (add crate to
   deny/allow, extend `SearchUpstream`-analog trait bound, update the CI lint
   BANNED_IMPORTS if the SDK ships a new subpackage, cite the ticket).
4. **How to audit externally** — the three greppable commands a reviewer runs
   to prove the invariant holds on any commit.
5. **The escape hatch** — `# voice-non-transit: safe` line comment (for CI lint),
   `[bans.allow]` (for cargo-deny). Each escape MUST cite a ticket.
6. **Cross-ref to sibling ticket for runtime egress audit** (§ Out of scope below).

## Out of scope (deferred to siblings)

- **AC4 runtime egress audit — deployment-side nftables/iptables rules on the
  gateway box.** This is a deployment/network gate, not a build-time invariant.
  File a follow-up ticket (`voice(p2.5-runtime): nftables egress deny for testimony ports`)
  and reference it from `voice-non-transit-invariant.md` § Related. Runtime
  egress belongs on the mika-cloud repo (Helm/K8s NetworkPolicy for cloud
  deployments) and on the gateway-box provisioning script for self-hosted
  Phase 2 substrate — coordination-scoped, not one repo.
- **LiveKit Agents SDK Python scaffold itself** (`skills/bundled/voice-livekit-agents/`).
  That's #1787's deliverable. This ticket ships the *gate that enforces the
  boundary once the scaffold exists* — the no-op-until-scaffold-lands semantics
  in Phase 3 make this ordering safe.
- **Whisper local FR variant selection** — #1793 (which local model to wire).
  This ticket enforces "local-only"; it does not choose which local.
- **Config schema for testimony vs conversation rooms.** Handled in #1786
  (Phase 1 P1.1 — LiveKit Cloud setup) + #1792 (Phase 2 P2.1 — self-hosted
  LiveKit); this ticket contributes the *type-safe surface* those tickets bind
  their config to.

## AC mapping (body → plan)

| Body AC | Plan phase | Concrete gate |
|---|---|---|
| AC1 — Compile fail if dev wires cloud STT into testimony lane | Phase 1 (Rust marker types) | `trybuild` compile-fail test in `tests/voice_lane_compile_fail/` |
| AC2 — Test suite `test_testimony_non_transit_invariant` passes | Phase 1 + Phase 3 | Rust integration test asserts trait bounds; `scripts/verify-voice-non-transit.sh` runs clean |
| AC3 — CI custom lint rule active | Phase 3 | `voice-non-transit-lint` CI job wired in `.github/workflows/ci.yml` |
| AC4 — `docs/voice-non-transit-invariant.md` explains the guarantee | Phase 4 | Doc created; runtime-egress carve-out cited to sibling ticket |
| AC5 — Audit externally: anyone verifies by reading code | Phases 1–4 | Doc § "How to audit externally" lists three grep-commands proving the invariant holds |

**Note on the body's "Runtime egress audit" (listed as scope item #4, not as an
AC number).** The plan carves it out (see § Out of scope) with a companion
ticket. This is not an AC-vs-plan divergence — the body enumerates 5 acceptance
criteria (AC1–AC5) and #4 in the body's `## Scope` is a **defense-in-depth
scope item**, not an AC. The plan honors it via a documented cross-ref plus a
follow-up ticket.

## Files to change

Under `crates/mika-gateway/`:
- `src/voice/mod.rs` — new module root.
- `src/voice/lane.rs` — `ConversationLane`, `TestimonyLane`, `VoiceLane` sealed trait.
- `src/voice/provider.rs` — `CloudStt`, `LocalStt`, `CloudTts`, `LocalTts` traits with lane-associated constants.
- `src/voice/room.rs` — `VoiceRoom<L, S, T>` generic room type with lane-scoped ctors.
- `src/voice/config.rs` — startup config validator (testimony endpoints must be loopback / LAN).
- `tests/voice_lane_compile_fail/` — `trybuild` fixtures + expected `.stderr`.
- `tests/voice_lane_invariant.rs` — Rust integration test `test_testimony_non_transit_invariant`.
- `Cargo.toml` — add `trybuild = "1"` as `[dev-dependencies]`.

At repo root:
- `mika/deny.toml` — `[bans]` deny list additions (Phase 2).
- `mika/scripts/verify-voice-non-transit.sh` — new lint script (Phase 3).
- `mika/.github/workflows/ci.yml` — new `voice-non-transit-lint` job.
- `mika/docs/voice-non-transit-invariant.md` — new doctrine doc.

## Verification

Ordered — each step is a hard gate:

1. `cargo build -p mika-gateway` — types compile.
2. `cargo test -p mika-gateway --test voice_lane_compile_fail` — trybuild passes;
   negative fixtures produce expected trait-bound errors.
3. `cargo test -p mika-gateway test_testimony_non_transit_invariant` — integration
   test green.
4. `cargo deny check bans` — new deny entries surface no existing dep hits (they
   shouldn't; nothing today depends on them). If a hit appears, the offending
   crate is already in a testimony path and the fix precedes ship.
5. `bash scripts/verify-voice-non-transit.sh` — clean (no-op until #1787 lands,
   then enforcing).
6. `cargo clippy --workspace --all-targets -- -D warnings` — no clippy regressions.
7. `cargo fmt --check` — clean.

## Definition of Done

Every item observable via a command, file inspection, or CI job. All must be green before merge.

- [ ] `crates/mika-gateway/src/voice/{mod,lane,provider,room,config}.rs` compile cleanly (`cargo build -p mika-gateway`).
- [ ] `ConversationLane` / `TestimonyLane` markers + sealed `VoiceLane` trait + `VoiceRoom<L, S, T>` generic ship, with no conversion path between lanes.
- [ ] `trybuild` compile-fail fixture in `crates/mika-gateway/tests/voice_lane_compile_fail/` produces expected trait-bound error when a cloud STT/TTS is wired into `TestimonyLane`.
- [ ] `test_testimony_non_transit_invariant` (Rust integration test) passes and covers: type-level lane bounds, config validator loopback/LAN check, and — once #1787 lands — Python lint no-op-to-enforcing transition.
- [ ] `mika/deny.toml [bans]` denies each named cloud STT/TTS crate identifier (elevenlabs, elevenlabs-rs, deepgram, deepgram-rs, azure-speech, google-speech, aws-sdk-transcribe, aws-sdk-polly). Escape entries in `[bans.allow]` (if any) name the extracted crate path per § Risks.
- [ ] `cargo deny check bans` exits 0 on the branch.
- [ ] `scripts/verify-voice-non-transit.sh` exists, is executable, mirrors `scripts/verify-egress-uniqueness.sh`, exits 0 as no-op when `skills/bundled/voice-livekit-agents/testimony/` is absent, and exits 1 on any banned import once the directory exists.
- [ ] `voice-non-transit-lint` CI job is wired into `.github/workflows/ci.yml` next to `egress-uniqueness-lint` and runs on every PR.
- [ ] `docs/voice-non-transit-invariant.md` exists and contains: Prime line quoted verbatim, three-gate enforcement layers with failure mode each catches, four-step recipe to add a new lane provider, three greppable audit commands, escape-hatch discipline, and cross-ref to runtime-egress companion ticket.
- [ ] Companion ticket filed for runtime egress (`voice(p2.5-runtime): nftables egress deny for testimony ports`) and cited in `docs/voice-non-transit-invariant.md § Related`.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --check` clean; `make verify-bundled-skills` clean.

## Acceptance criteria

Verbatim from `senara-solutions/mika#1796` issue body § Acceptance criteria:

- [ ] AC1 — Compile fail si dev accidentellement wire cloud STT dans testimony lane
- [ ] AC2 — Test suite : `test_testimony_non_transit_invariant` passe (vérifie types, config, egress)
- [ ] AC3 — CI custom lint rule active
- [ ] AC4 — Doc : `docs/voice-non-transit-invariant.md` explique la garantie et comment la maintenir
- [ ] AC5 — Audit externe : n'importe qui peut vérifier en lisant le code que testimony ne sort pas

## Risks and open questions

- **cargo-deny scope granularity.** cargo-deny does not natively scope bans to
  a single crate's dep-subgraph — it evaluates the workspace as a whole. If a
  future PR wants cloud STT for a *conversation-only* extraction crate, the
  ban trips workspace-wide. **Mitigation:** the ban list carries an inline
  comment naming the process ("extract to a separate crate never touched by
  testimony, then add to `[bans.allow]` citing the ticket **and naming the
  extracted crate path**"). Concretely: an escape entry MUST take the shape
  `see #NNNN — crate extracted to crates/<name>/` (not just `see #NNNN`) so
  a reviewer running `grep bans.allow deny.toml` can audit the extraction
  point without following a link. This is deliberately mild friction — the
  discipline of naming the ticket + crate path is the audit surface.
- **The `openai` import ban catches WhisperAPI *and* the OpenAI SDK generally.**
  If a future testimony feature legitimately needs an `openai` import for a
  non-STT purpose (e.g., LLM extractor per #1797), the ticket cites the
  `# voice-non-transit: safe` escape with the rationale. Preferred: pin the
  LLM extractor to an OpenAI-compatible **local** endpoint (Ollama, vLLM) — the
  config validator (Phase 1 startup check) enforces loopback/LAN, so the
  import might land but the runtime call cannot leave the box. **Explicit
  layering:** the CI lint is defense-in-depth; the config validator is the
  primary runtime guard for testimony-path calls that use generic HTTP.
- **`trybuild` snapshot fragility across compiler versions.** Rustc error
  messages change between toolchain versions and can break the `.stderr`
  snapshot. **Mitigation:** the CI toolchain is pinned by `rust-toolchain.toml`;
  a toolchain bump PR includes the `TRYBUILD=overwrite cargo test` refresh in
  the same commit. Not a spec-level concern for this ticket; standard operating
  practice for any trybuild-based test.
- **Python side is unbuilt today.** Phase 3's script is a no-op until #1787
  lands. This is intentional (arms the gate before the scaffold ships), but
  the value only materializes on scaffold-land — the sub-issue sequencing in
  #1785 has #1796 as Phase 2 P2.5, later than #1787 (Phase 1 P1.2). Ship order
  is not blocking; both merge before the pipeline goes live.

## References

- **Prime doctrine substrate** — the parent milestone's non-transit clause:
  `senara-solutions/mika#1785` § "Two lanes, hard boundary (non-transit doctrine)".
- **Egress-uniqueness precedent** — `senara-solutions/mika#1807` AC4 +
  `scripts/verify-egress-uniqueness.sh` (near-identical pattern this plan mirrors).
- **Structural-not-prompt discipline** —
  `~/.claude/projects/-data-workspace-mika-platform/memory/feedback_prompt_enforcement_fragile.md`.
- **CI lint script pattern precedents** — `scripts/check-byte-slices.sh` (#764),
  `scripts/check-loop-select.sh` (#848).
- **Sibling ticket** — `senara-solutions/mika#1793` (Whisper local FR variant
  selection — chooses WHICH local; this ticket enforces LOCAL-ONLY).
- **Companion (to-be-filed)** — `voice(p2.5-runtime): nftables egress deny for
  testimony ports` — the runtime/deployment-side defense-in-depth carved out of
  body scope item #4.

<!-- GROOMED marker: /mika-groom-ticket 2026-08-22 -->
