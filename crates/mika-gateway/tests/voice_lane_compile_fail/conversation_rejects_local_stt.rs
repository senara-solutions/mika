// mika#1796 — negative fixture: wiring a local STT into the conversation lane
// MUST fail to compile. Completes the four-corner assertion:
//
//   Room lane       | STT lane        | TTS lane       | Compiles?
//   ----------------|-----------------|----------------|----------
//   Conversation    | Conversation    | Conversation   | yes (positive)
//   Testimony       | Testimony       | Testimony      | yes (positive)
//   Testimony       | Conversation    | Testimony      | NO — testimony_rejects_cloud_stt.rs
//   Testimony       | Testimony       | Conversation   | NO — testimony_rejects_cloud_tts.rs
//   Conversation    | Testimony       | Conversation   | NO — this fixture
//
// The Testimony-lane rejections are the load-bearing gates (non-transit
// invariant). The Conversation-lane rejection here matters too: it prevents
// a testimony-only local provider from being smuggled into a conversation
// room where the pipeline might route audio to a cloud sink downstream.

use mika_gateway::voice::{
    SttProvider, TtsProvider, VoiceRoom,
    lane::{ConversationLane, TestimonyLane},
};

// Inline stubs — see testimony_rejects_cloud_stt.rs for the rationale.
struct WhisperCppStt;
impl SttProvider for WhisperCppStt {
    type Lane = TestimonyLane; // <-- binds to TESTIMONY lane
    fn provider_name(&self) -> &'static str {
        "whisper-cpp"
    }
}

struct ElevenLabsTts;
impl TtsProvider for ElevenLabsTts {
    type Lane = ConversationLane;
    fn provider_name(&self) -> &'static str {
        "elevenlabs"
    }
}

fn main() {
    // WhisperCppStt binds `type Lane = TestimonyLane`.
    // `VoiceRoom::conversation` requires `S: SttProvider<Lane = ConversationLane>`,
    // so this must not compile.
    let _room: VoiceRoom<ConversationLane, WhisperCppStt, ElevenLabsTts> =
        VoiceRoom::conversation(WhisperCppStt, ElevenLabsTts);
}
