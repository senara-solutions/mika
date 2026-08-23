---
title: "Self-identity ground truth as a tri-layer contract — prompt data + prompt rule + on-demand tool"
date: 2026-08-22
category: best-practices
module: crates/mika-agent/src/prompt.rs
problem_type: best_practice
component: agent-core
severity: high
applies_when:
  - The agent may be asked a factual question about itself whose answer is knowable inside the process (which model, which agent, which repo, which container)
  - The agent's training-data prior on "what am I" would produce a confident wrong answer if left unanchored
  - A commented-out or defaulted config value invites the LLM to infer instead of read
tags: [mika-agent, self-identity, anti-fabrication, prompt-anchoring, grounding, tri-layer, tool-verbatim, verb-discipline]
---

# Self-identity ground truth as a tri-layer contract

## Context — the founding incident (mika#1815, Al testeur 2026-07-20)

Al asked Mika « quel LLM utilises-tu ? ». Mika first answered honestly ("je ne sais pas … 11 providers"), Al said "oui, va voir", and Mika returned with a confidently wrong answer: « je tourne sur Anthropic — Claude Sonnet 4 ». Three layers of failure:

1. **Verbe faux** — said "je vais VÉRIFIER" but delivered INFÉRENCE ("très probablement", "vu la date").
2. **Inférence fausse** — Mika actually runs on GLM (z.ai), not Anthropic.
3. **Incohérence interne** — "je ne sais pas avec certitude" then "je tourne sur Anthropic" in the same turn.

The virtue Mika showed on mika#1784 (image-ingestion — "je ne vois pas l'image", no fabricated content) was missing on self-identity. The anti-fabrication rule must apply self-referentially.

## Pattern — three layers, one source of truth

Where the process already knows the answer to "what X am I?", make the agent unable to confabulate by giving it three redundant, coherent channels sourced from the same variable:

1. **Prompt data (always-injected)** — write a `## Runtime` (or `## Something`) section into every system prompt with the fact quoted literally: `` "You are currently running on provider `zai` model `glm-5.2`." ``. Include a paragraph telling the agent this is ground truth and forbidding inference from commented-out config or defaults.

2. **Prompt rule (directive-shaped)** — write a `## Self-Identity Discipline` (or `## X Discipline`) section immediately after the data. Four rules that all key on the data block above:
   - **Quote, don't infer** — for "which X are you?", quote the data section (or call the tool) verbatim.
   - **Verb discipline** — "I will VERIFY" implies reading the source; "I will GUESS / INFER" implies reasoning without a read. Never say VERIFY and deliver INFERENCE.
   - **Fallback honestly** — if ground truth is genuinely unavailable, say "I cannot reliably determine my X" and point at where the config lives. Never fabricate.
   - **Consistency across a single turn** — you may not say "I don't know" then assert with confidence in the next paragraph.

3. **On-demand tool** — register a small, no-arg, read-only builtin tool (`get_active_X`) that returns the same variable verbatim. This is the mechanism for scenarios where the user says "va voir" and the model wants to answer with a genuine tool call rather than just quoting the prompt.

## Sole-source discipline — anti-drift

All three layers must read from the **same in-process variable**. In mika#1815:

- `Settings::active_llm_config()` resolves the runtime provider/model at LLM-instance construction.
- `LlmProvider::provider_name()` + `LlmProvider::model_name()` expose it on every provider impl.
- `ToolContext::provider_name` + `ToolContext::model_name` carry it into every tool call.
- Prompt-assembly reads the same `llm.provider_name()` / `llm.model_name()` at turn-start.

If any layer resolves the fact independently, the tri-layer contract silently breaks the first time an env var or config field is renamed on one path only.

## When to reach for this pattern

Any "what X am I?" question whose answer is knowable inside the process but easy to guess wrong on training-data priors:

- Which model / LLM am I running? (mika#1815, this instance)
- Which agent identity is loaded?
- Which repo/branch am I looking at?
- Which customer / tenant am I serving?
- Which environment am I running in (dev/prod)?

The pattern does not apply when the fact is genuinely outside the process (e.g. "what's the weather?") — for those, tool-mediated retrieval + citation is the correct shape.

## Related

- `docs/solutions/best-practices/opaque-tool-errors-invite-llm-fabrication-2026-07-26.md` — analogue: opaque error output invites confabulation of causes.
- `docs/solutions/best-practices/citation-fabrication-prompt-anchoring-2026-05-02.md` — verbatim-quote discipline for architect review skills; same shape applied at a different layer.
- `docs/solutions/692-self-knowledge-kg-upgrade.md` — historical self-knowledge scaffolding; mika#1815 extends the pattern to runtime LLM identity specifically.
- Contrast case: mika#1784 (image-ingestion honesty) — the virtue extended user-facing but missing self-referentially until this pattern landed.

## Verb-discipline in prompts (secondary pattern)

The "VERIFY vs INFER" verb split from rule 2 above generalizes to any place a prompt asks the agent to know a fact:

- Any tool that returns ground truth for a category of question → describe it in the tool definition as "use this when you want to VERIFY (not INFER)".
- Any prompt directive that means "read the source" → use the word VERIFY.
- Any prompt directive that describes reasoning-from-priors → use the word GUESS or INFER.
- Ban paraphrasing VERIFY into INFER in agent output: "I will check" then "I estimate" is the same fabrication pattern this rule exists to catch.
