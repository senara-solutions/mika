//! Reference provider impls (mika#1796) — **test-utils only**.
//!
//! This module is gated behind `#[cfg(any(test, feature = "test-utils"))]`
//! so the stub types never ship in the release surface — they exist for
//! `trybuild` compile-fail fixtures and internal unit / integration tests.
//! Consumers writing real providers do so in their own crate/module and
//! impl [`SttProvider`](super::SttProvider) / [`TtsProvider`](super::TtsProvider)
//! directly.
//!
//! Zero-runtime-cost stub types whose names deliberately echo well-known
//! provider crates (`DeepgramStt`, `ElevenLabsTts`, `WhisperCppStt`,
//! `PiperTts`) so:
//!
//! 1. `tests/voice_lane_compile_fail/` fixtures can construct them by name
//!    to prove the type-checker rejects wiring cloud into testimony.
//! 2. `tests/voice_lane_invariant.rs` can build canonical rooms on top of
//!    them without requiring any real network dependency.
//! 3. A reader auditing this module immediately recognizes the pattern.
//!
//! The cloud-provider crate NAMES that these types echo (`deepgram`,
//! `elevenlabs`) are banned by `deny.toml [bans]`, so no legitimate
//! future provider impl in this crate will pull them in — see `deny.toml`
//! and `docs/voice-non-transit-invariant.md`.

use super::lane::{ConversationLane, TestimonyLane};
use super::provider::{SttProvider, TtsProvider};

/// Stub cloud STT — reference impl modeled after Deepgram.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeepgramStt;

impl SttProvider for DeepgramStt {
    type Lane = ConversationLane;
    fn provider_name(&self) -> &'static str {
        "deepgram"
    }
}

/// Stub cloud TTS — reference impl modeled after ElevenLabs.
#[derive(Debug, Default, Clone, Copy)]
pub struct ElevenLabsTts;

impl TtsProvider for ElevenLabsTts {
    type Lane = ConversationLane;
    fn provider_name(&self) -> &'static str {
        "elevenlabs"
    }
}

/// Stub local STT — reference impl modeled after whisper.cpp.
#[derive(Debug, Default, Clone, Copy)]
pub struct WhisperCppStt;

impl SttProvider for WhisperCppStt {
    type Lane = TestimonyLane;
    fn provider_name(&self) -> &'static str {
        "whisper-cpp"
    }
}

/// Stub local TTS — reference impl modeled after Piper.
#[derive(Debug, Default, Clone, Copy)]
pub struct PiperTts;

impl TtsProvider for PiperTts {
    type Lane = TestimonyLane;
    fn provider_name(&self) -> &'static str {
        "piper"
    }
}
