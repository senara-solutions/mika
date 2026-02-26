---
status: complete
priority: p2
issue_id: 298
tags: [code-review, prompt, quality]
dependencies: []
---

# Fix CLI Reference wording: "Available" implies exhaustive list

## Problem Statement

The CLI Reference section in the system prompt says "Available commands:" but the list is intentionally curated (showing the most common commands). This wording implies exhaustive completeness. If a user asks about an omitted command (e.g., `mika agents clone`, `mika skills test`, `mika setup`), the agent may incorrectly say "that command doesn't exist" because the prompt says "Never invent CLI commands that don't exist. Refer only to the commands listed above."

The combination of "Available commands" + "Refer only to the commands listed above" creates a closed-world assumption that is factually incorrect — there are more commands than listed.

## Findings

- **Code Simplicity Reviewer:** "Available commands:" should be "Common commands:" since the list is deliberately curated, not exhaustive. A single word change fixes the implication.
- **Architecture Strategist:** Cross-referencing with `cli.rs`, the list omits: `mika setup`, `mika agents clone/delete`, `mika teams status/log/create/delete`, `mika skills info/create/test/enable/disable`, `mika memory people/commitments/preferences/events/reset`, `mika reminders cancel`. These are intentionally omitted to save tokens (~200 tokens for the curated list vs ~400+ for exhaustive).
- **Architecture Strategist:** Additionally recommended a drift-detection test that validates the prompt mentions all top-level subcommand names.

## Proposed Solutions

### Option 1: Change wording to "Common commands" + soften restriction (Recommended)
- Change "Available commands:" to "Common commands:"
- Change "Never invent CLI commands that don't exist. Refer only to the commands listed above." to "Never invent CLI commands. These are the most common; other subcommands may exist."
- **Pros:** Accurate wording, prevents false negatives, single line change
- **Cons:** Agent may occasionally hallucinate less-common subcommands
- **Effort:** Small
- **Risk:** Low

### Option 2: Add exhaustive list
- List all subcommands including setup, agents clone/delete, etc.
- **Pros:** Fully accurate, keeps strict "refer only to" instruction
- **Cons:** Doubles token usage (~400+ tokens), most subcommands rarely asked about
- **Effort:** Small
- **Risk:** Low (but wastes tokens)

### Option 3: Add drift-detection test
- Add a test that cross-references the prompt against `Commands` enum names in `cli.rs`
- **Pros:** Catches future drift automatically
- **Cons:** Cross-crate test dependency, moderate effort
- **Effort:** Medium
- **Risk:** Low

## Technical Details

- **File:** `crates/mika-agent/src/prompt.rs` lines 146 and 157
- **CLI definition:** `crates/mika-cli/src/cli.rs` `Commands` enum

## Acceptance Criteria

- [ ] Prompt wording accurately reflects that the list is curated, not exhaustive
- [ ] Agent does not deny the existence of real but unlisted commands
- [ ] Test `test_prompt_includes_cli_section_for_cli_channel` updated if assertion text changes

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Closed-world assumptions in prompts cause false negatives |
