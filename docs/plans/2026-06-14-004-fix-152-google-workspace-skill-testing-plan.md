# Fix: Google Workspace Skill — Testing Fixes & Improvements

**Issue:** mika issue#152
**Type:** fix (skill activation + reliability)
**Branch:** `feat/152/skills-google-workspace-skill-testing`

## Problem Statement

PR #148 shipped the Google Workspace skill (`run_gws` tool), but acceptance testing revealed that natural language prompts fail to activate the skill. The three reported symptoms — keyword activation gaps, inconsistent `run_gws` loading, and legacy tool fallback even with explicit mentions — all share a single root cause: the keyword list in `skill.toml` is too narrow.

## Root Cause Analysis

The `run_gws` tool is **skill-scoped** — it only enters the LLM's tool array when the google-workspace skill matches the current turn's message via `match_skills()` in `matcher.rs`. The matcher does case-insensitive substring matching (`message_lower.contains(kw)`) against the skill's keyword list.

Current keywords: `["google", "gmail", "google calendar", "google drive", "gdrive"]`

Natural prompts like "show my latest 5 emails", "what meetings do I have today", or "triage my inbox" contain none of these keywords, so:
1. The skill doesn't match → `run_gws` is absent from the tool array
2. If the LLM tries `run_gws` anyway (from prior context), dispatch returns "Unknown tool: run_gws"
3. The agent falls back to whatever tools are available (shell commands, legacy tools)

This is not a registration bug — the builtin handler, `KNOWN_BUILTINS` entry, and `tools.json` are all correct. The issue is purely in the skill manifest's trigger configuration.

## Solution

**Make the google-workspace skill `always_on = true`.**

Rationale:
- Expanding the keyword list is a whack-a-mole game — users will always find new natural phrasings ("check my schedule", "find the doc", "send a note to", "what's on my plate today")
- The `run_gws` tool is lightweight (single tool definition, 76-line system prompt) — always-on cost is minimal
- Google Workspace is a core integration for an executive assistant, not an edge-case skill
- The tool itself validates service subcommands (`gmail`, `calendar`, `drive`) and blocks dangerous flags — the safety model doesn't depend on keyword gating
- This matches the pattern of other core skills like `shell-exec`, `git-ops`, and `github` which are always-on

**Additionally expand the keyword list** as a secondary signal for `required_tools` enforcement and skill-aware prompt context. Even with `always_on = true`, keyword matches promote the skill from `AlwaysOn` to `Keyword` match reason, which activates `[constraints].required_tools` enforcement if declared (#463).

## Implementation Steps

### Step 1: Update `skill.toml` manifest

**File:** `crates/mika-agent/templates/skills/google-workspace/skill.toml`

```toml
[skill]
name = "google-workspace"
description = "Interact with Google Workspace (Gmail, Calendar, Drive) using the gws CLI"
version = "0.1.1"
always_on = true
timeout_secs = 45

[triggers]
keywords = [
    "google", "gmail", "google calendar", "google drive", "gdrive",
    "email", "emails", "inbox", "send email",
    "calendar", "meeting", "meetings", "schedule", "agenda", "free", "busy",
    "drive", "document", "documents", "spreadsheet", "slides",
    "triage",
    "gws", "workspace"
]
```

Changes:
- `always_on = false` → `always_on = true`
- Version bump `0.1.0` → `0.1.1`
- Keywords expanded with natural language triggers across all three services (Gmail, Calendar, Drive)
- Added `"gws"` and `"workspace"` (were in the issue's original keyword list but missing from the manifest)

### Step 2: Update existing tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Review and update any tests that assert on the google-workspace keyword list or `always_on` state. The `test_run_gws_in_known_builtins` test should still pass (it checks `KNOWN_BUILTINS`, not the manifest).

### Step 3: Add regression test for skill activation on natural language

**File:** `crates/mika-agent/src/skills/matcher.rs` (or a new test in the existing test module)

Add a test that verifies the google-workspace skill matches on natural language prompts:
- "show my latest 5 emails" → skill matched (via `always_on`)
- "what meetings do I have today" → skill matched (keyword "meeting" + `always_on`)
- "triage my inbox" → skill matched (keyword "triage" + `always_on`)
- "search drive for quarterly report" → skill matched (keyword "drive" + `always_on`)

The test should verify the skill is present in the matched set regardless of keyword presence (due to `always_on`), and that keyword-bearing messages produce `MatchReason::Keyword` (not just `AlwaysOn`).

### Step 4: Verify no regression on `run_gh` behavior

The `run_gh` handler is in a separate skill (`github`) with its own `tools.json`. The google-workspace changes are isolated to the `google-workspace` skill manifest — no shared code paths are modified. Verify via existing test suite (`cargo test -p mika-agent`).

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/templates/skills/google-workspace/skill.toml` | `always_on = true`, version bump, keyword expansion |
| `crates/mika-agent/src/skills/matcher.rs` | Add regression test for natural language activation |

## Acceptance Criteria Mapping

| Criterion | How addressed |
|-----------|---------------|
| `run_gws` builtin handler loads reliably in every session | `always_on = true` ensures the skill (and its `run_gws` tool) is always in the tool array |
| Skill activates on natural language prompts | `always_on = true` + expanded keywords cover all natural phrasings |
| All 4 test scenarios pass cleanly when re-run | Root cause (keyword mismatch) is eliminated; manual re-test recommended |
| No regression on existing `run_gh` behavior | Isolated change; verified via existing test suite |

## Risk Assessment

**Low risk.** This is a manifest-only change (no Rust engine code modified). The `always_on` flag is a well-tested mechanism used by 7+ other bundled skills. The expanded keyword list uses the same substring matching already in production. No schema changes, no new dependencies, no API changes.

## Test Plan

1. `cargo test -p mika-agent` — full crate test suite (includes builtin handler tests, matcher tests)
2. `cargo clippy` — lint check
3. Manual verification: start a session and confirm "show my latest 5 emails" activates the google-workspace skill and `run_gws` is in the tool array
