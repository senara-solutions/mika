---
module: mika-agent
tags: [qa-review, eval, provider-calibration, cost-optimization, deepseek]
problem_type: investigation
category: architecture
issue: 1092
date: 2026-05-13
---

# Phase 0 — qa-review provider A/B pre-flight investigation

**Issue:** mika#1092
**Decision:** PROCEED with backfill prerequisite (Phase 0.5 ticket needed)

## Q2: DeepSeek effective context window (hard blocker gate)

**System prompt size:** `skills/bundled/qa-review/system_prompt.md` = 39,662 chars ≈ ~10k tokens.

**Actual mika-qa token distribution** (from `llm_calls` table, 19,221 calls over 31 days):

| Metric | Input tokens | Output tokens |
|--------|-------------|---------------|
| Average (all calls) | 990 | 67 |
| p50 | 0 (most calls are empty/small multi-turn) | — |
| p95 | 10,007 | — |
| Max | 38,931 | 258 |
| Avg (full PR reviews, >10k input) | 13,646 | 268 |

**Token bucket distribution:**

| Bucket | Count | % |
|--------|-------|---|
| 0 (empty) | 15,836 | 82.4% |
| 1-999 | 751 | 3.9% |
| 1k-5k | 1,391 | 7.2% |
| 5k-10k | 280 | 1.5% |
| 10k-20k | 929 | 4.8% |
| 20k-32k | 24 | 0.1% |
| 32k-64k | 10 | 0.05% |

**Key finding:** 82.4% of mika-qa calls are empty/zero-token (likely multi-turn conversation continuations where caching handles input). The actual full-context PR review calls (>10k input) number 962, with an average of 13,646 input tokens. Even the worst case (38,931 tokens) is well under DeepSeek V3's marketed 128k context window.

**DeepSeek provider config** (from `crates/mika-common/src/llm/mod.rs`): Default model `deepseek-chat`, max output tokens 8,192. No explicit context window limit configured — relies on API provider default.

**Verdict:** Representative total (13,646 avg) is well under 32k. Even the max (38,931) is under 64k. DeepSeek is viable for qa-review prompts. The 10 calls in the 32k-64k range (0.05% of traffic) may need monitoring but are not a blocker.

**Truncation behavior:** Not empirically verified in this investigation (would require a real-provider test run). DeepSeek's API returns HTTP 400 on context overflow rather than silently truncating — this is the safe failure mode. Recommend verifying with one real-provider eval run in Phase 1.

## Q1: Eval corpus size

**Current eval corpus for qa-review:**

| Location | Count | Type |
|----------|-------|------|
| `tests/eval/grounding_regressions/qa_review_*.rs` | 2 | Mock-response regression tests |
| `tests/eval/golden/skill_qa_review_bug_catch.rs` | 1 | Mock-response golden scenario |
| **Total qa-review-specific eval scenarios** | **3** | **All mock-response** |

**Real PR replay scenarios:** 0. The eval harness (mika#1059) uses `MockLlmProvider` with canned responses for CI. No real PR diff + expected verdict pairs exist as a corpus.

**Historical data available for backfill:** 19,221 mika-qa LLM calls from 2026-04-12 to 2026-05-13 (31 days). Of these, 962 are full-context PR review calls (>10k input tokens). This is a rich backfill source — each call contains the actual input tokens (system prompt + PR diff + context) and the model's output (verdict).

**Applying decision framework:** Corpus < 25 real PR scenarios → **PROCEED with backfill prerequisite**. A Phase 0.5 backfill ticket should:
1. Target 50 scenarios (25 minimum for directional comparison).
2. Extract from `llm_calls` table: capture real qa-review invocations with their full input context and verdicts.
3. Ensure diversity: approved PRs, changes-requested, docs-only, large multi-file diffs, dependency bumps.
4. Estimated effort: ~2hr for automated extraction script.

## Q3: Eval harness traffic mode

**Confirmed modes** (from `crates/mika-agent/tests/eval/harness.rs`):

1. **Mock mode** (default): `MockLlmProvider` with canned responses. Deterministic, runs on CI. Entry: `EvalHarness::builder().responses(vec![...]).build()`.
2. **Real-provider mode**: `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` env var (line 16 of `tests/eval/providers.rs`). Supports comma-separated provider list or `all`. Creates real providers from env API keys.
3. **Calibration mode**: `MIKA_EVAL_CALIBRATE=1` writes JSON artifacts to `target/eval-calibration/` with per-provider per-scenario outcomes, token counts, and latency.

**Real-provider matrix** (`tests/eval/test_real_provider_matrix.rs`): Runs the same scenario set against all configured providers sequentially. Collects per-provider outcomes in a single test run. Supports offline batch comparison natively.

**What does NOT exist:**
- No live A/B traffic split mechanism.
- No canary/shadow traffic mode.
- No runtime provider routing based on experiment assignment.

**Verdict:** Test-time only (replay). Phase 2 A/B will be offline batch comparison: run captured PR scenarios against each provider via `MIKA_EVAL_REAL_PROVIDERS=<provider>`, compare verdicts using calibration artifacts. The existing harness supports this natively — no Phase 2 infra ticket needed for the comparison mechanism.

## Q4: Eval-run cost budget

**Per-scenario cost estimates** (using actual avg for full PR reviews: 13,646 input / 268 output tokens):

| Provider | Input cost/M | Output cost/M | Per-scenario cost |
|----------|-------------|---------------|-------------------|
| Sonnet 4.6 | $3.00 | $15.00 | $0.045 |
| DeepSeek V3 | $0.27 | $1.10 | $0.004 |
| Kimi K2.5 | $0.60 | $2.00 | $0.009 |

**Phase cost projections (at 50 scenarios):**

| Phase | Variants × Scenarios | Sonnet | DeepSeek | Kimi | Total |
|-------|---------------------|--------|----------|------|-------|
| Phase 1 baseline | 1 × 50 | $2.25 | — | — | $2.25 |
| Phase 2 A/B | 3 × 50 | $2.25 | $0.20 | $0.45 | $2.90 |

**Worst-case estimate** (using max observed input 38,931 tokens, 50 scenarios, 3 variants): $7.77 total for Phase 2.

**Budget tracking:** Eval costs are tracked in the `llm_calls` table — same as production calls. No separate eval-budget tracking exists. Eval runs are indistinguishable from production calls in telemetry. Dashboard surfaces all LLM costs per agent.

**Verdict:** Even worst-case Phase 2 cost ($7.77) is trivially under the $20/cycle threshold. No special budget allocation needed.

## Decision

**DECISION: PROCEED** with backfill prerequisite.

**Rationale:**
- Q2 (hard blocker): **CLEAR.** DeepSeek context window accommodates all observed qa-review prompts (max 38,931 < 64k effective).
- Q1 (corpus): **< 25 real PR scenarios** (currently 0 real replays, 3 mock tests). Decision framework prescribes: proceed with backfill prerequisite.
- Q3 (harness mode): **CLEAR.** Replay-only, supports offline batch comparison natively. No infra ticket needed.
- Q4 (cost): **CLEAR.** Phase 2 worst-case $7.77, well under $100 threshold.

**Next steps:**
1. **Phase 0.5 ticket:** Backfill eval corpus from `llm_calls` historical data (19,221 calls, 962 full-context reviews). Target 50 diverse scenarios. ~2hr effort.
2. **Phase 1 ticket:** Baseline run — 1 variant (current Sonnet) × 50 scenarios. Gated on Phase 0.5 completion.
