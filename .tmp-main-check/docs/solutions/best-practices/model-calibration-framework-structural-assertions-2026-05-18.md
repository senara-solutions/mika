---
module: eval
date: 2026-05-18
problem_type: best_practice
component: testing_framework
severity: high
tags:
  - model-calibration
  - pre-swap-gate
  - eval-harness
  - structural-assertions
  - regression-prevention
applies_when: "Swapping any agent's base model or skill-override model (mika-dev, mika-arch, future roles)"
---

# Model Calibration Framework — Structural Assertions for Pre-Swap Gating

## Context

The 2026-05-07 kimi-to-sonnet model swap for mika-dev deployed without a calibration gate, producing three downstream incidents in 10 days: mika#1168 (refusal regression), mika#1166 (dev-groom contract violation), and mika#1173 (handler regression). The `project_model_calibration` memory entry (2026-04-23) established the principle but the framework was never built.

## Guidance

**Structural assertions (trace-shape, regex, keyword-presence) are sufficient for model-swap gating.** Semantic LLM-as-judge evaluation is orthogonal — it measures quality, not swap-readiness.

The calibration framework lives at `crates/mika-agent/src/calibration/` and provides:

1. **Role-scoped scenarios** — each scenario sends a fixture prompt to the target model and checks the response against structural criteria (no refusal patterns, correct disposition keyword, finding list presence, etc.)
2. **FailureClass taxonomy** — Refusal, Fabrication, EmptyResponse, Timeout, TransportError, ContractViolation, Other — maps 1:1 to known regression classes
3. **`calibrate` binary** — `cargo run --bin calibrate -- --role <role> --model <provider/model>` produces JSON artifact + markdown report
4. **Makefile targets** — `make calibrate-mika-dev MODEL=...` / `make calibrate-mika-arch MODEL=...`

### Key design decisions

- **Scenarios are Rust, not YAML** — scoring logic (regex, trace inspection) doesn't compress into declarative form. Fixtures (inputs) are markdown; assertions are code.
- **Binary, not test target** — exit code IS the gate (0=pass, 1=fail). Operators run it explicitly, CI consumes the exit code.
- **N=1 single-shot** — each scenario runs once. Cost: ~10 LLM calls per role. Multi-run averaging is v2.
- **No LLM-as-judge** — every assertion is mechanically checkable from the response text. No non-deterministic grading.

## Why This Matters

Without a calibration gate, every model swap is a fresh production incident. The three 2026-05-07 incidents burned collective hours of debugging that a 30-second `make calibrate-mika-dev` run would have caught pre-deploy.

The framework makes model exploration (grok-4, gpt-5, sonnet-4.7, opus-4.7) safe: operators get a quantitative pass/fail before any swap lands.

## When to Apply

- Before merging any PR that changes `MIKA_ANTHROPIC_API_KEY` routing, model defaults in `identity.toml`, or `skill_overrides.llm_model`
- Before deploying a model downshift for cost optimization
- When evaluating a new provider for an existing role
- When a skill's system prompt changes significantly (re-calibrate to verify the model still produces correct output shape)

## Examples

```bash
# Pre-swap gate for mika-dev on sonnet-4.7
make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-7

# Compare mika-arch on opus vs sonnet for cost optimization
make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6
make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6
# Compare reports side-by-side

# With baseline comparison (fails if pass rate drops)
cargo run --bin calibrate -- --role mika-dev \
  --model anthropic/claude-sonnet-4-6 \
  --baseline docs/eval/calibration/baselines/latest.json
```

## Related

- mika#1190 — framework implementation ticket
- mika#1168 — refusal regression that motivated the framework
- `crates/mika-agent/CLAUDE.md § Evaluation — Model Calibration` — technical reference
- `docs/plans/2026-05-17-003-feat-1190-eval-model-calibration-framework-plan.md` — groomed plan
- PR: https://github.com/senara-solutions/mika/pull/1195
