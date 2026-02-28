# Fix: File-Reader Always Available + Agent Image Chaining Guidance

**Date:** 2026-02-28
**Branch:** feat/multimodal-tool-results
**Status:** Planning

## Problem

After landing the multimodal tool results infrastructure (PR #31), testing reveals Mika still can't view screenshots. Two root causes:

### Problem 1: `read_file` tool unavailable during screenshot workflows

`templates/skills/file-reader/skill.toml` has `always_on = false`. When user says "take a screenshot of workspace 1", the keyword matcher activates the hyprland skill but NOT file-reader. Since `read_file` is only available when file-reader matches, Claude literally cannot call it — it's not in the tool definitions.

### Problem 2: Agent doesn't know to chain `read_file` after image-producing tools

Even if `read_file` were available, the system prompt doesn't tell the agent that when a tool produces an image file path (e.g., `~/Pictures/screenshots/ws1.png`), it should use `read_file` to view the image contents. The agent sees the path as plain text and doesn't know the multimodal pipeline will kick in.

## Solution

Two targeted changes, no architectural modifications needed.

### Phase 1: Make file-reader always-on

**File:** `templates/skills/file-reader/skill.toml`

Change `always_on = false` to `always_on = true`.

`read_file` is a fundamental capability (like a filesystem "read" syscall). It should always be available, not gated behind keyword matching. This is analogous to how shell-exec or other core tools are always available.

**Impact:** `read_file` tool will be included in every turn's tool definitions. Minimal token overhead (one tool definition). No behavioral change for existing workflows — just makes the tool available when it wasn't before.

### Phase 2: Add image chaining guidance to system prompt

**File:** `crates/mika-agent/src/prompt.rs`

Add guidance near the existing image line telling the agent to use `read_file` when another tool produces an image file path:

```
- When a tool produces an image file path (e.g., screenshot saved to /path/to/image.png), use `read_file` on that path to view the image contents.
```

This goes right after the existing line:
```
- Tools may return images (screenshots, image files); you will see and can describe their contents.
```

**Impact:** Teaches the agent the two-step pattern: (1) tool produces image path → (2) `read_file` to view it. Works for any tool that saves images, not just screenshots.

## Acceptance Criteria

- [ ] `templates/skills/file-reader/skill.toml` has `always_on = true`
- [ ] System prompt includes guidance to chain `read_file` on image file paths
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes
- [ ] End-to-end: screenshot skill saves image → agent calls `read_file` → multimodal envelope → Claude sees image

## Files Changed

1. `templates/skills/file-reader/skill.toml` — `always_on = false` → `always_on = true`
2. `crates/mika-agent/src/prompt.rs` — Add image chaining guidance line
