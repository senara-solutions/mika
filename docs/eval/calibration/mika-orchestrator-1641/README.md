# mika-orchestrator calibration baseline (mika#1641 AC2)

This directory holds the committed calibration baseline for the **mika-orchestrator**
role — the platform-orchestrator seat Mika (the executive-assistant agent) assumes
per mika#1641.

## Status: baseline pending an operator/CI run

The calibration **suite** ships in this PR and is fully wired:

- Role module: `crates/mika-agent/src/calibration/roles/mika_orchestrator.rs` (5 scenarios)
- Fixtures: `crates/mika-agent/tests/eval/calibration_fixtures/mika-orchestrator/`
- Runner: `make calibrate-mika-orchestrator MODEL=<provider/model>`
- Compile/test-verifiable invariants: `cargo test -p mika-agent calibration::roles::mika_orchestrator`

The **baseline artifact** (JSON + markdown showing a 100% pass on the chosen
production model) is **not** committed by this PR, by design. Generating it
requires a live LLM run — real provider API keys, network, and a resolved model
choice. Committing a hand-written "100% pass" JSON would be fabricated evidence,
which is precisely the failure mode the orchestrator calibration exists to
prevent (see the `ticket_framing_hard_evidence` and grounding-discipline
scenarios). The baseline is produced by actually running the gate.

## Open decision: which production model?

The baseline must be captured against the model the orchestrator seat will run in
production. That choice is an open grooming/operator question (plan Risk #1):

- **glm-5.2 (keep-cheap):** Mika's current model. Cheapest. Has documented
  behavioral quirks (CJK mika#1680, auto-undraft mika#1682) that may cost pass-rate
  on the judgment-dense orchestrator scenarios.
- **sonnet-4-6 (up-tier):** Orchestrator work is low-volume, high-judgment-density —
  possibly worth up-tiering even though dev/qa stay on glm-5.2 (the "quality first"
  call is a dev/qa scope decision, not a hard commit for the orchestrator seat).

Calibration is how this gets decided: run both, compare pass-rates, pick the
cheapest model that clears 100%.

## How to generate and commit the baseline

1. Ensure the chosen provider's API key is set (same env as mika-spirit — e.g.
   `MIKA_ANTHROPIC_API_KEY` or `MIKA_ZAI_API_KEY`).
2. Run the gate against the candidate model:
   ```bash
   make calibrate-mika-orchestrator MODEL=zai/glm-5.2
   # or
   make calibrate-mika-orchestrator MODEL=anthropic/claude-sonnet-4-6
   ```
3. The binary writes a JSON artifact + markdown report to
   `target/eval-calibration/mika-orchestrator-<timestamp>.{json,md}` (override with
   `--output`). Inspect the markdown report — the pass rate must read `100.0% (5/5)`.
4. If any scenario fails, read the failure class in the report. Either the model is
   not ready for the seat (try the other candidate) or a scenario/fixture needs
   tightening. Do NOT lower the bar to force a pass.
5. On a clean 100% run, copy the artifacts here with the canonical name (mirrors the
   `mika-qa-1632/` and `mika-dev-1633/` sibling baselines):
   ```
   docs/eval/calibration/mika-orchestrator-1641/mika-orchestrator-<model>-post-1641.json
   docs/eval/calibration/mika-orchestrator-1641/mika-orchestrator-<model>-post-1641.md
   ```
   e.g. `mika-orchestrator-glm-5.2-post-1641.json`.
6. Commit the baseline. Per mika#1190, any future orchestrator model swap must
   include an updated baseline from a passing `make calibrate-mika-orchestrator` run.

## Verification of the suite (no network required)

The suite's structural invariants are unit-tested and run in CI without any LLM
call:

```bash
cargo test -p mika-agent calibration::roles::mika_orchestrator
```

This asserts the 5-scenario set, unique IDs, tag coverage, the seed-name match,
and that each fixture contains the grounding signals its scenario keys on.
