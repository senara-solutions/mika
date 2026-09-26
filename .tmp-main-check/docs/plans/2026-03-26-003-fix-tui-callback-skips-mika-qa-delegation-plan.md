# Fix: TUI callback path skips mika-qa delegation (#269)

## Problem

When a `long_running:run_claude_pilot` callback is delivered to mika-dev in a TUI chat session, the mika-qa review step is never triggered. The TUI callback handler in `chat.rs` calls `format_callback_framing()` (generic framing) instead of `build_callback_trigger_context()` (workflow-aware framing that includes mandatory mika-qa delegation instructions).

## Root Cause

Two callback framing paths exist:
- **Silent mode** (`SilentTrigger::Callback`): Uses `build_callback_trigger_context()` which detects claude-pilot callbacks and injects workflow continuation instructions (delegate to mika-qa, manage work items).
- **TUI chat mode** (`chat.rs` line ~324): Uses `format_callback_framing()` which only wraps the result in XML tags with a generic status line — no workflow-specific instructions.

## Fix

1. **Make `build_callback_trigger_context` public** in `crates/mika-agent/src/agent.rs` (currently private `fn`).
2. **Replace `format_callback_framing` with `build_callback_trigger_context`** in `crates/mika-cli/src/commands/chat.rs` line 324.

This ensures both the silent and TUI callback paths receive identical workflow-aware instructions for claude-pilot callbacks, while maintaining the generic framing for all other callback types.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Make `build_callback_trigger_context` `pub` |
| `crates/mika-cli/src/commands/chat.rs` | Replace `format_callback_framing` → `build_callback_trigger_context` |

## Testing

- Existing tests for `build_callback_trigger_context` already cover claude-pilot and generic callback routing.
- Existing tests for `format_callback_framing` remain valid (it's still called internally by `build_callback_trigger_context`).
- No new tests needed — the fix is a call-site change, and both functions are already well-tested.
