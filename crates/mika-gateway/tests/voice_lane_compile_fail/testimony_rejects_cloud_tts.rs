// mika#1796 — negative fixture: wiring a cloud TTS into the testimony lane
// MUST fail to compile. Companion to `testimony_rejects_cloud_stt.rs`
// covering the TTS half of the lane invariant.

use mika_gateway::voice::{
    VoiceRoom,
    examples::{ElevenLabsTts, WhisperCppStt},
    lane::TestimonyLane,
};

fn main() {
    // ElevenLabsTts is a CloudTts, not a LocalTts.
    // `VoiceRoom::testimony` requires `T: LocalTts`, so this must not compile.
    let _room: VoiceRoom<TestimonyLane, WhisperCppStt, ElevenLabsTts> =
        VoiceRoom::testimony(WhisperCppStt::default(), ElevenLabsTts::default());
}
