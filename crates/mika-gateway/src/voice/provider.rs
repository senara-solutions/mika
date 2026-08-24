//! Lane-scoped STT/TTS provider traits (mika#1796).
//!
//! # Load-bearing invariant
//!
//! *A provider type can belong to exactly one lane.*
//!
//! The base traits [`SttProvider`] and [`TtsProvider`] each carry an
//! associated `type Lane: VoiceLane`. Because Rust allows only one `impl`
//! of a trait per type, a concrete provider (e.g., `DeepgramStt`) picks
//! exactly one lane at implementation time; it CANNOT simultaneously
//! satisfy both `SttProvider<Lane = ConversationLane>` and
//! `SttProvider<Lane = TestimonyLane>`. This closes the "hybrid provider
//! bypasses the type gate" hole (see mika#1796 review lens 1 — a plain
//! `CloudStt + LocalStt` blanket-trait split leaves the invariant
//! discipline-enforced, not construction-enforced).
//!
//! # Convenience aliases
//!
//! The four public alias traits — [`CloudStt`], [`CloudTts`], [`LocalStt`],
//! [`LocalTts`] — are automatically implemented for any `SttProvider` /
//! `TtsProvider` matching the corresponding lane. They exist for readable
//! bounds at call sites (`fn wire<S: LocalStt>(...)` reads better than
//! `fn wire<S: SttProvider<Lane = TestimonyLane>>(...)`). Because the
//! aliases are blanket impls over the disjoint associated type, the same
//! disjointness holds: `CloudStt` and `LocalStt` are mutually exclusive
//! for any single provider type.

use super::lane::{ConversationLane, TestimonyLane, VoiceLane};

/// Base STT provider trait. A concrete provider picks exactly one lane by
/// binding [`Self::Lane`] to a [`VoiceLane`] marker — that binding is the
/// load-bearing disjointness (see module docs).
pub trait SttProvider: Send + Sync + 'static {
    /// The lane this provider belongs to. A type can only impl `SttProvider`
    /// once, so it can only pick one lane.
    type Lane: VoiceLane;

    /// Human-readable provider identifier for logs and audit trails.
    fn provider_name(&self) -> &'static str;
}

/// Base TTS provider trait. Mirror of [`SttProvider`] for the TTS modality.
pub trait TtsProvider: Send + Sync + 'static {
    /// The lane this provider belongs to. A type can only impl `TtsProvider`
    /// once, so it can only pick one lane.
    type Lane: VoiceLane;

    /// Human-readable provider identifier for logs and audit trails.
    fn provider_name(&self) -> &'static str;
}

/// Alias trait — `SttProvider<Lane = ConversationLane>`. Auto-impl'd; do not
/// implement directly.
///
/// Reads well at call sites: `fn wire<S: CloudStt>(...)`. Semantically:
/// "an STT provider whose runtime call reaches a cloud endpoint" (Deepgram,
/// Whisper API on OpenAI, Google Speech, Azure Speech, AWS Transcribe, …).
pub trait CloudStt: SttProvider<Lane = ConversationLane> {}
impl<T: SttProvider<Lane = ConversationLane>> CloudStt for T {}

/// Alias trait — `TtsProvider<Lane = ConversationLane>`. Auto-impl'd; do not
/// implement directly.
pub trait CloudTts: TtsProvider<Lane = ConversationLane> {}
impl<T: TtsProvider<Lane = ConversationLane>> CloudTts for T {}

/// Alias trait — `SttProvider<Lane = TestimonyLane>`. Auto-impl'd; do not
/// implement directly.
///
/// Reads well at call sites: `fn wire<S: LocalStt>(...)`. Semantically:
/// "an STT provider that runs entirely on the host machine" (faster-whisper,
/// whisper.cpp, vosk, or a locally-hosted Whisper via Ollama / vLLM at a
/// loopback endpoint). The runtime config validator at
/// [`crate::voice::config`] rejects any endpoint outside loopback / RFC1918
/// LAN at startup.
pub trait LocalStt: SttProvider<Lane = TestimonyLane> {}
impl<T: SttProvider<Lane = TestimonyLane>> LocalStt for T {}

/// Alias trait — `TtsProvider<Lane = TestimonyLane>`. Auto-impl'd; do not
/// implement directly.
pub trait LocalTts: TtsProvider<Lane = TestimonyLane> {}
impl<T: TtsProvider<Lane = TestimonyLane>> LocalTts for T {}
