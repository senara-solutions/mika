---
title: Per-skill tool registration for claude-pilot dispatch family
date: 2026-05-17
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - Adding a new skill in the claude-pilot dispatch family (shape: handler sources _shared/dispatch-lib.sh, routes by SKILL field)
  - Considering "share one tool with a union-enum discriminator" vs "each skill owns its own tool"
  - Debugging why mika-dev's LLM picks the wrong dispatch class or fails to invoke the planned pipeline (`/ce:plan`, `/mika-groom-ticket`)
  - Reviewing any PR that proposes consolidating sibling skill tools onto a single host's union enum
tags:
  - skills
  - claude-pilot
  - dispatch
  - tool-registration
  - dev-pilot
  - dev-groom
  - structural-fix
  - detection-vs-prevention
related_components:
  - tooling
---

# Per-skill tool registration for claude-pilot dispatch family

## Context

Two coexisting designs for the claude-pilot dispatch skill family (currently dev-pilot + dev-groom, both wrapping headless Claude Code via claude-pilot) were tried:

1. **Union-enum on host (2026-05-02 → 2026-05-17, mika#932 / PR #934).** dev-pilot owned the only `run_claude_pilot` tool with a union enum on the `skill` parameter (`["dev-pilot", "dev-groom"]`). dev-groom was prompt-only — no `tools.json`, no handler. The shared `_shared/dispatch-lib.sh` case switch routed by the `$SKILL` value in the input JSON.

2. **Per-skill ownership (current, mika#1173, 2026-05-17 onward).** Each dispatch skill owns its own tool: dev-pilot owns `run_claude_pilot` (enum: `["dev-pilot"]`, entry `/mika`), dev-groom owns `run_claude_pilot_groom` (enum: `["dev-groom"]`, entry `/mika-groom-ticket`). Both handlers source the shared lib; the case switch still routes by SKILL, but each handler is reachable through its own tool name first.

Design 1 looked cheaper (one tool, one schema, one handler) but failed structurally: it created an implicit cross-boundary contract between mika-dev's LLM (which sees one tool, has to pick the right enum value) and the dispatch lib (which routes by side-channel SKILL field). The contract was not machine-enforced and regressed five times in fifteen days (#1032, #1081, #1097, #1109, #1134) — each fix a post-hoc detection of "session drifted into executor mode," not prevention.

mika#1173 reverted to design 2.

## Guidance

**Each skill in the claude-pilot dispatch family owns its own tool name.**

The rule:

- One tool per dispatch skill (`run_claude_pilot` for dev-pilot, `run_claude_pilot_groom` for dev-groom, `run_claude_pilot_<NAME>` for any future sibling).
- Tool's `skill` enum is a singleton (`["dev-pilot"]`, `["dev-groom"]`) — kept as a required field so the engine's `derive_dispatch_class(skill)` (executor.rs) still routes by class, but no LLM-side decision exists about which value to put there.
- Each skill ships its own `tools.json` + `handlers/run.sh` (the handler is a 6-line thin wrapper sourcing `_shared/dispatch-lib.sh`).
- The shared lib's case switch on `$SKILL` stays — it's the dispatch-class-to-entry-command mapping, and both old-shape and new-shape calls converge on it from above.

What stays shared:

- Worktree setup, slug derivation, env scrubbing, GitHub App auth, EXIT trap, post-flight detectors, callback delivery — all in `_shared/dispatch-lib.sh`. Per-skill ownership is about the **tool surface** (what mika-dev's LLM sees and dispatches against), not about plumbing.

### Adding a new dispatch sibling

1. Create `skills/bundled/<skill-name>/tools.json` registering its own tool name. Singleton `skill` enum. Required: `["skill", "prompt", "task_id"]`. Long-running.
2. Create `skills/bundled/<skill-name>/handlers/run.sh` (mode 755): thin wrapper sourcing `_shared/dispatch-lib.sh` and calling `dispatch_claude_pilot`.
3. Add a new arm in the shared lib's case switch mapping the SKILL value to its slash-command entry point.
4. Add the skill to the relevant well-known agent allowlist (`well_known_agents.rs` `MIKA_*_IDENTITY`).
5. Update `self-dev/system_prompt.md` and `self-dev-webhook-ready-label/system_prompt.md` (if applicable) to teach mika-dev when to dispatch the new tool.
6. **Grep the codebase for all tool-name string references** before shipping: engine intent-guard predicates (`agent.rs`), correction messages, prompt examples, test fixtures, doc strings. Tool-surface changes are inherently cross-cutting.

## Why This Matters

### Structural prevention vs detection

Design 1 produced six discoverable regression incidents (one original + five fix attempts) in ~2 weeks. Each fix was detection-focused: post-flight log greps for `/ce:plan` invocation, plan-file-size checks, early-exit guards. None addressed the underlying cause: the LLM's selection task at the dispatch boundary (which `skill` enum value to emit on which tool) was implicit and model-behavior-dependent. Every model shift (e.g., the 2026-05-07 mika-dev kimi-k2.5 → claude-sonnet-4-6 swap) invalidated the calibration.

Design 2 makes the wrong-value class structurally impossible at the boundary: mika-dev sees two discrete tools with distinct names. The selection is encoded in the tool name itself, which the LLM cannot conflate without the engine rejecting the call (no tool `run_claude_pilot_typo` exists).

This is a documented general principle. See `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` ("Threshold: 3rd attempt to fix the same class via prompt = stop and file an engine ticket"). mika#1173 hit N=5 attempts before the structural revert; the principle is now backed by concrete cost evidence.

### Tool surface as machine-enforceable contract

The fundamental property of design 2: the contract mika-dev follows is the tool name, which the engine's tool-registry enforces. There is no side-channel routing decision that the LLM has to "remember" to make correctly. Compare design 1's contract: "Pick the right `skill` value (`dev-pilot` for implementation, `dev-groom` for grooming) when calling `run_claude_pilot`." That contract lives entirely in prose; the engine accepts both enum values on the same tool, defers the routing decision to a shell-side case switch, and offers no upstream rejection if the LLM picks wrong.

See `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` for the canonical bug class. Design 1 is a textbook Shape A instance ("prompt instructs, framework refuses → LLM improvises a fallback → correctness is a dice roll"); design 2 is the canonical remedy ("co-design the contract").

## When to Apply

- **Any new skill in the claude-pilot dispatch family.** Skills that take operator-supplied issue references, create worktrees, and launch headless Claude Code sessions are in this family. Don't share the existing tools' enum.
- **When refactoring an existing dispatch sibling that uses union-enum routing.** Split it onto its own tool name; the new shape is a near-zero-cost migration (~30 lines of duplicated tool-schema JSON, one new 6-line handler).
- **When debugging "session drifted into executor mode" or "no `/ce:plan` invocation in session log" symptoms.** These are detection-class symptoms produced when the LLM emits a structurally-valid-but-semantically-wrong dispatch. Structural prevention (per-skill tool) eliminates the failure class.

## Examples

### Anti-pattern (design 1, reverted)

```jsonc
// dev-pilot/tools.json — single host, union enum
{
  "name": "run_claude_pilot",
  "input_schema": {
    "properties": {
      "skill": { "enum": ["dev-pilot", "dev-groom"] }
    }
  }
}

// dev-groom/ — prompt-only, no tools.json, no handler
```

```bash
# _shared/dispatch-lib.sh case switch — routes by side-channel
case "$SKILL" in
  dev-pilot)  ENTRY_COMMAND="/mika" ;;
  dev-groom)  ENTRY_COMMAND="/mika-groom-ticket" ;;
esac
```

mika-dev's LLM sees ONE tool, has to "remember" which enum value matches the user's intent. Wrong-enum calls succeed at the schema layer, fail downstream silently (HEAD unchanged, no plan file).

### Pattern (design 2, current)

```jsonc
// dev-pilot/tools.json — its own tool, narrowed enum
{
  "name": "run_claude_pilot",
  "input_schema": {
    "properties": {
      "skill": { "enum": ["dev-pilot"] }  // singleton
    }
  }
}

// dev-groom/tools.json — its own tool, distinct name
{
  "name": "run_claude_pilot_groom",
  "input_schema": {
    "properties": {
      "skill": { "enum": ["dev-groom"] }  // singleton
    }
  }
}
```

```bash
# dev-groom/handlers/run.sh — thin wrapper, same as dev-pilot's
#!/bin/bash
set -e
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot
```

mika-dev's LLM sees TWO discrete tools. The selection (implement vs groom) is encoded structurally; the engine rejects misspelled or hallucinated tool names at the schema layer.

### Cross-cutting verification step (lesson from mika#1173)

When changing a tool surface, grep the codebase for ALL string references before shipping:

```bash
git grep -nE 'run_claude_pilot[^_]' -- crates/ skills/ | grep -v 'run_claude_pilot_groom'
git grep -n '"skill":\s*"dev-' -- skills/
git grep -n 'skill == "dev-' -- crates/
git grep -n 's\.name == "run_claude_pilot"' -- crates/
```

mika#1173's plan §3 enumerated 8 implementation steps; /ce:review caught 3 cross-cutting BLOCKERS the plan missed (4 intent-guard predicates in `agent.rs` hardcoded the old tool name; `self-dev-webhook-ready-label/system_prompt.md` still taught the old shape; a self-dev JSON example incorrectly omitted the required `skill` field after literal interpretation of plan §3.8.1). All three were tool-name-coupling sites that the architect's plan didn't enumerate. Tool-surface changes are inherently cross-cutting; a grep audit step belongs in the test plan for every tool-rename PR.

## Related

- `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md` — original best-practice doc; describes design 1 (union-enum on host). Superseded by this doc for the tool-registration shape, but its guidance on the shared library (worktree setup, slug derivation, env scrubbing) remains correct.
- `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` — the canonical bug class. Design 1 was a Shape A instance; design 2 is the remedy.
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — N=3 threshold for promoting prompt-rules to engine-guards. mika#1173 reinforces with N=5 concrete evidence.
- `docs/solutions/best-practices/dispatch-lib-plan-on-branch-entry-command-override-2026-05-11.md` — sibling structural pattern. Plan-on-branch moved the entry-command decision out of the inner session up into dispatch-lib. Per-skill tool registration moves the dispatch-class decision out of an enum-value-on-a-shared-tool into distinct tool names. Same principle: don't depend on cross-boundary inference; encode the decision structurally.
- mika#1173 — the PR that performed the revert; carries the full plan + Phase 0 DB-grounded root-cause verification + cross-cutting blocker fixes.
- mika#932, PR #934 — original consolidation onto union-enum (design 1).
- mika#1032, #1081, #1097, #1109, #1134 — the five detection-style fix attempts that established the N=5 cost evidence.

## Companion: slash-command propagation into worktrees

mika#1173 also added a structural fix to a related load-bearing invariant: `_shared/dispatch-lib.sh` now `cp -r`s `.claude/commands/` from the platform root into each worktree's `.claude/` directory at worktree setup. Without this, the inner Claude Code session cannot resolve slash commands like `/mika-groom-ticket` or `/mika` and falls back to text-mode improvisation.

This is the same family of fix: don't depend on cross-boundary inference. The inner session needs its slash commands resolved structurally (file present in cwd's `.claude/commands/`), not by hoping the LLM improvises correctly when the file is absent. The snapshot semantics (worktree-creation time) are documented in the `_set_up_worktree` comment block; staleness during long-running sessions is bounded by worktree TTL and the slug-immutability principle (mika#844).
