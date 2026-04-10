---
title: Harden LLM prompt enforcement with mandatory sequence and artifact verification
date: 2026-04-10
tags: [prompt-engineering, skill-review, enforcement, verification]
issue: 514
---

# Problem

The `skill-review` skill prompt described a two-call workflow (inspect → persist) but agents frequently stopped after inspect or persisted malformed content (JSON dicts instead of markdown). The prompt was descriptive but not prescriptive — it explained what the agent *could* do, not what it *must* do.

# Solution

Three complementary hardening techniques, all prompt-level (no code changes):

1. **Mandatory sequence with verification:** Merged the enforcement language directly into the existing numbered workflow steps rather than adding a separate "MANDATORY" block. Step 3 now includes both the persist call AND the verification check (`"written": true`). This eliminates the redundancy that an initial draft introduced (two separate blocks describing the same steps).

2. **Content format requirements in tool schema:** The `content` parameter description in `tools.json` now specifies the expected format ("markdown starting with `##` heading, NOT JSON, NOT a dict"). Tool descriptions are the strongest signal for structured output compliance — stronger than system prompt instructions alone.

3. **Loop prevention:** Explicit cap of 3 `review_skill` calls per skill (inspect + persist + one retry) prevents endless re-inspection loops.

# Key Insight

When hardening LLM prompts for reliable tool use, avoid creating parallel descriptions of the same workflow. An initial approach added a "MANDATORY SEQUENCE" summary block above the detailed numbered steps — this created ~12 lines of duplication that could diverge over time. The better pattern: strengthen the *existing* steps with enforcement language rather than adding a separate enforcement overlay. One authoritative description with mandatory language beats two descriptions where one is "the rules" and the other is "the details."

# Applicability

Apply this pattern whenever a skill prompt describes a multi-step tool workflow that agents shortcut:
- Merge enforcement into existing step descriptions (don't add parallel blocks)
- Put format requirements in `tools.json` parameter descriptions (closest to the tool call)
- Add verification checks as part of the action step, not as a separate post-step
- Cap iteration counts to prevent loops
