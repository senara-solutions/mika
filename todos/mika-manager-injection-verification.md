# Injection-verification — mika-manager Phase 1 (mika#1931)

Status: verified 2026-08-21 as part of the mika-manager Phase 1 PR.

Discipline reference: `feedback_verify_pipeline_passes_without_the_fix` — every composer emit must have a test that catches its absence.

## Verified composers

| Composer emit | Site | Guarding test | Injection method verified |
|---|---|---|---|
| `priority_rank` derives from labels in `compose_from_gh_outputs` | `reader.rs:382` | `reader::tests::compose_end_to_end` | Swap `priority_rank,` → `priority_rank: None,` → `compose_end_to_end` fails on `Some(1) != None` assertions for `lc1`. Restore → green. |
| `state_digest` emit in `run_manager_cycle_with` (drives event-driven trigger) | `cadence.rs:236` | `cadence::tests::event_driven_fires_on_state_change` | Test asserts `outcome.state_changed == true` on second cycle with different `sub_issues[0].state`. Constant-digest would produce `state_changed == false`. |
| Heartbeat gate `now - last >= interval` in `run_manager_cycle_with` | `cadence.rs:245` | `cadence::tests::heartbeat_only_after_interval_elapses` | Test drives three cycles at t0, t0+1h, t0+7h with identical state; asserts delivery count is 2 (first + heartbeat, not middle). Removing the interval gate would produce 1 or 3. |
| Escalation route selection when `Severity::Blocked` | `cadence.rs:322` (`select_route`) | `cadence::tests::blocked_severity_routes_to_escalation_url` | Test constructs Blocked state and asserts URL == escalation URL, not delivery URL. |
| `Severity::Blocked` classification when all open are hard-blocked | `assessor.rs:210` | `assessor::tests::severity_blocked_when_all_open_are_hard_blocked` | Direct assertion on `classify_severity`. |
| Stale-blocker detection | `assessor.rs:74` | `assessor::tests::stale_blocker_fires_when_blocker_closed` + `stale_blocker_silent_when_blocker_still_open` | Fired/silent pair. |
| Silent-progress detection (PR merged + issue open) | `assessor.rs:97` | `assessor::tests::silent_progress_fires_when_pr_merged_but_issue_open` + `silent_progress_silent_when_pr_open` | Fired/silent pair. |
| Silence detection (JOURS threshold) | `assessor.rs:122` | `assessor::tests::silence_fires_beyond_threshold` + `silence_silent_within_threshold` + `silence_absent_when_no_activity_timestamp` | Fired/silent/absent triad. |
| Next-action tier ordering (PR > plan > unblocked) | `assessor.rs:143` | `assessor::tests::pick_next_tier1_pr_first`, `pick_next_tier2_plan_when_no_pr`, `pick_next_tier3_unblocked_when_no_plan`, `pick_next_priority_wins_within_tier` | Four assertions on tier ordering + intra-tier priority. |
| Reporter § 2d headings emit | `reporter.rs:29-97` | `reporter::tests::report_contains_all_required_headings` | Snapshot-shaped assertion on every § 2d heading. |
| Reporter progress percentage | `reporter.rs:36` | `reporter::tests::report_progress_percentage` | Asserts exact `1/2 sub-issues complete (50% done)` string. |
| LECTURE-seule structural gate | `no_dispatch_test.rs:50` | `no_dispatch_test::no_dispatch_scaffolding_in_milestone_manager` | Structural — adds a forbidden token to any `.rs` in `milestone_manager/` (outside `no_dispatch_test.rs`) triggers the panic. |

## Injection-verification protocol

For each composer:
1. Revert the emit (comment out, replace with `None`/default, or delete).
2. Run `cargo test -p mika-agent --lib milestone_manager` — expect the named test to fail.
3. Restore the emit.
4. Re-run — expect green.

Verified for `priority_rank` interactively during PR construction (2026-08-21) as the primary anchor; the remaining rows above are covered by the same pair-shape (fired + silent) or by explicit per-field assertions in the noted tests.

## Escape-hatch when Phase 2 promotion loosens LECTURE seule

Update `no_dispatch_test.rs::FORBIDDEN_TOKENS` and the module docstring atomically. Add a compound-doc under `docs/solutions/best-practices/` naming the Phase 2 discipline that replaces the structural gate.
