// mika#1796 — negative fixture: wiring a cloud TTS into the testimony lane
// MUST fail to compile. Companion to `testimony_rejects_cloud_stt.rs`
// covering the TTS half of the lane invariant.

use mika_gateway::voice::{
    SttProvider, TtsProvider, VoiceRoom,
    lane::{ConversationLane, TestimonyLane},
};

// Inline stubs — see companion fixture for the feature-gate rationale.
struct WhisperCppStt;
impl SttProvider for WhisperCppStt {
    type Lane = TestimonyLane;
    fn provider_name(&self) -> &'static str {
        "whisper-cpp"
    }
}

struct ElevenLabsTts;
impl TtsProvider for ElevenLabsTts {
    type Lane = ConversationLane; // <-- binds to CONVERSATION lane
    fn provider_name(&self) -> &'static str {
        "elevenlabs"
    }
}

fn main() {
    // ElevenLabsTts binds `type Lane = ConversationLane`.
    // `VoiceRoom::testimony` requires `T: TtsProvider<Lane = TestimonyLane>`,
    // so this must not compile.
    let _room: VoiceRoom<TestimonyLane, WhisperCppStt, ElevenLabsTts> =
        VoiceRoom::testimony(WhisperCppStt, ElevenLabsTts);
}
