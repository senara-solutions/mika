---
title: "Fabricated Action-Claim Guard"
issue: "#308"
date: "2026-04-11"
tags: [agent-core, hallucination, guard, defense-in-depth]
---

# Fabricated Action-Claim Guard

## Problem

The agent fabricated a PR comment URL ("Comment posted: https://github.com/…#issuecomment-4146200192") without executing any tool call. The URL returned HTTP 404 — the comment never existed. The agent had `run_gh` available but chose to hallucinate the result.

## Root Cause

The agent loop had three EndTurn post-conditions but none caught the case where the agent claims an action with a fabricated URL and zero tool calls:
1. Text-based tool call detection — catches XML tool calls as text
2. Required-tools gate — only fires when skills declare `required_tools`
3. Completion-claim guard — catches "merged"/"deployed" without work item updates

External skills (like qa-review) that don't declare `required_tools` had no safety net.

## Solution

Added a 4th post-condition: **fabricated action-claim guard**. Detects when:
1. Response contains a GitHub resource URL (`#issuecomment-`, `#discussion_r`, `#pullrequestreview-`, `/issues/`, `/pull/`)
2. AND contains an action-claim verb (`posted`, `commented`, `created`, `submitted`, `opened`, `reviewed`, `published`, `added`, `wrote`, `replied`, `approved`, `filed`, `raised`, `left a comment/review`)
3. AND zero tool calls were made in the turn

On match: reject response, re-prompt once (same pattern as other guards).

Also strengthened the system prompt grounding rule with explicit URL fabrication BAD/GOOD examples.

## Key Design Decisions

- **Zero-tool-call gate, not per-URL tracking**: Simpler heuristic — if any tool was called, we trust the output. The grounding rule covers residual risk.
- **GitHub URLs only (HTTP and HTTPS)**: Scoped to minimize false positives. Matches both `http://` and `https://` schemes since LLMs may fabricate either. Extendable later.
- **Markdown-safe regex**: Uses `[^\s>\]]` (no `)` exclusion) so URLs inside markdown links `[text](url#fragment)` are matched correctly.
- **Defense-in-depth**: This guard is not exhaustive — it's a backstop. The system prompt grounding rule is the primary defense.

## Files Changed

- `crates/mika-agent/src/agent.rs` — guard logic, regexes, detection function, 12 unit tests
- `crates/mika-agent/src/prompt.rs` — URL fabrication examples in grounding rule

## Pattern

This follows the established post-condition guard pattern in `run_loop()`:
1. `LazyLock<Regex>` for compiled patterns
2. Detection function with fast-path substring check
3. `_retry_done` flag for single-retry cap
4. `EndTurn`-only gate
5. Push assistant response + correction message, then `continue`
