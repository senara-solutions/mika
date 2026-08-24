//! mika#1796 — compile-fail harness proving the non-transit invariant.
//!
//! The type-level lane separation in `crate::voice` REJECTS a
//! `VoiceRoom<TestimonyLane, CloudStt, CloudTts>` construction attempt at
//! compile time. This harness uses `trybuild` to run the fixtures under
//! `tests/voice_lane_compile_fail/` and assert the expected trait-bound
//! errors — so a regression that weakens the lane separation makes the
//! *fixture* compile, which fails the trybuild assertion.
//!
//! # Regenerating snapshots after a toolchain bump
//!
//! Rustc error text changes between compiler versions; the `.stderr`
//! snapshot is pinned to the toolchain in `rust-toolchain.toml` (1.93). A
//! toolchain bump PR refreshes the snapshot in the same commit:
//!
//! ```text
//! TRYBUILD=overwrite cargo test -p mika-gateway --test voice_lane_compile_fail
//! ```
//!
//! # Doctrine
//!
//! See `docs/voice-non-transit-invariant.md` § "Three build-time gates" for
//! how this test slots into the composed defense (types + cargo-deny + CI
//! lint).

#[test]
fn lane_bounds_reject_wrong_lane_providers() {
    let t = trybuild::TestCases::new();
    // Load-bearing gates — non-transit invariant.
    t.compile_fail("tests/voice_lane_compile_fail/testimony_rejects_cloud_stt.rs");
    t.compile_fail("tests/voice_lane_compile_fail/testimony_rejects_cloud_tts.rs");
    // Symmetry gate — conversation-lane ctor also rejects wrong-lane providers,
    // proving the bound direction is enforced both ways.
    t.compile_fail("tests/voice_lane_compile_fail/conversation_rejects_local_stt.rs");
}
