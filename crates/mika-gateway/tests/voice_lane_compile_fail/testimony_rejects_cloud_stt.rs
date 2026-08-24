// mika#1796 — negative fixture: wiring a cloud STT into the testimony lane
// MUST fail to compile. This file must NOT compile; trybuild asserts that
// and checks the error message against the paired `.stderr` snapshot.
//
// If this file starts compiling on `cargo test -p mika-gateway --test
// voice_lane_compile_fail`, the type-level lane separation has regressed
// and the non-transit invariant is no longer verified by construction.

use mika_gateway::voice::{
    SttProvider, TtsProvider, VoiceRoom,
    lane::{ConversationLane, TestimonyLane},
};

// Inline stubs — kept in-fixture so `mika_gateway::voice::examples` can
// stay feature-gated and never ship in the release surface.
struct DeepgramStt;
impl SttProvider for DeepgramStt {
    type Lane = ConversationLane; // <-- binds to CONVERSATION lane
    fn provider_name(&self) -> &'static str {
        "deepgram"
    }
}

struct PiperTts;
impl TtsProvider for PiperTts {
    type Lane = TestimonyLane;
    fn provider_name(&self) -> &'static str {
        "piper"
    }
}

fn main() {
    // DeepgramStt binds `type Lane = ConversationLane`.
    // `VoiceRoom::testimony` requires `S: SttProvider<Lane = TestimonyLane>`,
    // so this must not compile.
    let _room: VoiceRoom<TestimonyLane, DeepgramStt, PiperTts> =
        VoiceRoom::testimony(DeepgramStt, PiperTts);
}
