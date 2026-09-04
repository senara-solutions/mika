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

    let decision = classify_promotion(
        &m,
        Some("fix/1680/mika-dev-tui-broken-glyph-rendering-in"),
        THRESHOLD,
    );
    match decision {
        PromotionGate::Refuse(ref r) => {
            // Named, and named *correctly*: #1680 carries four `crates/**` files
            // on top of its plan, so the salvage rule is the honest reason — not
            // the distance, which is also true but is the more generic fact.
            assert_eq!(r.slug(), "salvage_work_on_stale_branch");
            // mika#2140 AC5, and the assertion no fixture without a `files` key
            // can satisfy: the refusal carries the files the operator must
            // judge, and carries only those — the plan file is not accused.
            // (That these reach the *message* is asserted in the unit tests,
            // which can see the private renderer.)
            let PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch {
                ref non_plan_files,
                ..
            }) = decision
            else {
                unreachable!("just matched a salvage refusal")
            };
            assert_eq!(
                non_plan_files,
                &vec![
                    "crates/mika-agent/src/agent_loop/mod.rs".to_string(),
                    "crates/mika-agent/src/evidence/guards.rs".to_string(),
                    "crates/mika-agent/src/evidence/mod.rs".to_string(),
                    "crates/mika-agent/src/well_known_agents.rs".to_string(),
                ]
            );
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

/// **The one verdict mika#2140 flips in the dangerous direction — named, not
/// silent.**
///
/// `ci/2048-re-enable-release-please` is 17 behind with a single commit that
/// touches three config files and **no plan at all**. Under `ahead_by > 1` it
/// promoted; under the file-based predicate it is refused, and the prefix is the
/// *sole* cause — the distance rule would have promoted it (17 < 50). That is
/// precisely the shape the plan's Fire-Disposition says must reopen the prefix
/// question rather than answer itself quietly, so it is asserted here in the
/// open instead of being deleted or widened away.
///
/// **Why it is nonetheless accepted, measured rather than argued.** This branch
/// cannot reach the gate in production:
///
/// - Phase 0 (`auto_pull.rs` `phase0_feed_ready_pool`) and Phase 1
///   (via `select_best_candidate`) both filter on `is_groomed`, which requires a
///   literal ``> - **Plan:** `docs/plans/`` callout. A branch with no plan file
///   has no such ticket.
/// - Phase 2 filters only on `ready`, so it *could* reach the gate — but the
///   live population is empty: of the 18 open mika tickets carrying a
///   `> - **Branch:**` callout on 2026-09-04, **zero** point at a branch with no
///   plan file.
/// - #2048 itself is CLOSED and carries no grooming callout at all.
///
/// The negative control this test used to provide — "behind, rebasable, still
/// promoted" — is not lost: it moves to the #2118/#2120 replays below, on
/// branches that are actually grooming branches, which is the population this
/// gate exists to judge.
#[test]
fn auto_pull_replay_2048_no_plan_file_is_refused_and_the_prefix_is_the_sole_cause() {
    let m = fixture("2048-diverged-17-behind-1-ahead.json");
    let StalenessMeasurement::Measured(s) = &m else {
        unreachable!()
    };
    // Self-cleaning half: the distance rule does *not* also refuse this branch.
    // If someone raises the threshold below 17, this assertion fires and the
    // test stops being about the prefix — which is the point.
    assert!(
        s.behind_by < THRESHOLD,
        "this test is only meaningful while the distance rule would promote"
    );
    assert_eq!(
        s.ahead_by, 1,
        "and while the old predicate would promote too"
    );

    match classify_promotion(&m, Some("ci/2048-re-enable-release-please"), THRESHOLD) {
        PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch {
            non_plan_files, ..
        }) => {
            assert_eq!(
                non_plan_files,
                vec![
                    ".github/workflows/release-pr.yml".to_string(),
                    "release-please-config.json".to_string(),
                    "version.txt".to_string(),
                ]
            );
        }
        other => panic!("expected a named salvage refusal, got {other:?}"),
    }
}

/// **AC3** — regression on the two real bodies the ticket measured.
///
/// Both branches carry nothing but their own plan file, over two and three
/// grooming commits respectively. Both were labelled `operator-gated` by the old
/// predicate and sat out of the pool for days.
///
/// **Non-vacuity is asserted first**, because a regression test that would also
/// have passed before the fix proves nothing: each fixture is checked to be
/// inside the *old* predicate's refusal zone (`behind_by > 0 && ahead_by > 1`)
/// before the new verdict is asserted. Replace either fixture with an
/// `ahead_by == 1` branch and this test fails rather than going quietly green.
#[test]
fn auto_pull_replay_2118_and_2120_multi_pass_grooming_promotes() {
    for (name, branch) in [
        (
            "2118-diverged-13-behind-3-ahead.json",
            "fix/2118/skills-cloud-sur-un-tenant-cloud-google",
        ),
        (
            "2120-diverged-13-behind-2-ahead.json",
            "fix/2120/auto-pull-is-groomed-exige-docs-plans",
        ),
    ] {
        let m = fixture(name);
        let StalenessMeasurement::Measured(s) = &m else {
            unreachable!()
        };
        assert!(
            s.behind_by > 0 && s.ahead_by > 1,
            "{name} must sit in the old predicate's refusal zone, else this \
             regression test is vacuous (behind {}, ahead {})",
            s.behind_by,
            s.ahead_by
        );
        assert!(
            s.changed_files
                .as_ref()
                .expect("fixture carries a file list")
                .iter()
                .all(|f| f.starts_with("docs/plans/")),
            "{name} must carry nothing but its plan"
        );
        assert!(
            matches!(
                classify_promotion(&m, Some(branch), THRESHOLD),
                PromotionGate::Promote {
                    detail: "behind_within_threshold"
                }
            ),
            "{name} must be promoted"
        );
    }
}

/// **The living positive control** (Fire-Disposition, surface 2).
///
/// `feat/1727/…` is the one open branch whose refusal survives the fix: it
/// carries a `crates/mika-cli/docs/…-audit-and-plan.md` alongside its plan — a
/// document, not code, which the narrow prefix classifies as "not grooming".
/// Literally true, semantically arguable.
///
/// The second assertion is the self-cleaning one: the refusal is
/// **overdetermined** — 190 behind, far past the threshold — so here the prefix
/// decides only the *reason*, never the *outcome*. The day someone freezes a
/// boundary case where the prefix is the sole cause of a refusal that would
/// otherwise have promoted, this assertion fails and the prefix question reopens
/// instead of answering itself in silence.
#[test]
fn auto_pull_replay_1727_is_the_measured_boundary_case() {
    let m = fixture("1727-diverged-190-behind-3-ahead.json");
    let StalenessMeasurement::Measured(s) = &m else {
        unreachable!()
    };
    assert!(
        s.behind_by > THRESHOLD,
        "#1727's refusal must stay overdetermined by distance ({} behind)",
        s.behind_by
    );

    match classify_promotion(
        &m,
        Some("feat/1727/tui-tui-as-thin-http-client-of-mika"),
        THRESHOLD,
    ) {
        PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch {
            non_plan_files, ..
        }) => {
            assert_eq!(
                non_plan_files,
                vec![
                    "crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md"
                        .to_string()
                ]
            );
        }
        other => panic!("expected a named salvage refusal, got {other:?}"),
    }
}

/// AE3 — a branch at distance 0 is promoted with no further check. This is the
/// shape every hand-dispatched ticket had on 2026-08-31, and every one of them
/// produced a mergeable PR.
///
/// This fixture deliberately carries **no** `files` key (mika#2140): its branch
/// was merged and deleted from origin, so it can no longer be recaptured — and
/// it does not need to be. `behind_by == 0` short-circuits before the salvage
/// rule is ever reached, so the file list is never read on this path. It doubles
/// as the integration-level control that a payload without `files` still parses.
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
