//! Voice-lane orchestration surface — build-time non-transit invariant (mika#1796).
//!
//! # Why
//!
//! Mika's voice pipeline has two lanes with fundamentally different privacy
//! contracts:
//!
//! - **Conversation lane** — cloud STT/TTS is permitted. The user is talking
//!   to Mika interactively; latency and quality dominate; the audio traversing
//!   cloud providers is the intended contract.
//! - **Testimony lane** — LOCAL-only STT/TTS. The user is recording something
//!   they want to preserve (memoir, testimony, private reflection); the audio
//!   MUST NEVER leave the box the user is on. This is a doctrinal invariant,
//!   not a preference.
//!
//! # How this module enforces the invariant
//!
//! Prime doctrine: *construis l'incapacité, ne promets pas la retenue.*
//! (Build the incapacity, don't promise the restraint.)
//!
//! The testimony-lane non-transit property is a **build-time invariant, not a
//! runtime toggle**. This module provides the type-level surface that makes it
//! impossible — by construction — to wire a cloud STT/TTS provider into the
//! testimony lane:
//!
//! 1. [`lane`] — Zero-sized `ConversationLane` / `TestimonyLane` marker types,
//!    unified by a **sealed** [`VoiceLane`](lane::VoiceLane) trait (downstream
//!    crates cannot add new lanes).
//! 2. [`provider`] — Lane-scoped provider traits: [`CloudStt`](provider::CloudStt)
//!    / [`CloudTts`](provider::CloudTts) live in the conversation lane;
//!    [`LocalStt`](provider::LocalStt) / [`LocalTts`](provider::LocalTts) live
//!    in the testimony lane. The trait bounds carry the lane at the type level.
//! 3. [`room`] — [`VoiceRoom<L, S, T>`](room::VoiceRoom), a generic room
//!    orchestrator whose per-lane constructors only accept providers matching
//!    the lane. Wiring a `CloudStt` into `VoiceRoom<TestimonyLane, _, _>` fails
//!    at compile time.
//! 4. [`config`] — Runtime config validator (defense-in-depth). Even though
//!    the type system rejects mis-wiring at build time, the validator rejects
//!    any testimony endpoint that resolves outside loopback / RFC1918 LAN at
//!    startup — fails closed (gateway refuses to start, not warn+continue).
//!
//! # How to audit this externally
//!
//! Three greppable commands prove the invariant holds on any commit:
//!
//! ```text
//! # 1. The lane types exist and are sealed.
//! rg 'pub trait VoiceLane: sealed::Sealed' crates/mika-gateway/src/voice/lane.rs
//!
//! # 2. Only the conversation-lane ctor accepts Cloud providers;
//! #    only the testimony-lane ctor accepts Local providers.
//! rg 'impl<S: CloudStt, T: CloudTts> VoiceRoom<ConversationLane' crates/mika-gateway/src/voice/room.rs
//! rg 'impl<S: LocalStt, T: LocalTts> VoiceRoom<TestimonyLane' crates/mika-gateway/src/voice/room.rs
//!
//! # 3. The compile-fail fixture proves the type-checker rejects mis-wiring.
//! cargo test -p mika-gateway --test voice_lane_compile_fail
//! ```
//!
//! Companion gates (outside this module):
//!
//! - `deny.toml [bans]` — cloud STT/TTS crates cannot be pulled into the
//!   workspace at all (Phase 2 of mika#1796).
//! - `scripts/verify-voice-non-transit.sh` — CI lint blocking cloud imports
//!   in the future `skills/bundled/voice-livekit-agents/testimony/` subtree
//!   (Phase 3 of mika#1796; no-op until mika#1787 lands the LiveKit scaffold).
//! - `docs/voice-non-transit-invariant.md` — full doctrine + audit recipe.

pub mod config;
pub mod examples;
pub mod lane;
pub mod provider;
pub mod room;

pub use config::{VoiceConfig, VoiceConfigError};
pub use lane::{ConversationLane, TestimonyLane, VoiceLane};
pub use provider::{CloudStt, CloudTts, LocalStt, LocalTts};
pub use room::VoiceRoom;
