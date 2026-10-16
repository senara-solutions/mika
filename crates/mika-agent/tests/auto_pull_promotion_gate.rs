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
///
/// **This is also the named negative control of the mika#2170 slug split, and
/// the comment has to say so.** #1680 carries four `crates/**` files *on top of
/// its plan*, so it stays `salvage_work_on_stale_branch` while a plan-less
/// branch moves to `stale_branch_without_plan`. Read without this paragraph,
/// the slug assertion below looks like an arbitrary string check that a future
/// reader would "harmonise" with its neighbour on the #2048 fixture — and the
/// split would stop being tested in the only direction that can go wrong
/// silently, the one where the *old* name must survive.
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
/// **mika#2170 changed what carries that last conclusion, not the date.** The
/// 2026-09-04 measurement still says the population was empty *that day*; what
/// it never was, is a guard. It used to rest on a dormant ticket whose wake
/// condition could not be evaluated (`non_plan_files` is `docs/plans/` filtered
/// out, so the condition was trivially true of every salvage refusal). It now
/// rests on a slug an operator can `GROUP BY`: this fixture is the **frozen
/// positive population** of `stale_branch_without_plan`, and the day a real
/// branch joins it, one query says so. The refusal itself is unchanged — only
/// its name and its wording are.
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
        PromotionGate::Refuse(RefusalReason::StaleBranchWithoutPlan {
            ref changed_files, ..
        }) => {
            // The mika#2140 assertion, kept whole: the refusal carries the files
            // the operator must judge. On this population `changed_files` and
            // `non_plan_files` are the same three paths — which is exactly the
            // fact the new variant names.
            assert_eq!(
                changed_files,
                &vec![
                    ".github/workflows/release-pr.yml".to_string(),
                    "release-please-config.json".to_string(),
                    "version.txt".to_string(),
                ]
            );
        }
        other => panic!("expected a named plan-less refusal, got {other:?}"),
    }

    // mika#2170 AC4 — the slug the operator's `GROUP BY` reads, and the one word
    // the message must have lost: this branch may well be finished work.
    let PromotionGate::Refuse(r) =
        classify_promotion(&m, Some("ci/2048-re-enable-release-please"), THRESHOLD)
    else {
        unreachable!("just matched a refusal")
    };
    assert_eq!(r.slug(), "stale_branch_without_plan");
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
///
/// **Second negative control of the mika#2170 split** (#1680 is the first).
/// #1727 carries its plan *and* one document outside the prefix, so it keeps
/// `salvage_work_on_stale_branch`: the split turns on whether a plan file is
/// present at all, never on whether the offending path looks like code. Pinning
/// it here is what keeps the two questions — "is this a grooming branch?" and
/// "is this document or code?" — from being quietly merged, the second being
/// precisely the policy decision mika#2140 reserved and D8 refuses.
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

    let branch = Some("feat/1727/tui-tui-as-thin-http-client-of-mika");
    match classify_promotion(&m, branch, THRESHOLD) {
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

    // mika#2170 AC5 — the negative control, asserted on the slug an operator
    // actually queries rather than only on the variant a Rust reader sees.
    let PromotionGate::Refuse(r) = classify_promotion(&m, branch, THRESHOLD) else {
        unreachable!("just matched a refusal")
    };
    assert_eq!(r.slug(), "salvage_work_on_stale_branch");
    // Non-vacuity of *this* control: the branch really does carry a plan, which
    // is the only reason it keeps the old name.
    assert!(
        s.changed_files
            .as_ref()
            .expect("fixture carries a file list")
            .iter()
            .any(|f| f.starts_with("docs/plans/")),
        "#1727 must carry its plan, else it is no longer a control of the split"
    );
}

/// **mika#2170 AC3 / R5 — the patch renames refusals; it decides nothing.**
///
/// The one thing this work must not do is move a branch across the gate. So the
/// partition is pinned whole, over every frozen fixture at once: which branches
/// promote, which refuse, and — for the refusals — under which of the four
/// slugs. Written as a literal table, so a change of verdict shows up as a diff
/// a reviewer reads rather than as a test that silently still passes.
///
/// It is deliberately redundant with the per-fixture tests above. Those explain
/// *why* each branch lands where it does and would each need its own edit;
/// this one makes a decision change impossible to ship as a one-line edit to a
/// single test. The two refusal names that appear here are the split, side by
/// side: `stale_branch_without_plan` on the one branch carrying no plan,
/// `salvage_work_on_stale_branch` on the two that carry one.
#[test]
fn auto_pull_replay_the_promote_refuse_partition_is_unchanged() {
    let expected: [(&str, &str, &str); 7] = [
        // fixture, branch, verdict (promote detail, or refusal slug)
        (
            "1680-diverged-180-behind-2-ahead.json",
            "fix/1680/mika-dev-tui-broken-glyph-rendering-in",
            "salvage_work_on_stale_branch",
        ),
        (
            "1727-diverged-190-behind-3-ahead.json",
            "feat/1727/tui-tui-as-thin-http-client-of-mika",
            "salvage_work_on_stale_branch",
        ),
        (
            "1959-diverged-75-behind-1-ahead.json",
            "feat/1959/mcp-manifest-data-grade-field-l4-forward",
            "branch_too_far_behind",
        ),
        (
            "2048-diverged-17-behind-1-ahead.json",
            "ci/2048-re-enable-release-please",
            "stale_branch_without_plan",
        ),
        (
            "2118-diverged-13-behind-3-ahead.json",
            "fix/2118/skills-cloud-sur-un-tenant-cloud-google",
            "behind_within_threshold",
        ),
        (
            "2120-diverged-13-behind-2-ahead.json",
            "fix/2120/auto-pull-is-groomed-exige-docs-plans",
            "behind_within_threshold",
        ),
        (
            "2123-ahead-0-behind-1-ahead.json",
            "fix/2123/dispatch-lib-le-rebase-est-tent-au",
            "up_to_date",
        ),
    ];

    // The refused set and the promoted set, named rather than counted: a count
    // survives two branches swapping sides, which is the failure this guards.
    let mut refused: Vec<&str> = Vec::new();
    let mut promoted: Vec<&str> = Vec::new();
    for (name, branch, verdict) in expected {
        match classify_promotion(&fixture(name), Some(branch), THRESHOLD) {
            PromotionGate::Refuse(r) => {
                assert_eq!(r.slug(), verdict, "{name}: refusal slug changed");
                refused.push(name);
            }
            PromotionGate::Promote { detail } => {
                assert_eq!(detail, verdict, "{name}: promotion detail changed");
                promoted.push(name);
            }
        }
    }

    assert_eq!(
        refused,
        vec![
            "1680-diverged-180-behind-2-ahead.json",
            "1727-diverged-190-behind-3-ahead.json",
            "1959-diverged-75-behind-1-ahead.json",
            "2048-diverged-17-behind-1-ahead.json",
        ]
    );
    assert_eq!(
        promoted,
        vec![
            "2118-diverged-13-behind-3-ahead.json",
            "2120-diverged-13-behind-2-ahead.json",
            "2123-ahead-0-behind-1-ahead.json",
        ]
    );
    // Non-vacuity: this is a partition, so neither side may be empty — a table
    // that accidentally lost its promotions would otherwise pass.
    assert_eq!(refused.len() + promoted.len(), 7);
    assert!(!refused.is_empty() && !promoted.is_empty());
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
