# Plan: Phase 0 — qa-review provider A/B pre-flight investigation

**Ticket:** mika issue#1092
**Type:** scope (investigation, no source changes)
**Date:** 2026-05-12
**Branch:** `scope/1092/qa-review-phase-0-answer-4-pre-flight`

## Goal

Answer 4 pre-flight questions that gate the qa-review provider A/B evaluation. Output is a DECISION comment (PROCEED / BLOCKED / REFRAME) with citations.

## Constraints

- **No source code changes.** Read-only investigation.
- **1-2hr budget.** Each question gets one focused investigation pass.
- **Citation-grounded.** Every answer must cite file paths, line numbers, or sqlite queries — no "should be" prose.

## Decision framework

The investigation's value is the decision framework, not the data gathering. Corpus size (Q1) is already known to be ~0 (mika#1059 merged today). Define the branch criteria upfront:

### Corpus-size thresholds

| Corpus size | Decision | Rationale |
|-------------|----------|-----------|
| >= 50 real PR scenarios | PROCEED to Phase 1 | Sufficient for directional comparison across 3 variants |
| 25-49 real PR scenarios | PROCEED with caveat | Directional signal only; note statistical limitations in Phase 1 ticket |
| < 25 real PR scenarios | PROCEED with backfill prerequisite | File a backfill ticket as Phase 0.5; define the backfill strategy below |

### Backfill strategy (when corpus < 25)

If backfill is needed, the investigation must specify:
1. **Target count:** Minimum 25 scenarios for directional comparison, 50+ for confidence.
2. **Scenario diversity:** Must cover approved PRs, changes-requested, docs-only, large multi-file diffs, edge cases (CI-only changes, dependency bumps).
3. **Source:** Backfill from `llm_calls` table (capture real qa-review invocations with their PR diff inputs and verdicts) via automated extraction script — not manual curation.
4. **Time estimate:** Automated backfill from historical data = 1 ticket (~2hr). Organic accumulation to 50 scenarios at current PR velocity (~5 PRs/day) = ~10 days.

### Hard blockers (any one → BLOCKED)

- Q2: DeepSeek effective context window < representative qa-review prompt total tokens → DeepSeek variant is infeasible, A/B reduces to 2 variants (may REFRAME instead).
- Q3: Eval harness cannot run real-provider comparisons at all → infra prerequisite needed.
- Q4: Per-cycle cost > $100 → budget allocation required before proceeding.

### Statistical significance note

For 3 variants with binary quality ratings (pass/fail), 100+ scenarios per variant would be needed for p<0.05 significance. For Likert-scale quality rating or ranked comparison, fewer suffice. This Phase 0 does not need to resolve the exact threshold — it needs to confirm whether a meaningful corpus exists or can be built.

## Investigation plan

Investigation is ordered by uncertainty and blocking potential: Q2 first (highest uncertainty, hard blocker), then Q1 (known answer, framework application), Q3 (known answer, citation), Q4 (calculation).

### Q2: DeepSeek effective context window (HIGHEST PRIORITY — hard blocker gate)

**What to check:**
- Measure qa-review system prompt token count: `skills/bundled/qa-review/system_prompt.md` is 49KB chars. Estimate ~12-15k tokens.
- Measure representative PR diff size from a recent qa-review run: query `llm_calls` table for mika-qa agent invocations, extract actual input token counts.
  ```sql
  SELECT input_tokens, output_tokens, model, created_at
  FROM llm_calls
  WHERE agent_id = 'mika-qa'
  ORDER BY created_at DESC LIMIT 20;
  ```
- Sum: system prompt + PR diff + repository context + grounding artifacts = total input tokens.
- Cross-check against DeepSeek V3's effective context window. Marketed: 128k. Empirical effective: research community reports of degradation past 32-64k.
- **Truncation behavior check (F5):** Document DeepSeek's API behavior on oversized inputs — does it silently truncate (dangerous: looks like it works but misses context) or return an HTTP error (safe: observable failure)? If the prompt fits within the window, verify full-context comprehension by checking whether DeepSeek responses reference content from the end of the prompt, not just the beginning. A single real-provider test via the eval harness (`MIKA_EVAL_REAL_PROVIDERS=deepseek`) suffices.

**Decision inputs:**
- If representative total < 32k tokens: DeepSeek likely viable.
- If representative total 32-64k tokens: risky, needs calibration run to confirm. Truncation behavior determines whether this is BLOCKED or "proceed with monitoring."
- If representative total > 64k tokens: DeepSeek likely out. DECISION may shift to REFRAME (2-variant comparison only).

### Q1: Eval corpus size (known answer — apply decision framework)

**What to check:**
- Count scenarios in `crates/mika-agent/tests/eval/grounding_regressions/` — currently 20 scenarios (1-18 pre-#1059, 19-20 from #1059). These are mock-response assertion tests, not real PR-review replays.
- Count fixtures in `crates/mika-agent/tests/eval/grounding_regressions/fixtures/`.
- Count golden dataset entries in `crates/mika-agent/tests/eval/golden/`.
- Check if `llm_calls` table in `~/.mika/data/mika.db` has qa-review invocations that could seed additional scenarios — query for actual historical data:
  ```sql
  SELECT COUNT(*) as total_calls,
         MIN(created_at) as earliest,
         MAX(created_at) as latest
  FROM llm_calls
  WHERE agent_id = 'mika-qa';
  ```

**Key distinction:** The ticket asks about "PR scenarios" — real PR review cases, not mock-response unit tests. The current eval harness uses `MockLlmProvider` with pre-programmed responses for CI, and `MIKA_EVAL_REAL_PROVIDERS` for integration tests. For a provider A/B, we need real PR diffs + expected verdicts as the corpus. Corpus is known to be ~0 real PR replays (harness landed today). Apply the decision framework above.

### Q3: Eval harness traffic mode (known answer — cite mika#1059 design)

**Known from mika#1059 design** (cite, don't re-investigate per F3):
- **Mock mode** (default): `MockLlmProvider`, deterministic replay. Runs on CI. Entry: `EvalHarness::builder().responses(vec![...]).build()`.
- **Real provider mode**: `#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS` env var. Integration tests with actual LLM calls. Entry: `MIKA_EVAL_REAL_PROVIDERS=anthropic cargo test -p mika-agent --test eval -- --ignored`.
- **Calibration mode**: `MIKA_EVAL_CALIBRATE=1` captures artifacts to `target/eval-calibration/`.

**Traffic mode:** Test-time only. No live production traffic-split mechanism exists. Phase 2 A/B will be offline batch comparison: run the same captured PR scenarios against each provider via `MIKA_EVAL_REAL_PROVIDERS=<provider>`, compare verdicts. Each provider needs a separate test run. No Phase 2 infra ticket needed for the comparison mechanism itself — the existing harness suffices.

**Cite:** `crates/mika-agent/tests/eval/harness.rs` (EvalHarness struct), `crates/mika-agent/CLAUDE.md` (eval harness documentation).

### Q4: Eval-run cost budget (calculation from actual telemetry)

**What to check:**
- Pull actual token statistics from `llm_calls` telemetry rather than rate-card estimates (per F4):
  ```sql
  SELECT
    AVG(input_tokens) as avg_input,
    AVG(output_tokens) as avg_output,
    COUNT(*) as total_calls,
    -- p50/p95 approximations
    input_tokens, output_tokens
  FROM llm_calls
  WHERE agent_id = 'mika-qa'
  ORDER BY input_tokens;
  ```
- Calculate per-provider cost using actual token distribution:
  - Sonnet: ~$3/M input, $15/M output
  - DeepSeek V3: ~$0.27/M input, $1.10/M output
  - Kimi: ~$0.60/M input, $2.00/M output
- Phase 1 = 1 variant x N scenarios x actual avg tokens. Phase 2 = 3 variants x N scenarios x actual avg tokens.
- Check how eval costs are tracked: `llm_calls` table captures all LLM invocations including eval runs. Operator-visible via dashboard. No separate eval-budget tracking exists — eval runs are indistinguishable from production calls in telemetry.

**Decision inputs:**
- If < $20/cycle: trivial, proceed without special budget allocation.
- If $20-100/cycle: worth noting, operator should pre-approve.
- If > $100/cycle: needs explicit budget allocation or corpus reduction. → BLOCKED.

## Deliverable

Single comment on mika issue#1092 with:
1. Four paragraphs (one per question, ordered Q2→Q1→Q3→Q4), each with file:line or sqlite query citations.
2. DECISION line: PROCEED / BLOCKED / REFRAME, evaluated against the decision framework above.
3. If PROCEED: next-ticket reference (Phase 1 baseline ticket number, or Phase 0.5 backfill ticket if corpus < 25).
4. If BLOCKED: list specific blockers with ticket references for resolution.

## Risks

- **Corpus size is ~0 real PR scenarios.** The eval harness landed today (mika#1059). The 20 existing scenarios are mock-response assertion tests, not real PR replays. Decision framework above pre-defines the backfill path.
- **DeepSeek effective context is empirical, not documented.** May need to rely on community reports. Silent truncation is the dangerous failure mode — must be explicitly checked.
- **Eval harness is test-time only.** No live traffic-split mechanism exists. Phase 2 will be offline batch comparison — the existing harness supports this via `MIKA_EVAL_REAL_PROVIDERS`.
