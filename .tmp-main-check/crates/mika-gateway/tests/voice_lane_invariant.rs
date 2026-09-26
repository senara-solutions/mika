//! mika#1796 — integration test for the testimony-lane non-transit invariant.
//!
//! Asserts the three build-time gates the [`voice`](mika_gateway::voice)
//! module ships:
//!
//! 1. **Type bounds** — `VoiceRoom<TestimonyLane, _, _>` requires
//!    `S: SttProvider<Lane = TestimonyLane>` + `T: TtsProvider<Lane = TestimonyLane>`.
//!    Negative proof lives in `tests/voice_lane_compile_fail/` (a
//!    compile-fail fixture would corrupt this positive-path test file if
//!    inlined). Positive path is exercised here.
//! 2. **Config validator** — testimony endpoints outside loopback / RFC1918
//!    LAN are rejected at startup, fail-closed.
//! 3. **Lane discriminant carries through the type** — `lane_name()` returns
//!    the correct string for each ctor. External audit tools depend on it.
//!
//! Requires the `test-utils` feature (activates `voice::examples`).

#![cfg(feature = "test-utils")]

use mika_gateway::voice::{
    ConversationLane, SttProvider, TestimonyLane, TtsProvider, VoiceConfig, VoiceConfigError,
    VoiceLane, VoiceRoom,
    examples::{DeepgramStt, ElevenLabsTts, PiperTts, WhisperCppStt},
};

#[test]
fn test_testimony_non_transit_invariant() {
    // --- Gate 1: type bounds compose correctly on the positive path -----
    let testimony_room = VoiceRoom::testimony(WhisperCppStt, PiperTts);
    assert_eq!(testimony_room.lane_name(), TestimonyLane::NAME);
    assert_eq!(testimony_room.stt().provider_name(), "whisper-cpp");
    assert_eq!(testimony_room.tts().provider_name(), "piper");

    let conversation_room = VoiceRoom::conversation(DeepgramStt, ElevenLabsTts);
    assert_eq!(conversation_room.lane_name(), ConversationLane::NAME);
    assert_eq!(conversation_room.stt().provider_name(), "deepgram");
    assert_eq!(conversation_room.tts().provider_name(), "elevenlabs");

    // Assert associated-type lane binding is what the stubs advertise.
    assert_eq!(
        <WhisperCppStt as SttProvider>::Lane::NAME,
        TestimonyLane::NAME
    );
    assert_eq!(<PiperTts as TtsProvider>::Lane::NAME, TestimonyLane::NAME);
    assert_eq!(
        <DeepgramStt as SttProvider>::Lane::NAME,
        ConversationLane::NAME
    );
    assert_eq!(
        <ElevenLabsTts as TtsProvider>::Lane::NAME,
        ConversationLane::NAME
    );

    // --- Gate 2: config validator enforces loopback / RFC1918 LAN -------
    // Positive: loopback + LAN endpoints validate cleanly.
    let good = VoiceConfig {
        testimony_endpoints: vec![
            "127.0.0.1:8080".to_string(),
            "[::1]:8080".to_string(),
            "10.0.0.5:9000".to_string(),
            "192.168.1.100:9000".to_string(),
        ],
    };
    good.validate()
        .expect("all loopback/LAN endpoints should validate");

    // Negative: a WAN address is rejected fail-closed.
    let bad = VoiceConfig {
        testimony_endpoints: vec!["127.0.0.1:8080".to_string(), "8.8.8.8:443".to_string()],
    };
    let err = bad.validate().expect_err("WAN endpoint must be rejected");
    assert!(
        matches!(err, VoiceConfigError::TestimonyEndpointNotLocal { .. }),
        "expected TestimonyEndpointNotLocal, got {err:?}"
    );

    // Negative: a hostname is rejected — DNS-time resolution is out of
    // scope for this validator.
    let host = VoiceConfig {
        testimony_endpoints: vec!["whisper.example.com:8080".to_string()],
    };
    let err = host.validate().expect_err("hostname must be rejected");
    assert!(
        matches!(err, VoiceConfigError::TestimonyEndpointIsHostname { .. }),
        "expected TestimonyEndpointIsHostname, got {err:?}"
    );

    // Negative: garbage port is rejected.
    let bad_port = VoiceConfig {
        testimony_endpoints: vec!["127.0.0.1:99999".to_string()],
    };
    let err = bad_port
        .validate()
        .expect_err("out-of-range port must be rejected");
    assert!(
        matches!(err, VoiceConfigError::TestimonyEndpointUnparseable { .. }),
        "expected TestimonyEndpointUnparseable, got {err:?}"
    );

    // --- Gate 3: lane discriminants are stable for external audit -------
    // These strings appear in logs and are grep'd by external audit tooling.
    assert_eq!(ConversationLane::NAME, "conversation");
    assert_eq!(TestimonyLane::NAME, "testimony");
}
