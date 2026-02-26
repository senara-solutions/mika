---
title: "Agent API Self-Knowledge, Built-in Skill Awareness, and Skill Protection"
date: 2026-02-26
severity: medium
module: mika-agent
component: "system prompt assembly, tool output, bundled skills registry"
tags:
  - self-knowledge
  - api-awareness
  - skill-system
  - prompt-engineering
  - channel-gating
symptoms:
  - "Agent unable to guide API integrators on HTTP endpoints and gateway routing"
  - "list_skills output showed all skills identically without origin distinction"
  - "Agent unaware built-in skills cannot be overwritten or that toggle_skill tool exists"
root_cause: "System prompt lacked API section for api channel, bundled skills weren't tagged in tool output, and skill protection mechanism wasn't communicated to agent"
---

# Agent API Self-Knowledge, Built-in Skill Awareness, and Skill Protection

## Problem

The Mika agent had three self-knowledge gaps after the CLI self-knowledge fix:

1. **API ignorance** — When accessed via the `api` channel, the agent had no reference for its own HTTP endpoints (`/message`, `/heartbeat`, `/health`), gateway routing, or Telegram pairing flow. It couldn't guide integrators.

2. **Skill blindness** — `list_skills` output showed all skills identically. The agent couldn't distinguish built-in skills (tmux, shell-exec, web-search, file-reader, calendar) from user-created custom skills.

3. **Protection unawareness** — The agent didn't know built-in skills can't be overwritten via `create_skill`, and didn't know the `toggle_skill` tool existed for enabling/disabling skills.

## Root Cause

- The system prompt assembly (`prompt.rs`) had a `write_cli_section()` for CLI-channel self-knowledge but no equivalent for the API channel.
- The `list_skills` tool had no concept of skill origin — it formatted status, keywords, and tool counts but never checked whether a skill was bundled.
- The Tool Usage prompt section mentioned `create_skill` but omitted `toggle_skill` and `list_skills` for skill awareness.

## Solution

Four targeted changes across three files.

### Change 1: `is_bundled_skill()` function (`bundled_skills.rs`)

```rust
/// Check whether a skill name matches a bundled (built-in) skill.
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS.iter().any(|s| s.name == name)
}
```

**Key design decision:** Uses a function that derives from the authoritative `BUNDLED_SKILLS` static, not a parallel constant. The initial implementation used a `BUNDLED_SKILL_NAMES: &[&str]` constant with a sync test — code review caught the dual-maintenance risk and refactored to a single-source-of-truth predicate. `BundledSkill` remains private; only the predicate is exported.

### Change 2: Origin tags in `list_skills.rs`

```rust
use crate::bundled_skills::is_bundled_skill;

let origin = if is_bundled_skill(&entry.manifest.skill.name) {
    " [built-in]"
} else {
    " [custom]"
};

// Format: "- tmux (enabled) [built-in] — Terminal multiplexer [always-on]"
```

Tags each skill in the output, following the existing `[always-on]` tag pattern.

### Change 3: `write_api_section()` in `prompt.rs`

```rust
fn write_api_section(prompt: &mut String, channel_type: Option<&str>) {
    if channel_type != Some("api") {
        return;
    }
    prompt.push_str(
        "## API Reference\n\
         You are being accessed via the Mika HTTP API. Architecture:\n\
         - **Gateway** receives webhooks (Telegram, WhatsApp) and forwards to per-customer agent containers.\n\
         - **Agent API** (this container): `POST /message` (202 async), `POST /heartbeat` (200/204), `GET /health`.\n\
         - Auth: Bearer token in `Authorization` header (gateway <-> agent shared secret).\n\
         - `POST /message` body: `{\"text\": \"...\", \"chat_id\": N, \"channel\": \"telegram\", \"request_id\": \"...\"}`\n\
         - Responses are delivered asynchronously via the gateway's `POST /send` endpoint.\n\
         - Telegram pairing: user sends `/start` to bot -> gateway creates customer record -> provisions agent container.\n\
         Never invent API endpoints. Refer only to those listed above.\n\n",
    );
}
```

Mirrors the `write_cli_section()` pattern exactly — same function signature, same channel-gating guard, same placement in `build_system_prompt()`. Called after `write_cli_section()`. The two sections are mutually exclusive (a channel can only be one type).

### Change 4: Skill awareness lines in Tool Usage (`prompt.rs`)

```rust
prompt.push_str(
    "- You have built-in skills (use list_skills to see which). Built-in skills cannot be overwritten.\n",
);
prompt.push_str(
    "- You can enable or disable skills with toggle_skill.\n",
);
```

Two lines (~38 tokens total) added to the always-present Tool Usage section. The `list_skills` tool output (Change 2) carries the full `[built-in]`/`[custom]` detail — the prompt just points the agent to it.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/bundled_skills.rs` | Add `pub fn is_bundled_skill()` |
| `crates/mika-agent/src/tools/list_skills.rs` | Tag `[built-in]`/`[custom]` in output |
| `crates/mika-agent/src/prompt.rs` | Add `write_api_section()`, skill awareness + toggle_skill lines |

8 new tests:
- `test_list_skills_tags_builtin`, `test_list_skills_tags_custom` (list_skills.rs)
- `test_prompt_includes_api_section_for_api_channel`, `test_prompt_omits_api_section_for_cli`, `test_prompt_omits_api_section_for_telegram` (prompt.rs)
- Extended `test_prompt_includes_tool_usage_section` to assert `built-in skills`, `list_skills`, `toggle_skill` (prompt.rs)

## Key Design Decisions

1. **Function over constant** — `is_bundled_skill()` derives from `BUNDLED_SKILLS` directly, eliminating the drift risk of a parallel `BUNDLED_SKILL_NAMES` constant. No sync test needed.

2. **Channel-gating pattern consistency** — `write_api_section()` follows the identical pattern as `write_cli_section()` and `write_channel_section()`. All are private, pure functions with early-return guards.

3. **Minimal prompt budget** — Only ~38 tokens added to every prompt (skill lines). The API section (~170 tokens) only appears for API-channel messages. CLI users never pay for API docs and vice versa.

4. **Information, not enforcement** — The `[built-in]`/`[custom]` tag is informational. The actual protection (directory-exists check in `create_skill`) was already in place. The agent just needed to *know* about it.

## Prevention Strategies

1. **New channel → new `write_*_section()`:** When adding a channel type, create a dedicated channel-gated prompt section. Update `VALID_CHANNELS` allowlist. Add inclusion + exclusion tests.

2. **New tool → mention in Tool Usage:** Every agent tool should be mentioned in the system prompt's Tool Usage section. Without it, the agent may not discover the tool from definitions alone.

3. **New bundled skill → automatic:** Adding a skill to `BUNDLED_SKILLS` automatically makes `is_bundled_skill()` recognize it. No parallel list to update.

4. **Drift-detection tests:** The CLI reference already has a drift test (`test_cli_prompt_mentions_top_level_commands`). Consider extending this pattern to tool mentions in the prompt.

5. **Single source of truth:** Never maintain parallel constants or hardcoded lists for data that already exists in an authoritative registry. Use derived functions or predicates instead.

## Related Documentation

- [Agent CLI Self-Knowledge and Skill Triggers](./agent-cli-self-knowledge-and-skill-triggers.md) — Previous fix adding `write_cli_section()`, keyword fixes, and drift tests. This solution extends the same channel-gating pattern to the API channel.
- [Agent Skill Hallucination, TUI Scroll, Telegram Awareness](./agent-skill-hallucination-tui-scroll-telegram-awareness.md) — Channel awareness via `VALID_CHANNELS` allowlist. Foundation for channel-gated prompt sections.
- [Filesystem Skill Registry Implementation](../architecture-decisions/filesystem-skill-registry-implementation.md) — Full skill system architecture. The `is_bundled_skill()` function connects the bundled skills module to the tool layer.
- [CLI-to-Telegram Messaging and Bundled Skill Seeding](../integration-issues/cli-telegram-messaging-and-skill-seeding.md) — Bundled skill seeding with `skill!` macro and seed-but-don't-overwrite pattern.

## Related Todos

- **#300** (complete) — Replace `BUNDLED_SKILL_NAMES` with `is_bundled_skill()` function
- **#301** (complete) — Add `toggle_skill` mention to Tool Usage prompt
- **#289** (pending) — Add agent config tools (follow-up, out of scope)
