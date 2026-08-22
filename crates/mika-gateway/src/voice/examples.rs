//! Reference provider impls (mika#1796).
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
//! **These types are stubs, not real providers.** They implement the trait
//! surface with no-op methods. The real provider wiring lands with mika#1787
//! (LiveKit Agents Python scaffold) — this module is the type-safe surface
//! that scaffold binds to.
//!
//! The cloud-provider crate NAMES that these types echo (`deepgram`,
//! `elevenlabs`) are banned by `deny.toml [bans]`, so no legitimate future
//! provider impl in this crate will pull them in — see `deny.toml` and the
//! doctrine doc `docs/voice-non-transit-invariant.md`.

use super::provider::{CloudStt, CloudTts, LocalStt, LocalTts};

/// Stub cloud STT — reference impl of [`CloudStt`], modeled after Deepgram.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeepgramStt;

impl CloudStt for DeepgramStt {
    fn provider_name(&self) -> &'static str {
        "deepgram"
    }
}

/// Stub cloud TTS — reference impl of [`CloudTts`], modeled after ElevenLabs.
#[derive(Debug, Default, Clone, Copy)]
pub struct ElevenLabsTts;

impl CloudTts for ElevenLabsTts {
    fn provider_name(&self) -> &'static str {
        "elevenlabs"
    }
}

/// Stub local STT — reference impl of [`LocalStt`], modeled after whisper.cpp.
#[derive(Debug, Default, Clone, Copy)]
pub struct WhisperCppStt;

impl LocalStt for WhisperCppStt {
    fn provider_name(&self) -> &'static str {
        "whisper-cpp"
    }
}

/// Stub local TTS — reference impl of [`LocalTts`], modeled after Piper.
#[derive(Debug, Default, Clone, Copy)]
pub struct PiperTts;

impl LocalTts for PiperTts {
    fn provider_name(&self) -> &'static str {
        "piper"
    }
}
