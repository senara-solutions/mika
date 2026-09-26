---
module: agent
tags: [intent-guard, callback, silent-mode, post-condition, endturn]
problem_type: logic-error
category: prompt-enforcement
ticket: mika#870
related: [mika#871, mika#696, mika#702, mika#846]
---

# Callback Turn Silent Exit — Terminal Action Guard (#870)

## Problem

When a `long_running:run_claude_pilot` callback subtask fires and the LLM
diagnoses failure (running diagnostic tool calls like `check_task`,
`gh pr list`) but returns EndTurn with empty text, the agent loop exits
silently.  No `send_message` call, no `update_task_status` call, zero rows
in `messages` for the session.  The operator is blind to dev-run failures.

### Root cause (three layers)

1. **Silent-mode loop exit without persistence.** When the LLM returns
   empty text and `mode.follow_up_on_empty()` returns `false` (Silent mode),
   the loop exits with `text: None`.  The `messages` write is gated on
   `text.is_some()`, so nothing is persisted.

2. **No callback-specific post-condition guard.** The INTENT_GUARDS registry
   is evaluated inside the `if !text.is_empty()` block — it never fires when
   text is empty.  No guard covers the empty-text exit path.

3. **Prompt framing is a hint, not a contract.**
   `build_callback_trigger_context` says "use send_message" but the LLM
   (kimi-k2.5 in the observed incident) rationalizes around soft instructions.

## Solution

### Engine guard (load-bearing)

New `IntentPrecondition` entry `callback_terminal_action` in the
`INTENT_GUARDS` const array.  AND-shape: requires BOTH `update_task_status`
AND `send_message` (attempts, not just successes).  `create_task` is optional.

**Trigger:** `msg.starts_with("[callback:")` — matches the synthetic user
message format emitted by `run_silent_agent` for `SilentTrigger::Callback`.

**Two guard sites** because the registry path and the empty-text path are
separate branches in `run_loop`:

| Code path | When it fires | Guard mechanism |
|-----------|---------------|-----------------|
| Non-empty text EndTurn | `if !text.is_empty()` → INTENT_GUARDS evaluation | Registry entry |
| Empty text EndTurn (Silent mode) | `if !mode.follow_up_on_empty()` | Inline guard before early exit |

Both share `CALLBACK_TERMINAL_ACTION_LABEL` and
`CALLBACK_TERMINAL_ACTION_CORRECTION` consts to prevent string drift.

### Prompt nudge (defense-in-depth)

Paragraph appended to `build_callback_trigger_context` mirroring the guard's
AND-shape contract.  Not load-bearing per
`feedback_prompt_enforcement_fragile.md`.

### Key design decision: trigger predicate

The plan specified `msg.starts_with("A background task has")` — the
`format_callback_framing` output.  That text is in the **system prompt**,
not the user message.  In silent callback mode, `user_input_text` is
`[callback: {label}]`.  Implementation uses `[callback:` prefix matching
against the actual user message format.

## Verification

```bash
cargo test -p mika-agent --test eval callback_terminal_action
```

Four integration tests:
1. **Happy path** — both tools called, clean exit
2. **Recovery** — guard fires once, agent complies on retry
3. **Persistent failure** — guard fires once then dormant, max-steps cap halts
4. **Non-callback skip** — guard does not fire on regular messages

Note: tests run in Conversation mode (not Silent mode) because the
EvalHarness uses `run_agent()`.  The inline empty-text guard for Silent mode
is structurally identical but only reachable in production
(`run_silent_agent`).  The shared predicate functions
(`callback_trigger_active`, `callback_terminal_action_satisfied`) are
exercised by the Conversation-mode tests.

## Related

- **mika#871** — Parent task leak (sibling fix).
- **mika#696 / #702** — Original webhook zero-tools guard and registry.
- **mika#846** — Ready-label dispatch guard (same registry pattern).
- **mika#862, #863, #864** — Open EndTurn-guard family (shared helper
  extraction deferred until second guard ships per YAGNI).
