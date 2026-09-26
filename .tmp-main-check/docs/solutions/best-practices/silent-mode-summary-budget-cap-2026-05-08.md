---
title: "Silent-mode summary budget cap prevents context-channel leakage"
date: 2026-05-08
category: best-practices
module: mika-agent
problem_type: best_practice
component: assistant
severity: medium
applies_when:
  - "Agent receives callback/webhook/heartbeat triggers with a short messages array"
  - "Conversational summary dominates the system prompt relative to the actual trigger message"
  - "Agent references 'prior turns' that don't exist in the current silent-mode session"
tags:
  - prompt-assembly
  - silent-mode
  - context-channel-leakage
  - identity-toml
  - summary-injection
  - mika-1009
  - axis-3
---

# Silent-mode summary budget cap prevents context-channel leakage

## Context

mika#1009 identified a 300:1 system-prompt-to-messages token ratio in silent-mode turns: the messages array contains a single trigger event (a few hundred tokens) while the system prompt — including the conversational summary from compaction — runs thousands of tokens. At this ratio the LLM misinterprets summary content as prior conversation turns and produces degenerate responses referencing "prior turns" the model never participated in.

Axis 4 (mika#1019) addressed this for summary-dominant agents (mika-arch) with `[context.summary].inject = false` — a global load-prevention gate. But agents that benefit from summary continuity on interactive turns but get burned by it on silent turns need a **mode-conditional** gate, not a global opt-out. Axis 3 fills this gap.

## Guidance

Add `max_tokens` to the `[context.summary]` section in `identity.toml`. The field is mode-agnostic at the schema level — the silent-mode gating lives in code (`load_gated_summary()` in `agent.rs`), not in the field name.

```toml
# Cap summary to ~1000 tokens on silent-mode turns, keep full on interactive turns
[context.summary]
inject = true
max_tokens = 1000
```

Or for stricter control — omit summary entirely on silent turns:

```toml
[context.summary]
inject = true
max_tokens = 0    # load-omit sentinel: summary skipped on silent-mode turns
```

### Implementation details

- **`load_gated_summary()`** (`agent.rs`) consolidates the Axis 4 + Axis 3 gate sequence. Both summary injection sites (conversation-mode and silent-mode) call this single helper.
- **Invariant:** Axis 4 (`inject = false`) MUST short-circuit before Axis 3 evaluation — the `inject` check is the first operation, preventing any DB call when load-prevention is active.
- **`truncate_to_token_budget()`** (`prompt.rs`) performs the actual truncation at a word boundary using `str::floor_char_boundary()` for UTF-8 safety. Uses `CHARS_PER_TOKEN_ESTIMATE = 4` (heuristic, conservative for English).
- **`Some(0)` sentinel:** Treated as a structural omit signal (summary not injected), NOT as a "zero-token cap." Same code path as Axis 4's `inject = false` short-circuit but conditional on silent mode.
- **Non-silent turns are never affected** regardless of `max_tokens` value.

### Gate sequence

```
Axis 4: inject = false?  →  Yes: skip (no DB call)  →  Ok(None)
                          →  No: load summary from DB
Axis 3: silent mode + max_tokens set?
  →  Some(0): skip (load-omit sentinel)  →  Ok(None)
  →  Some(n): truncate to ~n tokens       →  Ok(Some(truncated))
  →  None or non-silent: full summary     →  Ok(Some(full))
```

## Why This Matters

Silent-mode turns (callback, webhook, heartbeat, reminder, deferred dispatch) have no human in the loop and no streaming UI. The short messages array creates a token ratio that causes the LLM to hallucinate conversation context from the summary. Axis 3 provides per-agent, mode-conditional control so operators can balance summary continuity on interactive turns against leakage risk on silent turns.

This is defense-in-depth alongside Axis 4 (global opt-out) and the eventual Axis 2 (summarizer content reform). Each axis operates at a different layer: Axis 4 controls *whether* to load; Axis 3 controls *how much* to inject when loaded; Axis 2 will control *what* goes into the summary in the first place.

## When to Apply

- When an agent exhibits "prior turns" hallucination on callback or webhook turns but needs full summary on interactive turns
- When provisioning a new agent that handles both interactive and silent triggers
- When debugging context-channel leakage symptoms (degenerate responses, false completion claims on silent turns)

## Examples

**Before (Axis 4 only — binary choice):**

```toml
# Option A: Keep summary everywhere (leaks on silent turns)
[context.summary]
inject = true

# Option B: Remove summary everywhere (loses continuity on interactive turns)
[context.summary]
inject = false
```

**After (Axis 3 — mode-conditional):**

```toml
# Summary on interactive turns, capped on silent turns
[context.summary]
inject = true
max_tokens = 500
```

## Related

- mika#1009 — Parent finding doc identifying 4-axis fix plan
- mika#1019 — Axis 4 sibling (`[context.summary].inject` opt-out)
- `docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md` — Parent finding
- `docs/solutions/best-practices/per-agent-context-injection-opt-out-2026-05-07.md` — Axis 4 compound doc
- `crates/mika-agent/CLAUDE.md` § Context Injection Configuration — field documentation
- `docs/configuration.md` § identity.toml — operator-facing documentation
