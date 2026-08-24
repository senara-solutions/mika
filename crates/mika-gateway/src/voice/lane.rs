//! Voice-lane marker types and the sealed [`VoiceLane`] trait (mika#1796).
//!
//! The two marker types are zero-sized and non-convertible. The `VoiceLane`
//! trait is sealed via the private [`sealed::Sealed`] super-trait so that
//! downstream crates cannot add a new `impl VoiceLane for ...` — the invariant
//! is that these two lanes are the complete universe of voice lanes.

/// Zero-sized marker for the **conversation lane** — cloud STT/TTS permitted.
///
/// Only used at the type level (as a `VoiceRoom<ConversationLane, _, _>`
/// type parameter). Instances are never constructed by application code —
/// the ctor is public so example / test fixtures may construct one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversationLane;

/// Zero-sized marker for the **testimony lane** — LOCAL-only STT/TTS.
///
/// The type system prevents wiring a cloud provider into this lane; see
/// [`crate::voice::room::VoiceRoom`]'s per-lane ctors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestimonyLane;

/// Sealed trait unifying [`ConversationLane`] and [`TestimonyLane`].
///
/// **Sealed** — downstream crates cannot add new lanes. This is the invariant
/// that makes the "two-lanes, hard boundary" doctrine of mika#1785 §
/// non-transit into a compile-time property.
///
/// Supertrait bounds are minimal: only `sealed::Sealed` (the sealing contract)
/// and `'static` (needed because `Lane` is used as an associated type in
/// `SttProvider` / `TtsProvider`, which themselves require `'static`).
/// `Copy` / `Debug` are impl'd on the marker types themselves via `derive`,
/// not required here — future lane markers can carry different derives.
pub trait VoiceLane: sealed::Sealed + 'static {
    /// Human-readable lane name for logs and audit trails.
    const NAME: &'static str;
}

impl VoiceLane for ConversationLane {
    const NAME: &'static str = "conversation";
}

impl VoiceLane for TestimonyLane {
    const NAME: &'static str = "testimony";
}

/// Private sealing module — see the standard sealed-trait pattern in
/// <https://rust-lang.github.io/api-guidelines/future-proofing.html#sealed-traits-protect-against-downstream-implementations-c-sealed>.
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::ConversationLane {}
    impl Sealed for super::TestimonyLane {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_names_are_stable() {
        // Log / audit surfaces depend on these strings — regressions here
        // would break external audit tooling that greps by lane name.
        assert_eq!(ConversationLane::NAME, "conversation");
        assert_eq!(TestimonyLane::NAME, "testimony");
    }

    #[test]
    fn markers_are_zero_sized() {
        // The lane markers are pure type-level tags; carrying no runtime
        // data is what lets `VoiceRoom<L, S, T>` treat the lane as a
        // phantom parameter with no ABI cost.
        assert_eq!(core::mem::size_of::<ConversationLane>(), 0);
        assert_eq!(core::mem::size_of::<TestimonyLane>(), 0);
    }
}
