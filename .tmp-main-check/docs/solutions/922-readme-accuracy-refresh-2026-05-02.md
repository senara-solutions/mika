---
module: docs
tags: [readme, accuracy, documentation-drift]
problem_type: documentation-drift
category: best-practices
---

# README Accuracy Refresh

## Problem

README.md had drifted significantly from reality:
- Test count was ~1169 (actual: ~3500)
- LLM described as single-provider "Claude (Sonnet 4.6 default) via direct API" (actual: 11 providers via `LlmProvider` trait)
- Project structure omitted `mika-a2a`, `dashboard/`, `packages/ui/`, `skills/bundled/`
- Knowledge Graph subsystem (a major feature surface) was not mentioned

## Solution

Updated all factual claims with verifiable sources:
- Test count sourced from `cargo test` output (3512 passed)
- Provider list sourced from `ProviderKind` enum in `crates/mika-common/src/llm/mod.rs`
- Directory listing sourced from `ls crates/` and `ls .`

## Lessons

1. **Cite sources for counts.** README numbers like test counts rot fast. Always cite the source command in PR body so reviewers can verify.
2. **Audit on milestone boundaries.** README drift accumulates silently — adding a periodic README audit to milestone checklists would catch this earlier.
3. **Architecture diagrams should use generic labels.** "Claude API" → "LLM API" is more stable against provider changes.
