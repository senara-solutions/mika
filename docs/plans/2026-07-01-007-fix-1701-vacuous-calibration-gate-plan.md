---
type: fix
issue: 1701
title: Fix vacuous swap-gate — calibrate binary must hard-fail on pass-rate < 100% + baseline auto-load
status: draft
---

# Plan — mika#1701 vacuous calibration gate

## Ticket

mika#1701 — swap-gate is vacuous. Two independent defects: (a) the only `exit(1)` is unreachable because baseline file doesn't exist at Makefile-supplied path; (b) the standalone floor-gate at `calibrate.rs:293-297` has empty body. Real bug in mika#1190 discipline enforcement — every past swap decision was made against unenforced evidence.

## Problem

Two failure paths both silently exit 0:

**Defect 1:** `calibrate.rs:272` — the only hard-fail exit lives inside `if current_unweighted_rate < baseline_rate` (line 266), which only runs when the `--baseline` file loads. Makefile targets pass `docs/eval/calibration/baselines/latest.json` which doesn't exist. Real committed baselines live in per-issue directories (`docs/eval/calibration/mika-dev-1633/`, etc.). Warning printed, exit 0.

**Defect 2:** `calibrate.rs:293-297` — the standalone floor-gate:
```rust
if report.pass_rate < 1.0 && args.baseline.is_some() {
    // Only fail hard if there's a baseline to compare against
    // Without baseline, we're just establishing one
}
```
Empty body. Fires only when baseline exists AND pass-rate < 1.0, but even then does nothing.

**Consequence:** every model-swap decision made under this gate was against unenforced evidence. Vincent's owned-model bet (glm-teacher distillation) will use this gate to validate future mika-7B QLoRA candidates. **This gate must be real before that path activates.**

## Scope

**In scope (v1 ships):**

1. **Defect 2 fix (blocking).** Populate the standalone floor-gate body. When `pass_rate < 1.0`, print a clear diagnostic (which scenario(s) failed, pass_rate delta from 1.0) and `std::process::exit(1)`.
2. **Defect 1 fix (blocking).** Two paths for the baseline path issue:
   - **Path A**: fix the Makefile to point at the latest per-role baseline file (e.g., `docs/eval/calibration/mika-dev-1633/latest.md` symlinked to `latest.json`). Requires updating the Makefile plus establishing a canonical per-role "latest" convention.
   - **Path B**: make baseline discovery smarter — the binary tries `docs/eval/calibration/baselines/latest.json` first, then walks per-issue dirs to find the newest, then falls back to warn-only if none. Robust to file-naming conventions.
   - **Committed choice**: **Path A** — simpler, ships faster, matches motto. Add symlink or move-to-canonical-path.
3. **Guardrail against silent-pass regression.** Add a test: `cargo test -p mika-agent --test calibrate_integration` — invoke calibrate binary with a mock that returns pass_rate < 1.0 → assert exit code non-zero. Prevents future refactor from re-introducing the vacuous state.
4. **Commit baseline files under the canonical path.** Move-or-symlink `docs/eval/calibration/{mika-dev-1633,mika-arch-<>,mika-qa-1632}/latest.json` to `docs/eval/calibration/baselines/mika-{dev,arch,qa}.json`. Update the Makefile if needed.
5. **Documentation.** Update `mika/CLAUDE.md § Model calibration (#1190)` to describe the now-real gate contract: pass_rate < 100% → exit 1; missing baseline → exit 2 with clear error; passing gate → exit 0.

**Out of scope:**

- Redesigning the calibration scenarios themselves (separate axis; mika#1699 addresses permission-policy specifically).
- Making the gate stricter than pass_rate < 1.0 (per-scenario weight enforcement, latency thresholds).
- Auto-generating baselines on first run (would make the gate self-defeating).

## Committed positions

1. **Empty body is the primary bug.** Fix that first (Defect 2). Even without the Makefile path fix, populating the body means anyone who passes a valid `--baseline` gets a real gate.
2. **Path A over Path B for baseline discovery.** Simpler shape; the walk-and-discover approach adds complexity without clear benefit given we control both the Makefile and the baseline file locations.
3. **Exit 2 for missing baseline (not exit 0).** Currently warns and exits 0. That's misleading. Missing baseline = "gate is not enforceable" = exit 2 (distinct from pass_rate failure exit 1). Operators must supply a baseline OR opt in to no-gate mode via a flag.
4. **`--establish-baseline` flag for the "first run" case.** Currently the tool tries to double-purpose: gate OR baseline-establishment. Split explicitly: default = gate mode (baseline required), `--establish-baseline` = writes new baseline + exits 0 regardless.
5. **Integration test is load-bearing.** This is a swap-gate discipline bug that regressed unnoticed for months. The test prevents it happening again.

## Acceptance criteria

- **AC1** — `calibrate.rs:293-297` empty body replaced with a diagnostic-printing `std::process::exit(1)`. Diagnostic names the failing scenarios + pass-rate delta.
- **AC2** — Missing `--baseline` file → exit 2 with clear error ("baseline path does not exist: <path>. Pass --establish-baseline to write a new one, or provide a valid baseline path"). No silent exit 0.
- **AC3** — New `--establish-baseline` flag: when set, writes current run's artifact as the baseline + exits 0 regardless of pass-rate. Documented in `calibrate.rs` doc-comment.
- **AC4** — Makefile targets updated: use canonical `docs/eval/calibration/baselines/mika-{dev,arch,qa}.json` paths. Baselines committed at those paths (moved or symlinked from per-issue dirs).
- **AC5** — Integration test: `cargo test -p mika-agent --test calibrate_integration` — invokes binary with pass_rate=0.6 (one scenario failed via mock) + valid baseline → asserts exit 1. Second test case: pass_rate=1.0 + valid baseline → asserts exit 0. Third: no baseline → asserts exit 2.
- **AC6** — Docs update in `mika/CLAUDE.md § Model calibration (#1190)`: gate contract described (exit codes 0/1/2), `--establish-baseline` documented.
- **AC7** — Regression check: `cargo test -p mika-agent` clean.

## Implementation steps

**Phase 1 — Defect 2 fix (calibrate.rs body).** Populate the empty conditional at :293-297 with `eprintln!` diagnostic + `std::process::exit(1)`. ~10 lines.

**Phase 2 — Defect 1 fix (baseline discovery + missing-file handling).** Change the `else` at :286-290 to exit 2 (not just warn). Add `--establish-baseline` flag. ~40 lines.

**Phase 3 — Baseline files at canonical path.** Move or symlink existing baselines from per-issue dirs to `docs/eval/calibration/baselines/mika-{dev,arch,qa}.json`. Update Makefile if the paths need adjustment.

**Phase 4 — Integration test.** Author `tests/eval/calibrate_integration.rs` with three test cases (fail, pass, missing-baseline).

**Phase 5 — Docs.** Update `mika/CLAUDE.md § Model calibration` with the new gate contract.

## Verification

- **Manual test 1:** `make calibrate-mika-dev MODEL=openrouter/z-ai/glm-5.2` with current committed baseline → expect exit 0 (100% pass).
- **Manual test 2:** Same command with a broken model spec → expect exit 1 with diagnostic naming failed scenarios.
- **Manual test 3:** `make calibrate-mika-dev MODEL=... BASELINE=/nonexistent/path.json` → expect exit 2 with clear error.
- **Manual test 4:** `make calibrate-mika-dev MODEL=... --establish-baseline` → exit 0, new baseline written.
- **Integration test:** All three AC5 cases pass.
- **CI:** `cargo test -p mika-agent` clean.

## Risks

1. **Existing baselines may be schema-drifted.** The `docs/eval/calibration/{mika-dev-1633,...}/latest.json` files may have been written with an older `CalibrationArtifact` schema. Moving them to canonical path may still need a schema-migration step. Mitigation: baseline load already returns a warning on schema mismatch; the gate should treat schema mismatch as exit 2 (same as missing), not silent-pass.
2. **Deploy timing.** Every model-swap PR from now uses this gate. Land + deploy before any pending swap PRs try to use it — otherwise those PRs regress.
3. **Establishing baselines is a chicken-and-egg problem.** If baseline files are corrupted or missing on prod, every calibrate run exits 2 until baselines are re-established. Mitigation: `--establish-baseline` flag lets operators re-establish cleanly; document this in mika/CLAUDE.md.
4. **Per-scenario weight enforcement.** Current AC1 fires on `pass_rate < 1.0`. If a scenario has `weight: 0.5`, is 4/5 with the half-weight scenario passing = pass_rate 0.9 (fail) or 0.95 (weighted-pass-fail)? Current baseline compare uses unweighted count (line 244 comment); floor gate should too for consistency.
5. **The Makefile change breaks any external scripts that call calibrate directly.** Low risk — only mika-platform + wizzard likely to use these targets.

## Out of scope (repeated)

- Redesigning the scenarios (separate axis).
- Making the gate stricter (per-scenario latency thresholds, etc.).
- Auto-baseline-on-first-run.

## References

- mika#1701 — this ticket
- wizzard-CC's finding — surfaced during wizzard#16 trajectory-corpus feasibility spike
- mika#1704 — duplicate closed as dup of this
- [[project-mika-owned-model-dev-qa-quality-first]] — the framework this gate protects
- `crates/mika-agent/src/bin/calibrate.rs:272` — Defect 1 unreachable exit
- `crates/mika-agent/src/bin/calibrate.rs:293-297` — Defect 2 empty body
- `Makefile:83,87,91` — the wrong-path invocations
- `docs/eval/calibration/mika-dev-1633/`, `mika-dev-1657/`, `mika-qa-1632/` — existing per-issue baselines
- mika#1190 — the discipline this bug undermines
- Vincent's owned-model direction 2026-07-01 — future gate consumer
