---
date: 2026-04-09
tags: [tools, llm, minimax, tokenization, field-alias, structural-constraints]
issue: 488
related-memory: feedback_prompt_enforcement_fragile
---

# Tool field aliases for LLM tokenization quirks

## Problem

`minimax/minimax-m2.7` consistently emits `"reason"` instead of `"reasoning"` in `update_core_memory` tool input JSON. Prompt-level instructions ("use exact field name 'reasoning'") do not prevent it — the model reads the instruction, acknowledges it, then still emits `"reason":`. This is a tokenization / alignment quirk, not a prompt-comprehension failure.

Observed traces: `091d4ec0a6f34e15bab4f507d9ee556c`, `5d613822-3348-11f1-81f8-c22d175085f1` (mika-dev SQLite, 2026-04-08).

## Root cause

Some LLMs have tokenizer/alignment biases that cause reliable misspellings of specific JSON keys. These are not random errors — the same model reliably truncates the same key the same way. Prompt discipline cannot fix tokenization: the model has already committed to the wrong tokens before it "reads" the instruction.

## Solution

Add a narrow, engine-side alias for the affected field in the specific tool handler. Canonical field always wins when both are present. Do NOT document the alias in the tool schema — we do not want to advertise misspellings or encourage models to rely on them.

```rust
// Accept `reason` as an alias for `reasoning` to accommodate tokenization
// quirks in some LLMs (e.g., minimax/minimax-m2.7 truncates the key).
// Canonical `reasoning` wins when both are present.
let reasoning_canonical = input["reasoning"].as_str();
let reasoning = reasoning_canonical
    .or_else(|| input["reason"].as_str())
    .unwrap_or("");
if reasoning_canonical.is_none() && input.get("reason").is_some() {
    tracing::debug!(
        target: "mika::tools",
        model = ?ctx.model_name,
        provider = ?ctx.provider_name,
        "update_core_memory: accepted 'reason' as alias for 'reasoning'"
    );
}
```

Tool JSON schema `required` array stays `["section", "action", "reasoning"]` — the schema is the canonical contract; the alias is an engine-layer compatibility shim.

## Why not a stricter prompt rule?

Per `feedback_prompt_enforcement_fragile`: *"Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints."* Tokenization artifacts are the purest form of this — the model cannot choose to tokenize differently. Prompt instructions are useless. Engine-level structural accommodations are the only reliable fix.

## Scope discipline

- **Narrow:** One field, one tool. Do NOT preemptively add aliases to other tools.
- **Risk-free:** If a model correctly sends `reasoning`, the alias branch never fires. If it sends `reason`, the call succeeds instead of failing silently at the end of a turn.
- **Hidden:** Not in schema, not in tool description, not in system prompt. Only in code.
- **Observable:** DEBUG log with model + provider lets us track which models hit the alias, so we can file follow-ups if the pattern spreads.

## When to apply this pattern

File a follow-up and add a new alias when:
1. A specific model reliably misspells the same key across multiple traces (not a one-off).
2. Prompt-level instructions have been tried and failed.
3. The misspelling is deterministic (same input → same wrong key).

Do NOT apply this pattern for:
- One-off errors or random field omissions (the model made a mistake, not a tokenization quirk).
- Fields that vary by model — use the canonical name and let the failing model's users switch models.
- Adding generic fuzzy-match parsing — overkill, hides real bugs, and expands the contract surface.

## References

- Issue: [#488](https://github.com/senara-solutions/mika/issues/488)
- Plan: `docs/plans/2026-04-09-001-fix-update-core-memory-reason-alias-plan.md`
- File: `crates/mika-agent/src/tools/update_core_memory.rs`
- Related: `senara-solutions/mika-platform#17` (model calibration umbrella)
