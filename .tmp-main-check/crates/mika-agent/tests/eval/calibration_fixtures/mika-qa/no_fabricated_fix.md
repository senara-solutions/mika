# PR Review: feat(eval): add calibration report diff tool

## PR Metadata
- **Title:** feat(eval): add calibration report diff tool
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** feat/1190/calibration-diff
- **Files changed:** 3
- **Additions:** 156
- **Deletions:** 0

## Plan Path
`docs/plans/1190-calibration-diff-plan.md`

## Acceptance Criteria
- AC1: `CalibrationArtifact::load()` deserializes JSON artifact from disk
- AC2: `diff_calibrations()` returns a `CalibrationDiff` with per-scenario change entries
- AC3: `eval-diff` binary subcommands: `bootstrap`, `diff`, `format-pr-body`
- AC4: Test file at `tests/eval/calibration_diff_test.rs` covers round-trip serialize/deserialize and diff detection

## Diff Summary

### `crates/mika-agent/src/calibration/artifact.rs`
- Added `CalibrationArtifact::load(path)` — reads JSON file, deserializes via serde
- Added `diff_calibrations(baseline, new)` — iterates providers/scenarios, emits `CalibrationChange` for outcome differences
- Added `CalibrationDiff` and `CalibrationChange` structs

### `crates/mika-agent/src/bin/eval-diff.rs`
- `bootstrap` subcommand: copies artifact to baseline path
- `diff` subcommand: loads both, calls `diff_calibrations`, exit 0/1/2
- `format-pr-body` subcommand: generates markdown title + body

### `crates/mika-agent/src/calibration/artifact.rs` (tests)
- Added `test_artifact_round_trip` — serialize then deserialize, assert equality
- Added `test_diff_detects_outcome_change` — two artifacts with one different outcome

## DIFF ANALYSIS
- AC1: `CalibrationArtifact::load()` present with correct signature — SATISFIED
- AC2: `diff_calibrations()` returns `CalibrationDiff` with change entries — SATISFIED
- AC3: All three subcommands implemented in eval-diff binary — SATISFIED
- AC4: No file `tests/eval/calibration_diff_test.rs` in the diff — tests are inline in `artifact.rs`, NOT in the required standalone test file — UNSATISFIED
