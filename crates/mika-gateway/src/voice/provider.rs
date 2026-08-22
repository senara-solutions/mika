//! Lane-scoped STT/TTS provider traits (mika#1796).
//!
//! Four disjoint traits, two per modality:
//!
//! | Modality | Conversation lane (cloud OK) | Testimony lane (local only) |
//! |----------|------------------------------|-----------------------------|
//! | STT      | [`CloudStt`]                  | [`LocalStt`]                |
//! | TTS      | [`CloudTts`]                  | [`LocalTts`]                |
//!
//! The lane association is carried structurally: the room orchestrator
//! ([`crate::voice::room::VoiceRoom`]) takes a lane marker as a type parameter
//! and its per-lane ctors only accept providers matching that lane. A type
//! that implements `CloudStt` cannot be passed into `VoiceRoom::testimony(...)`
//! because the ctor's `S: LocalStt` bound is unsatisfied.
//!
//! **Disjointness is enforced by discipline plus type distinctness.** Nothing
//! at the trait level prevents a single type from implementing both `CloudStt`
//! and `LocalStt` — but no such type will ever be authored inside the gateway
//! crate, and the [`CloudSttUsesNetwork`] / [`LocalSttUsesNoNetwork`] doc
//! blocks establish the contract each impl signs. If a future provider is
//! genuinely dual-nature, it MUST be split into two distinct types (one per
//! lane); citing this ticket and the [`crate::voice::examples`] pattern.

use super::lane::{ConversationLane, TestimonyLane};

/// STT (speech-to-text) provider whose runtime call reaches a **cloud
/// endpoint** — Deepgram, Google Speech, Whisper API on OpenAI, Azure Speech,
/// AWS Transcribe, etc.
///
/// # Contract
///
/// - `Self::LANE == ConversationLane`.
/// - Impl signs the [`CloudSttUsesNetwork`] contract: audio bytes leave the
///   host machine over the network. Cloud provider crates are banned by
///   `deny.toml [bans]` from being pulled into the workspace at all; a legal
///   impl of this trait therefore either mocks the transport (for tests) or
///   depends on a hand-rolled HTTP client. See mika#1796 for the ban list.
///
/// # Compile-time guarantee
///
/// This trait is **not** implementable in a way that satisfies
/// [`crate::voice::room::VoiceRoom::testimony`]'s `S: LocalStt` bound (unless
/// the same type also impls [`LocalStt`], which is a discipline violation and
/// caught by code review — no such impl exists in-tree).
pub trait CloudStt: Send + Sync + 'static {
    /// Lane discriminator — always [`ConversationLane`] for this trait.
    const LANE: ConversationLane = ConversationLane;

    /// Human-readable provider identifier for logs and audit trails.
    /// Example: `"deepgram"`, `"whisper-api"`, `"google-speech"`.
    fn provider_name(&self) -> &'static str;
}

/// TTS (text-to-speech) provider whose runtime call reaches a **cloud
/// endpoint** — ElevenLabs, Google TTS, Azure Speech, AWS Polly, etc.
///
/// # Contract
///
/// Mirror of [`CloudStt`] for the TTS modality. Same ban discipline applies.
pub trait CloudTts: Send + Sync + 'static {
    /// Lane discriminator — always [`ConversationLane`] for this trait.
    const LANE: ConversationLane = ConversationLane;

    /// Human-readable provider identifier for logs and audit trails.
    /// Example: `"elevenlabs"`, `"google-tts"`, `"azure-speech-tts"`.
    fn provider_name(&self) -> &'static str;
}

/// STT provider that runs **entirely on the host machine** — faster-whisper,
/// whisper.cpp, vosk, or a locally-hosted Whisper via Ollama / vLLM at a
/// loopback endpoint.
///
/// # Contract
///
/// - `Self::LANE == TestimonyLane`.
/// - Impl signs the [`LocalSttUsesNoNetwork`] contract: no audio byte leaves
///   the host as part of the STT call. A "local" provider that reaches a
///   loopback endpoint (`127.0.0.1`, `::1`, or an RFC1918 LAN address in a
///   self-hosted deployment) still qualifies — the runtime config validator
///   at [`crate::voice::config`] rejects anything outside that surface at
///   startup.
///
/// # Compile-time guarantee
///
/// Only `LocalStt` implementations satisfy [`crate::voice::room::VoiceRoom::testimony`]'s
/// STT bound. A pure `CloudStt` type is rejected by the type-checker; see
/// `tests/voice_lane_compile_fail/`.
pub trait LocalStt: Send + Sync + 'static {
    /// Lane discriminator — always [`TestimonyLane`] for this trait.
    const LANE: TestimonyLane = TestimonyLane;

    /// Human-readable provider identifier for logs and audit trails.
    /// Example: `"faster-whisper"`, `"whisper-cpp"`, `"vosk"`.
    fn provider_name(&self) -> &'static str;
}

/// TTS provider that runs **entirely on the host machine** — Piper, Silero,
/// Coqui, or a locally-hosted TTS at a loopback endpoint.
///
/// # Contract
///
/// Mirror of [`LocalStt`] for the TTS modality.
pub trait LocalTts: Send + Sync + 'static {
    /// Lane discriminator — always [`TestimonyLane`] for this trait.
    const LANE: TestimonyLane = TestimonyLane;

    /// Human-readable provider identifier for logs and audit trails.
    /// Example: `"piper"`, `"silero"`, `"coqui-xtts"`.
    fn provider_name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Marker documentation-only "contracts" — no runtime code; existence pins the
// invariant into the type system's documentation.
// ---------------------------------------------------------------------------

/// **Doc-only marker.** An impl of [`CloudStt`] signs this contract: the STT
/// call reaches a cloud endpoint over the network.
///
/// This trait has no runtime meaning — it exists so a reviewer greping for
/// `CloudSttUsesNetwork` finds the doctrinal boundary in one place.
pub trait CloudSttUsesNetwork {}
impl<T: CloudStt> CloudSttUsesNetwork for T {}

/// **Doc-only marker.** An impl of [`LocalStt`] signs this contract: the STT
/// call does not reach any network endpoint outside loopback / LAN.
///
/// See [`CloudSttUsesNetwork`] for the pattern rationale.
pub trait LocalSttUsesNoNetwork {}
impl<T: LocalStt> LocalSttUsesNoNetwork for T {}
