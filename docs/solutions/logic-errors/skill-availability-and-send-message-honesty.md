---
title: "Skill Availability Gap and send_message Delivery Honesty"
date: 2026-02-27
category: logic-errors
module: mika-agent (skills, tools, prompt)
severity: high
tags: [skill-matching, always-on, send-message, heartbeat-safety, agent-native, tool-output]
problem_type: semantic-mismatch, skill-availability
pr: "https://github.com/senara-solutions/mika/pull/25"
---

# Skill Availability Gap and send_message Delivery Honesty

## Problem Statement

Two user-reported issues plus one security finding from code review:

1. **Telegram message not received**: User sent a message on Telegram but Mika didn't receive it. Log showed `send_message (CLI mode, no sender configured)` with `ToolOutput::success`, misleading the agent into thinking delivery succeeded when it hadn't.

2. **Tmux tools not available**: Agent said "I need to use the tmux tools directly — but I don't see them listed in my available tools." The tmux skill had `always_on = false`, requiring keyword substring match against the user's message.

3. **Heartbeat security gap** (discovered during review): With tmux made always-on, exec-type shell tools were injected into autonomous heartbeat/silent runs with no user oversight.

## Investigation Steps

1. Read agent log `~/.mika/agents/main/logs/mika.log.2026-02-27` — all activity was CLI-mode, no server/telegram entries. `send_message` logged "CLI mode, no sender configured" at `info` level.

2. Checked `crates/mika-cli/src/init.rs:127-161` — `make_message_sender()` correctly creates `GatewayMessageSender` when `MIKA_ROUTING_URL` + `MIKA_INTERNAL_TOKEN` are set. The code was correct; the env vars were simply not configured.

3. Checked `templates/skills/tmux/skill.toml` — confirmed `always_on = false` with keywords `["tmux", "terminal session", "background process", "long-running"]`.

4. Checked `crates/mika-agent/src/skills/matcher.rs:3-22` — skill matching is per-turn substring matching. Only matched skills' tools are injected. No mechanism for the agent to request tools not matched.

5. Checked `crates/mika-agent/src/agent.rs:677` — heartbeat uses `always_on_skills()` which would include tmux if made always-on.

## Root Cause

**Issue 1**: `send_message` returned `ToolOutput::success("Message logged locally (no outbound sender configured).")` when no sender was configured. The `is_error: false` flag told Claude the tool succeeded, and the text sounded like a normal operational message. Claude had no signal that delivery actually failed.

**Issue 2**: Skill matching in `matcher.rs` does simple substring matching on the lowercased user message. The tmux skill required one of its keywords to appear. Since the agent cannot request tools that aren't in its current tool list, any conversation where tmux was needed but the keywords weren't present resulted in a capability gap.

**Issue 3**: `always_on_skills()` returned ALL always-on skills regardless of handler type. Heartbeat mode (autonomous, no user) would get exec-type tools (tmux, shell-exec) that could execute arbitrary shell commands.

## Solution

### Fix 1: Make tmux skill always-on

**File:** `templates/skills/tmux/skill.toml`

```toml
[skill]
name = "tmux"
always_on = true  # was: false
```

Core capabilities should always be available. The 6 tmux tools add ~600 tokens per API call — acceptable for a fundamental agent capability.

### Fix 2: Warn on undelivered send_message

**File:** `crates/mika-agent/src/tools/send_message.rs`

```rust
// Intentionally returns success, not error. The message was persisted to the
// conversation DB (line above), but external delivery was not attempted because
// no outbound sender is configured. Using ToolOutput::error here would cause
// Claude to retry the tool call in a loop, since the error is permanent and
// not fixable by retrying. The warning text gives Claude enough context to
// inform the user.
None => {
    warn!("send_message called but no outbound sender configured");
    Ok(ToolOutput::success(
        "No outbound sender configured — message was NOT delivered. \
         To enable Telegram delivery, set MIKA_ROUTING_URL and MIKA_INTERNAL_TOKEN.",
    ))
}
```

Key design decision: `ToolOutput::success` (not `error`) because:
- The DB persist succeeded (line 49: `ctx.db.save_message(...)`)
- The error is permanent — retrying won't fix missing env vars
- `ToolOutput::error` would cause Claude to retry in a loop
- The warning text is explicit enough for Claude to inform the user

### Fix 3: Safe always-on skills for heartbeat mode

**File:** `crates/mika-agent/src/skills/mod.rs`

```rust
pub fn safe_always_on_skills(&self) -> Vec<&SkillEntry> {
    use crate::skills::manifest::ToolHandler;

    self.skills
        .iter()
        .filter(|e| {
            e.enabled
                && e.manifest.skill.always_on
                && !e.skill_tools.iter().any(|t| {
                    matches!(t.handler, ToolHandler::Exec { .. } | ToolHandler::Http { .. })
                })
        })
        .collect()
}
```

Heartbeat mode in `agent.rs` now uses `safe_always_on_skills()` instead of `always_on_skills()`. Silent prompt conditionally includes `send_message` guidance only when a sender is configured.

### Fix 4: Shell-exec parity

**File:** `templates/skills/shell-exec/skill.toml` — changed `always_on = false` to `true`. The tmux prompt references shell-exec as the preferred alternative for quick commands, so it must also be available.

## Prevention Strategies

### Skill always-on decision framework

- **Core capabilities** (terminal, shell, memory): always-on
- **Niche/expensive capabilities** (external APIs, file writes): keyword-gated
- **Document the decision** in the skill.toml or code comments
- **Audit all execution contexts** when making always-on: interactive, heartbeat, reminder, silent

### Tool output honesty pattern

- [ ] Never return `success` for a no-op or failed delivery
- [ ] Use `success` with explicit warning text for partial success (DB persisted, delivery failed)
- [ ] Use `error` for hard failures where retrying might help
- [ ] Add code comments explaining the success/error choice at every branch

### Heartbeat safety boundary

- [ ] Autonomous background runs must NOT have exec-type tools
- [ ] Use handler-type filtering (`safe_always_on_skills`) to separate safe from dangerous
- [ ] Guard prompt instructions on capability availability (don't tell agent to use send_message if no sender)
- [ ] Test that heartbeat agent cannot call dangerous tools

### Feature parity checklist

When changing a skill's always-on status:
- [ ] Verify behavior in interactive mode
- [ ] Verify behavior in heartbeat mode
- [ ] Verify behavior in reminder mode
- [ ] Check token budget impact (~200 tokens per tool per API call)
- [ ] Update CLAUDE.md architecture section if significant

## Related Documentation

- `docs/solutions/integration-issues/cli-telegram-messaging-and-skill-seeding.md` — Previous fix for GatewayMessageSender not constructed in CLI path
- `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md` — Agent hallucination from tool capability gaps
- `docs/solutions/architecture-decisions/filesystem-skill-registry-implementation.md` — Skill registry design and matching architecture
- `docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md` — Heartbeat pre-filtering, outbound message delivery
- `docs/solutions/code-review-workflow/self-knowledge-skill-10-findings-resolution.md` — Token budget impact of always-on tools
- `docs/solutions/logic-errors/agent-cli-self-knowledge-and-skill-triggers.md` — Keyword matching false positives, trigger design
- `docs/solutions/security-issues/code-review-7aba1ec-shell-injection-memory-safety.md` — Tmux handler security hardening

## Files Modified

| File | Change |
|------|--------|
| `templates/skills/tmux/skill.toml` | `always_on = true` |
| `templates/skills/shell-exec/skill.toml` | `always_on = true` |
| `crates/mika-agent/src/tools/send_message.rs` | Warning text + code comment |
| `crates/mika-agent/src/skills/mod.rs` | `safe_always_on_skills()` method |
| `crates/mika-agent/src/agent.rs` | Heartbeat uses safe method |
| `crates/mika-agent/src/prompt.rs` | Silent prompt sender guard |
