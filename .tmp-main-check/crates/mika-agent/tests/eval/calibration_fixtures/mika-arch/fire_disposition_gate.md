# Fire-Disposition Gate: Detector Plan Missing Fire-Disposition (ITERATE)

## Plan Under Review: docs/plans/1574-test-byte-slice-lint.md

### Summary

Add a CI lint that rejects unsafe `&str` byte-slicing patterns (`s[..n]`,
`&s[a..b]`) across the `mika-agent` crate. These patterns panic on multi-byte
UTF-8 boundaries. The lint is a `scripts/check-byte-slices.sh` grep-based
detector wired into the `byte-slice-lint` CI job.

### Design

- **Detector:** `scripts/check-byte-slices.sh` greps all `crates/**/*.rs` for
  the byte-slice pattern and exits non-zero when any match is found.
- **CI wiring:** A new `byte-slice-lint` job in `ci.yml` runs the script on
  every pull request.
- **Success path:** The script exits `0` when zero matches are found — "no
  violations" is the green state.

### Implementation Units

#### Unit 1: Detector script

Add `scripts/check-byte-slices.sh` that greps for `\[\.\.` and `\[[0-9a-z_]+\.\.`
byte-index slice patterns on `&str`-typed expressions and reports each hit with
`file:line`.

#### Unit 2: CI job

Add a `byte-slice-lint` job to `.github/workflows/ci.yml` that runs the script
and fails the build on non-zero exit.

### Test Plan

- Unit: fixture strings that should and should not trip the grep.
- Integration: run the script against the current tree in CI.

### Migration

None — additive script + CI job.

## Definition of Done

- `scripts/check-byte-slices.sh` exists and is executable.
- The `byte-slice-lint` CI job runs the script on every PR.

## Acceptance criteria

1. `scripts/check-byte-slices.sh` exits non-zero when a byte-slice pattern is
   present and zero when none is present.
2. The `byte-slice-lint` CI job is wired into `ci.yml` and runs on every PR.
3. The detector reports each violation with a `file:line` reference.
