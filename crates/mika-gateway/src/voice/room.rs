//! [`VoiceRoom`] — generic room orchestrator whose per-lane ctors enforce
//! the non-transit invariant (mika#1796).
//!
//! The type parameter `L: VoiceLane` is a phantom carrying the lane at the
//! type level. The per-lane ctors ([`VoiceRoom::conversation`],
//! [`VoiceRoom::testimony`]) are the *only* way to construct a room; each
//! bounds the STT and TTS types to providers whose associated `Lane` type
//! matches the room's lane. Because a concrete provider type can implement
//! [`SttProvider`](super::SttProvider) / [`TtsProvider`](super::TtsProvider)
//! only ONCE, it picks its lane at implementation time and cannot smuggle
//! itself into another room.

use core::marker::PhantomData;

use super::lane::{ConversationLane, TestimonyLane, VoiceLane};
use super::provider::{SttProvider, TtsProvider};

/// A voice room orchestrator, parameterized by the lane `L` and its STT/TTS
/// provider types `S` and `T`.
///
/// # Type-level invariant
///
/// The struct is generic over any triple `(L, S, T)`, but the *only public
/// constructors* are the per-lane ctors:
///
/// - [`VoiceRoom::conversation`] — requires `S: SttProvider<Lane = ConversationLane>` +
///   `T: TtsProvider<Lane = ConversationLane>`.
/// - [`VoiceRoom::testimony`] — requires `S: SttProvider<Lane = TestimonyLane>` +
///   `T: TtsProvider<Lane = TestimonyLane>`.
///
/// So while `VoiceRoom<TestimonyLane, DeepgramStt, ElevenLabsTts>` is
/// *nameable* as a type, it cannot be *constructed*, because `DeepgramStt`
/// binds `type Lane = ConversationLane` — the compiler rejects the
/// `S: SttProvider<Lane = TestimonyLane>` bound on `VoiceRoom::testimony`.
/// See `tests/voice_lane_compile_fail/` for the snapshot-tested proof.
#[derive(Debug)]
pub struct VoiceRoom<L: VoiceLane, S, T> {
    _lane: PhantomData<L>,
    stt: S,
    tts: T,
}

impl<S, T> VoiceRoom<ConversationLane, S, T>
where
    S: SttProvider<Lane = ConversationLane>,
    T: TtsProvider<Lane = ConversationLane>,
{
    /// Construct a conversation-lane room. STT and TTS must be conversation-
    /// lane providers — the associated-type bounds enforce it.
    pub fn conversation(stt: S, tts: T) -> Self {
        Self {
            _lane: PhantomData,
            stt,
            tts,
        }
    }
}

impl<S, T> VoiceRoom<TestimonyLane, S, T>
where
    S: SttProvider<Lane = TestimonyLane>,
    T: TtsProvider<Lane = TestimonyLane>,
{
    /// Construct a testimony-lane room. STT and TTS must be testimony-lane
    /// providers — the associated-type bounds enforce it. This is the
    /// load-bearing ctor for the non-transit invariant: a type that binds
    /// `type Lane = ConversationLane` cannot be passed here.
    pub fn testimony(stt: S, tts: T) -> Self {
        Self {
            _lane: PhantomData,
            stt,
            tts,
        }
    }
}

impl<L: VoiceLane, S, T> VoiceRoom<L, S, T> {
    /// Human-readable lane name — mirrors [`VoiceLane::NAME`] for the type
    /// parameter. Useful in logs and audit trails.
    pub fn lane_name(&self) -> &'static str {
        L::NAME
    }

    /// Borrow the room's STT provider.
    pub fn stt(&self) -> &S {
        &self.stt
    }

    /// Borrow the room's TTS provider.
    pub fn tts(&self) -> &T {
        &self.tts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::examples::{DeepgramStt, ElevenLabsTts, PiperTts, WhisperCppStt};
    use crate::voice::provider::{SttProvider, TtsProvider};

    #[test]
    fn conversation_room_accepts_cloud_providers() {
        // Positive path: cloud STT + cloud TTS wire cleanly into a
        // conversation-lane room. Unit-struct ctors are bare per clippy's
        // `default_constructed_unit_structs` lint.
        let room = VoiceRoom::conversation(DeepgramStt, ElevenLabsTts);
        assert_eq!(room.lane_name(), "conversation");
        assert_eq!(room.stt().provider_name(), "deepgram");
        assert_eq!(room.tts().provider_name(), "elevenlabs");
    }

    #[test]
    fn testimony_room_accepts_local_providers() {
        // Positive path: local STT + local TTS wire cleanly into a
        // testimony-lane room. Companion to the compile-fail fixture in
        // `tests/voice_lane_compile_fail/` (which proves the NEGATIVE path
        // is rejected at compile time).
        let room = VoiceRoom::testimony(WhisperCppStt, PiperTts);
        assert_eq!(room.lane_name(), "testimony");
        assert_eq!(room.stt().provider_name(), "whisper-cpp");
        assert_eq!(room.tts().provider_name(), "piper");
    }
}
