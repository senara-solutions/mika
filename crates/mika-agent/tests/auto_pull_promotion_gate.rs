//! Replay of the mika#2123 promotion gate against the branches that actually
//! broke the loop (U4).
//!
//! The unit tests in `auto_pull::tests` cover the decision table with
//! hand-written values. This suite covers something they cannot: that the real
//! GitHub compare payloads, for the real branches, parse into the values the
//! gate expects and produce the intended verdicts. A gate verified only against
//! numbers a test author typed is a gate verified against the author's belief
//! about the API.
//!
//! Fixtures and their measured rebase outcomes: see
//! `tests/fixtures/auto_pull_compare/PROVENANCE.md`.

use mika_agent::auto_pull::{
    PromotionGate, RefusalReason, StalenessMeasurement, classify_promotion, parse_compare_payload,
};

/// The default threshold at the time these fixtures were frozen.
const THRESHOLD: i64 = 50;

fn fixture(name: &str) -> StalenessMeasurement {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/auto_pull_compare/"
    );
    let raw = std::fs::read_to_string(format!("{path}{name}"))
        .unwrap_or_else(|e| panic!("fixture {name} must be readable: {e}"));
    StalenessMeasurement::Measured(
        parse_compare_payload(&raw).unwrap_or_else(|e| panic!("fixture {name} must parse: {e}")),
    )
}

/// AC4 — non-vacuity. Replaying #1680's promotion must produce a **named**
/// refusal, not a dispatch.
///
/// Measured 2026-09-01: 180 behind, 2 ahead, and a real `git rebase origin/main`
/// conflicts on `agent_loop/mod.rs` and `evidence/guards.rs` — the same two
/// files the issue reported on 2026-08-31.
#[test]
fn auto_pull_replay_1680_is_refused_by_name() {
    let m = fixture("1680-diverged-180-behind-2-ahead.json");
    let StalenessMeasurement::Measured(s) = &m else {
        unreachable!()
    };
    assert_eq!(
        (s.behind_by, s.ahead_by, s.status.as_str()),
        (180, 2, "diverged")
    );

    match classify_promotion(
        &m,
        Some("fix/1680/mika-dev-tui-broken-glyph-rendering-in"),
        THRESHOLD,
    ) {
        PromotionGate::Refuse(r) => {
            // Named, and named *correctly*: #1680 carries a `wip(...)` salvage
            // commit on top of its plan (ahead 2), so the salvage rule is the
            // honest reason — not the distance, which is also true but is the
            // more generic fact.
            assert_eq!(r.slug(), "salvage_work_on_stale_branch");
        }
        PromotionGate::Promote { detail } => {
            panic!("#1680 must not be promoted (got promote/{detail})")
        }
    }
}

/// AC4 — non-vacuity of the **distance rule specifically**.
///
/// #1959 is the only frozen branch the distance rule refuses on its own: 75
/// behind, but `ahead_by == 1`, so the salvage rule does not fire. Remove the
/// threshold (`MIKA_AUTO_PULL_MAX_BEHIND=0`) and this refusal must disappear.
/// Both halves are asserted here, so the test cannot pass vacuously in either
/// direction.
#[test]
fn auto_pull_replay_1959_refusal_is_the_threshold_and_nothing_else() {
    let m = fixture("1959-diverged-75-behind-1-ahead.json");
    let branch = Some("feat/1959/mcp-manifest-data-grade-field-l4-forward");

    match classify_promotion(&m, branch, THRESHOLD) {
        PromotionGate::Refuse(RefusalReason::TooFarBehind {
            behind_by,
            ahead_by,
            threshold,
            ..
        }) => {
            assert_eq!((behind_by, ahead_by, threshold), (75, 1, THRESHOLD));
        }
        other => panic!("#1959 must be refused for distance, got {other:?}"),
    }

    // Threshold disabled → the same branch is promoted. This is the mutation the
    // PR body demonstrates: it is the *only* rule refusing here, so removing it
    // must flip the verdict.
    assert!(matches!(
        classify_promotion(&m, branch, 0),
        PromotionGate::Promote { .. }
    ));
}

/// AC3 — the negative control, on a real branch.
///
/// `ci/2048-re-enable-release-please` is 17 behind with only its own commit, and
/// a real `git rebase origin/main` succeeds (measured 2026-09-01). It **must**
/// be promoted. A gate that refuses everything looks exactly like a gate that
/// works, from the outside; this is the assertion that tells them apart.
#[test]
fn auto_pull_replay_2048_behind_but_rebasable_is_promoted() {
    let m = fixture("2048-diverged-17-behind-1-ahead.json");
    assert!(matches!(
        classify_promotion(&m, Some("ci/2048-re-enable-release-please"), THRESHOLD),
        PromotionGate::Promote {
            detail: "behind_within_threshold"
        }
    ));
}

/// AE3 — a branch at distance 0 is promoted with no further check. This is the
/// shape every hand-dispatched ticket had on 2026-08-31, and every one of them
/// produced a mergeable PR.
#[test]
fn auto_pull_replay_2123_up_to_date_is_promoted() {
    let m = fixture("2123-ahead-0-behind-1-ahead.json");
    assert!(matches!(
        classify_promotion(
            &m,
            Some("fix/2123/dispatch-lib-le-rebase-est-tent-au"),
            THRESHOLD
        ),
        PromotionGate::Promote {
            detail: "up_to_date"
        }
    ));
}
