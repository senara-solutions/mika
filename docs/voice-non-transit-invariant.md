# Voice testimony lane — non-transit invariant

> **Prime line — non-negotiable.**
> *Construis l'incapacité, ne promets pas la retenue.*
> (Build the incapacity, don't promise the restraint.)

The testimony lane's non-transit property is a **build-time invariant, not a
runtime toggle**. On any commit that lands on `main`, the testimony code path
*cannot* reference a cloud STT/TTS provider — the compiler, the dependency
graph, and CI all refuse to produce such a build.

This document is the reader's map to the three composed gates, the audit
recipe, the escape hatches, and the runtime companion that closes the loop
at the network layer.

Ticket: [`senara-solutions/mika#1796`](https://github.com/senara-solutions/mika/issues/1796).
Parent milestone: [`senara-solutions/mika#1785`](https://github.com/senara-solutions/mika/issues/1785)
(Mika Voice — Phase 2 testimony lane) § "Two lanes, hard boundary".

## The two lanes

| Lane            | Cloud STT/TTS | Use case                                                    |
| --------------- | ------------- | ----------------------------------------------------------- |
| **Conversation** | Permitted     | Interactive dialog: latency + quality dominate.             |
| **Testimony**    | **Never**    | Memoir, testimony, private reflection — audio MUST NOT leave the box. |

The distinction is doctrinal, not a preference. A user recording testimony has
made a privacy contract with the substrate; violating it once is not
recoverable. The design response is to make the violation **structurally
impossible**, not to trust that no operator, no PR reviewer, and no future
refactor will ever slip.

## Three build-time gates (composed)

Each gate catches a different failure mode. All three must be green on every PR.

### Gate 1 — Rust type-level lane separation

**Where:** [`crates/mika-gateway/src/voice/`](../crates/mika-gateway/src/voice/mod.rs).

The gateway crate exposes zero-sized `ConversationLane` and `TestimonyLane`
marker types, unified by a **sealed** `VoiceLane` trait. STT and TTS provider
capabilities are split into four disjoint traits — `CloudStt` / `CloudTts` /
`LocalStt` / `LocalTts`. The room orchestrator `VoiceRoom<L, S, T>` is
generic over the lane, and its per-lane constructors only accept providers
matching the lane:

```rust
impl<S: CloudStt, T: CloudTts> VoiceRoom<ConversationLane, S, T> { pub fn conversation(...) }
impl<S: LocalStt,  T: LocalTts> VoiceRoom<TestimonyLane,    S, T> { pub fn testimony(...) }
```

**Failure caught:** a developer writes `VoiceRoom::testimony(DeepgramStt, ElevenLabsTts)`.
The type-checker rejects it — `DeepgramStt` does not satisfy the `S: LocalStt`
bound.

**Regression proof:** [`crates/mika-gateway/tests/voice_lane_compile_fail/`](../crates/mika-gateway/tests/voice_lane_compile_fail/)
holds `trybuild` compile-fail fixtures. If the type-level lane separation ever
weakens, the fixture *compiles*, which fails the trybuild assertion.

**Defense-in-depth (config validator):** [`voice::config::VoiceConfig::validate`](../crates/mika-gateway/src/voice/config.rs)
rejects any testimony endpoint that resolves outside loopback / RFC1918 LAN at
startup, fail-closed. Even if a config file names a "local" provider pointing
at a cloud URL, the gateway refuses to start.

### Gate 2 — `cargo-deny` bans

**Where:** [`deny.toml`](../deny.toml) `[bans]`.

Cloud STT/TTS crate identifiers are banned from the workspace dependency
graph entirely:

```toml
deny = [
    { name = "elevenlabs" },        # cloud TTS
    { name = "elevenlabs-rs" },
    { name = "deepgram" },          # cloud STT
    { name = "deepgram-rs" },
    { name = "azure-speech" },
    { name = "google-speech" },
    { name = "aws-sdk-transcribe" },
    { name = "aws-sdk-polly" },
]
```

**Failure caught:** a developer adds `deepgram = "0.4"` to any crate's
`Cargo.toml`. `cargo deny check bans` fails; CI blocks the PR.

`cargo-deny` evaluates the workspace as a whole — the ban is workspace-wide,
not scoped to a subgraph. See the escape process under
[§ Adding a new lane provider](#adding-a-new-lane-provider) for how to handle
a legitimate future need.

### Gate 3 — CI lint on the Python testimony subtree

**Where:** [`scripts/verify-voice-non-transit.sh`](../scripts/verify-voice-non-transit.sh),
wired as the `voice-non-transit-lint` job in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

`skills/bundled/voice-livekit-agents/testimony/**/*.py` may not import any
cloud STT/TTS SDK or raw network primitive (`aiohttp`, `httpx`, `requests`,
`urllib.request`). The script mirrors [`scripts/verify-egress-uniqueness.sh`](../scripts/verify-egress-uniqueness.sh)
(mika#1807 AC4) — same shape, same doctrine.

**No-op-until-scaffold-lands.** The LiveKit Agents SDK scaffold is
[mika#1787](https://github.com/senara-solutions/mika/issues/1787)'s
deliverable. Until it lands, the script exits 0 as a no-op. The instant the
testimony directory appears, the gate becomes enforcing without any
additional wiring — so no interleaving-order bug can slip cloud STT/TTS into
`testimony/**` between merges.

**Failure caught:** a developer adds `from deepgram import Deepgram` to a
file under `testimony/`. The lint job fails; CI blocks the PR.

## How to audit externally

Anyone can prove the invariant holds on any commit by running these three
greppable commands from the repository root:

```bash
# 1. The lane types exist and are sealed.
rg 'pub trait VoiceLane: sealed::Sealed' crates/mika-gateway/src/voice/lane.rs

# 2. Only the conversation-lane ctor accepts Cloud providers;
#    only the testimony-lane ctor accepts Local providers.
rg 'impl<S: CloudStt, T: CloudTts> VoiceRoom<ConversationLane' crates/mika-gateway/src/voice/room.rs
rg 'impl<S: LocalStt,  *T: LocalTts> VoiceRoom<TestimonyLane'  crates/mika-gateway/src/voice/room.rs

# 3. The compile-fail fixture proves the type-checker rejects mis-wiring.
cargo test -p mika-gateway --test voice_lane_compile_fail

# 4. Ban list contains every cloud provider identifier.
grep -E '^\s*\{ name = "(elevenlabs|deepgram|azure-speech|google-speech|aws-sdk-(transcribe|polly))' deny.toml

# 5. The Python testimony subtree is either absent (no-op) or clean.
bash scripts/verify-voice-non-transit.sh
```

Each command returns green independently. A red result on any single one is
enough to prove the invariant no longer holds.

## Adding a new lane provider

The four-step recipe for adding a new provider — for either lane — is:

1. **If the provider is a cloud SDK you want in the CONVERSATION lane and its
   crate name is on the ban list**, first extract the conversation-lane
   wiring into a separate crate that is NEVER pulled into the testimony code
   path. Then add an entry to `deny.toml`'s `[bans.allow]` **whose comment
   names the extracted crate path**:

   ```toml
   [bans.allow]
   { name = "deepgram" }  # see #NNNN — crate extracted to crates/mika-voice-conversation/
   ```

   Bare `see #NNNN` (no crate path) is not enough — a reviewer running
   `grep bans.allow deny.toml` must be able to audit the extraction point
   without following a link. This is deliberately mild friction: the
   discipline of naming the ticket + crate path IS the audit surface.

2. **Extend the lane-scoped provider trait implementation.** Impl `CloudStt` /
   `CloudTts` / `LocalStt` / `LocalTts` for your provider type, matching the
   lane. See [`crates/mika-gateway/src/voice/examples.rs`](../crates/mika-gateway/src/voice/examples.rs)
   for reference impls.

3. **If the provider ships a new Python subpackage**, extend `BANNED_IMPORTS`
   in [`scripts/verify-voice-non-transit.sh`](../scripts/verify-voice-non-transit.sh)
   so the CI lint catches an accidental import into `testimony/**` even if
   the top-level package name matches an existing pattern.

4. **Cite the ticket** in every change (deny.toml comment, trait impl doc,
   lint list comment). The audit trail is the discipline.

## Escape hatches

Two, one per gate that supports one. Each escape **must cite a ticket**.

- **`deny.toml [bans.allow]`** — the crate-level escape. See
  [§ Adding a new lane provider](#adding-a-new-lane-provider) step 1 for the
  discipline (name the extracted crate path in the comment).
- **`# voice-non-transit: safe`** — the Python line-level escape. Append this
  comment to any import line in `testimony/**` that the lint would otherwise
  reject. Only use for a genuinely LOCAL surface (e.g., a LiveKit control
  socket over `httpx` targeting a loopback endpoint). Cite the ticket that
  justifies the exception. The `openai` import ban in particular catches the
  Whisper API — a legitimate `openai` import into `testimony/**` for an
  LLM-extractor purpose (e.g., mika#1797) requires this escape plus the
  config validator's loopback/LAN enforcement so the runtime call can't
  leave the box.

The type-level Rust gate has no escape hatch by design. If a future feature
genuinely needs a lane that isn't `ConversationLane` or `TestimonyLane`, the
`VoiceLane` sealed trait must be unsealed in a separate PR that surfaces the
doctrinal change explicitly.

## Runtime companion (deployment layer)

The build-time invariant makes it impossible to *build* a testimony path that
reaches a cloud endpoint. That is not the same as making it impossible to
*run* one. Two runtime hardenings close the loop:

1. **Config validator at startup.** Already inside Gate 1's defense-in-depth
   — see [`voice::config::VoiceConfig::validate`](../crates/mika-gateway/src/voice/config.rs).
   Rejects any testimony endpoint outside loopback / RFC1918 LAN,
   fail-closed.
2. **Network-layer egress deny.** An nftables/iptables rule on the gateway
   host that blocks testimony ports from reaching WAN. This is deployment
   plumbing (mika-cloud NetworkPolicy for cloud deployments; self-hosted
   provisioning script for gateway-box installs), not build-time.

The runtime egress rule is tracked as a companion follow-up ticket:
[**mika#1961 — `voice(p2.5-runtime): nftables egress deny for testimony ports`**](https://github.com/senara-solutions/mika/issues/1961)
— filed alongside this ticket per mika#1796 § Out of scope. This document is
the canonical cross-reference.

## Related

- [Sibling — mika#1793 (Whisper local FR variant selection)](https://github.com/senara-solutions/mika/issues/1793)
  — chooses WHICH local provider; this ticket enforces LOCAL-ONLY.
- [Precedent — mika#1807 (egress-uniqueness lint)](https://github.com/senara-solutions/mika/issues/1807)
  — the near-identical pattern Gate 3 mirrors.
- [Precedent — mika#848 (loop-select-lint)](https://github.com/senara-solutions/mika/issues/848),
  [mika#764 (byte-slice-lint)](https://github.com/senara-solutions/mika/issues/764)
  — the structural-not-prompt CI lint precedents.
- [Companion — mika#1961 (`voice(p2.5-runtime): nftables egress deny for testimony ports`)](https://github.com/senara-solutions/mika/issues/1961)
  — the runtime/deployment-side defense-in-depth carved out of body scope
  item #4.
