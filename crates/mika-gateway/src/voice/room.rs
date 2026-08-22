//! [`VoiceRoom`] — generic room orchestrator whose per-lane ctors enforce
//! the non-transit invariant (mika#1796).
//!
//! The type parameter `L: VoiceLane` is a phantom carrying the lane at the
//! type level. The per-lane ctors ([`VoiceRoom::conversation`],
//! [`VoiceRoom::testimony`]) are the *only* way to construct a room; each
//! bounds the STT and TTS types to the traits that live in its lane.

use core::marker::PhantomData;

use super::lane::{ConversationLane, TestimonyLane, VoiceLane};
use super::provider::{CloudStt, CloudTts, LocalStt, LocalTts};

/// A voice room orchestrator, parameterized by the lane `L` and its STT/TTS
/// provider types `S` and `T`.
///
/// # Type-level invariant
///
/// The struct itself is generic over any triple `(L, S, T)`, but the *only
/// public constructors* are the per-lane ctors:
///
/// - [`VoiceRoom::conversation`] — requires `S: CloudStt` + `T: CloudTts`.
/// - [`VoiceRoom::testimony`] — requires `S: LocalStt` + `T: LocalTts`.
///
/// So while `VoiceRoom<TestimonyLane, DeepgramStt, ElevenLabsTts>` is
/// *nameable* as a type, it cannot be *constructed*, because `DeepgramStt`
/// (a `CloudStt`) does not satisfy the `S: LocalStt` bound on
/// `VoiceRoom::testimony`. See `tests/voice_lane_compile_fail/` for the
/// snapshot-tested proof.
#[derive(Debug)]
pub struct VoiceRoom<L: VoiceLane, S, T> {
    _lane: PhantomData<L>,
    stt: S,
    tts: T,
}

impl<S: CloudStt, T: CloudTts> VoiceRoom<ConversationLane, S, T> {
    /// Construct a conversation-lane room. STT and TTS must be cloud
    /// providers — the trait bounds enforce it.
    pub fn conversation(stt: S, tts: T) -> Self {
        Self {
            _lane: PhantomData,
            stt,
            tts,
        }
    }
}

impl<S: LocalStt, T: LocalTts> VoiceRoom<TestimonyLane, S, T> {
    /// Construct a testimony-lane room. STT and TTS must be local providers
    /// — the trait bounds enforce it. This is the load-bearing ctor for the
    /// non-transit invariant: a type that impls `CloudStt` but not `LocalStt`
    /// cannot be passed here without a compile error.
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

    #[test]
    fn conversation_room_accepts_cloud_providers() {
        // Positive path: cloud STT + cloud TTS wire cleanly into a
        // conversation-lane room. This exercises the ctor bounds.
        // Unit-struct ctors: `TypeName` rather than `TypeName::default()` —
        // clippy's `default_constructed_unit_structs` lint enforces this
        // for zero-sized markers.
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
