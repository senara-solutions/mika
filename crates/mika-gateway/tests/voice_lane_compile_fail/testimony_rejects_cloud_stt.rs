// mika#1796 — negative fixture: wiring a cloud STT into the testimony lane
// MUST fail to compile. This file must NOT compile; trybuild asserts that
// and checks the error message against the paired `.stderr` snapshot.
//
// If this file starts compiling on `cargo test -p mika-gateway --test
// voice_lane_compile_fail`, the type-level lane separation has regressed
// and the non-transit invariant is no longer verified by construction.

use mika_gateway::voice::{
    VoiceRoom,
    examples::{DeepgramStt, PiperTts},
    lane::TestimonyLane,
};

fn main() {
    // DeepgramStt is a CloudStt, not a LocalStt.
    // `VoiceRoom::testimony` requires `S: LocalStt`, so this must not compile.
    let _room: VoiceRoom<TestimonyLane, DeepgramStt, PiperTts> =
        VoiceRoom::testimony(DeepgramStt::default(), PiperTts::default());
}
