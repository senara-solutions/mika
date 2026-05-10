---
module: qa-review
tags: [prompt-engineering, grounding, content-fidelity, per-element-enumeration]
problem_type: misread
category: prompt-engineering
---

# Per-Element Enumeration + Grounding Rules for QA Verdicts

## Problem

QA verdicts produced factual misreads when verifying ACs with multi-element thresholds:

1. **Aggregation failure:** Claimed "all 4 corpora below threshold" when 2/4 were above threshold. The LLM scan-and-summarized instead of enumerating individual values.
2. **Absence hallucination:** Claimed "R5 section missing" when the section existed with 3 confirmed resolutions. The LLM asserted absence without searching for the actual heading.

Root cause: the prompt required AC verification but didn't enforce **per-element enumeration** for multi-element conditions, nor **citation-based grounding** for presence/absence claims.

## Solution

Three complementary prompt rules added to `skills/bundled/qa-review/system_prompt.md`:

### 1. Per-element enumeration (Step 2.5.5)

When an AC contains multi-element thresholds, the verdict MUST enumerate every element by name with observed value and per-element pass/fail. Aggregate summaries like "all N pass/fail" are forbidden.

### 2. Quote-based grounding for absence claims (Step 2.5.5)

Before asserting content is absent, the verdict must state the exact heading searched and either quote found content or list actual headings present. Prevents the scan-and-miss failure mode.

### 3. Quantitative-claim citation (Data Integrity Rules)

Every quantitative claim must have a tool-result citation. Claims without citations must be downgraded to "could not verify" rather than asserted as fact.

## Pattern (reusable)

When an LLM prompt requires verification of structured data:

1. **Force enumeration** — never let the LLM aggregate. Enumerate-and-verify is more tokens but prevents false summaries.
2. **Require citation** — make the LLM prove it saw the data by quoting source. Prevents hallucinated readings.
3. **Provide positive AND negative examples** — show both the correct output shape and the specific failure mode being prevented.

## References

- Issue: senara-solutions/mika-skills#159
- Plan: `docs/plans/2026-05-10-001-fix-qa-review-per-ac-enumeration-per-corpus-plan.md` (on mika-skills branch)
- Triggering incident: mika#926 (misread verdict)
- Follow-up: mika#1059 (eval harness)
