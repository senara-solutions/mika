---
title: "Deterministic skill context injection — engine-owned pre-fetch for qa-review"
type: brainstorm
status: ready_for_plan
date: 2026-03-26
---

# Deterministic Skill Context Injection

## What We're Building

A declarative `[context]` section in `skill.toml` that lets skills declare typed data requirements. The engine pre-fetches the data before the LLM turn begins and injects it into the system prompt as a template variable. The LLM analyzes data it *received*, not data it *chose to fetch*.

First application: `qa-review` skill declares `[context.pr_diff]` with `type = "gh_pr_diff"`. The engine extracts the PR URL from the task message, fetches the diff via the GitHub API, and injects it as `{{pr_diff}}` in the system prompt. The model never decides whether to read the diff — it's already in context.

## Why This Approach

### The failure that triggered this

On 2026-03-26, mika-qa reviewed PR #281 (fix: conditional required_tools enforcement, +615/-59, 10 files). Turn audit revealed:

- **Zero diff-fetching tool calls** in the entire turn
- The agent fabricated a review of a completely different PR (described "dashboard dev runs source filter" changes from ~PR #278)
- Posted a "pass" QA verdict for code it never read
- Only 2 LLM calls (Sonnet 4.6), 1 tool call (`gh pr comment`)

### Why prompt-level fixes already failed

A prior fix (2026-03-25) added echo-back enforcement, DIFF ANALYSIS requirements, and Data Integrity Rules to the qa-review prompt. The agent fabricated a plausible DIFF ANALYSIS section anyway — within 24 hours. Prompt-level enforcement is advisory; the model can comply or not.

### Why engine-level tool constraints are insufficient

The existing `required_tools = ["run_gh"]` gate has three gaps:
1. **Tool name mismatch**: prompt says `run_shell("gh ...")`, constraint checks `run_gh` — a single `run_gh` call for `gh pr view` satisfies the gate without fetching the diff
2. **No argument checking**: engine verifies the tool was *called*, not *what it was called with*
3. **Single retry**: after one correction attempt, any response is accepted

### Why removing the LLM from the fetch wins

The core insight: **if the LLM doesn't control the fetch, it can't skip the fetch.** By making the engine own data retrieval and inject it as context, the entire class of "agent skipped reading the data" failures becomes structurally impossible. The LLM's job shrinks to analysis and judgment — the parts it's actually good at.

## Key Decisions

### 1. Declarative `[context]` in skill.toml

```toml
# skill.toml
[context.pr_diff]
type = "gh_pr_diff"
required = true
```

- `type` identifies an engine-owned fetch operation (not a shell command)
- `required = true` means the skill does not run if the fetch fails — hard gate, no LLM involved
- The engine matches `type` to a built-in handler; unknown types are errors at skill load time

**Why not shell pre-steps:** shell commands run in the agent trust boundary, errors are untyped, and the project has moved toward Rust builtins (ADR-007). Context fetches are engine-owned operations.

**Why not explicit `source` fields:** a `source = "task_message"` or `source = "$.pr_url"` field would require designing a mini query language for context resolution — YAGNI. The `gh_pr_diff` type handler knows that PR URLs come from the task message by convention. If a different source is ever needed, that's a different context type.

### 2. Context type handler owns the full pipeline

Each context type is a Rust function: `fn(task_message: &str, ...) -> Result<ContextBlock>`.

For `gh_pr_diff`:
1. `extract_pr_url(&task_message)` — pure function, anchored to GitHub PR URL shape, returns `Result<Url, _>`, testable in isolation
2. Fetch diff via GitHub API (engine-owned, not `run_gh` delegation)
3. Apply truncation policy if oversized
4. Return `ContextBlock { name: "pr_diff", content: String, truncated: bool, metadata: ... }`

**Multiple PR URLs edge case**: if the task message contains multiple PR URLs, take the first. Document this behavior. Ambiguity is better handled by failing explicitly in a future version if needed.

### 3. Template variable injection in system_prompt.md

```markdown
## PR Diff (provided by engine — do not re-fetch)
{{pr_diff}}

Analyze the diff above. Your scope is limited to the files shown.
```

Simple `{{var}}` string replacement — no Handlebars, no template engine. Each `[context.*]` key maps to a `{{key}}` placeholder. Unresolved placeholders are errors (caught at skill load time against the prompt text).

The prompt should make clear this is engine-provided data. This prevents the model from trying to "re-verify" by calling fetch tools.

### 4. Smart truncation for large diffs

When the diff exceeds the token limit (configurable, default ~50K tokens):

**Truncation priority order:**
1. Non-generated files first (skip `*.generated.*`, lockfiles, `dist/`, schema dumps)
2. Files with highest churn density (changed lines / total lines) get priority
3. Truncate *within* large files before dropping whole files

**Injected notice:**
```
--- Diff truncated at ~50K tokens ---
Files omitted (23 remaining, excluded by size/generated policy):
- src/generated/schema.rs (+2100)
- src/generated/types.rs (+1800)
Review scope is limited to files shown above.
```

Naive byte-count truncation from the bottom would frequently drop the most interesting files if generated code appears first in the diff.

### 5. Scope: qa-review only, no general gates

This brainstorm is scoped to:
- The `[context]` mechanism in skill.toml (generic, reusable)
- The `gh_pr_diff` context type handler (qa-review specific)
- Template variable injection in system prompts (generic)

**Not in scope:**
- Pre-action gates (e.g., locking `gh pr comment` until diff was fetched) — revisit if a second skill needs step-locking
- Multi-turn chunking for oversized diffs — revisit if truncation produces materially wrong reviews
- Other context types beyond `gh_pr_diff` — add as needed

## What Changes

| Component | Change |
|-----------|--------|
| `skill.toml` manifest | New `[context.*]` section parsed by `Manifest` |
| Skill manifest parser (`manifest.rs`) | Parse `ContextRequirement { type, required }` |
| Agent engine (`agent.rs`) | Pre-LLM context resolution step in `run_team_agent()` / `run_loop()` |
| Context type registry | New module with `gh_pr_diff` handler |
| System prompt loader | Template variable substitution (`{{var}}` replacement) |
| qa-review `skill.toml` | Add `[context.pr_diff]` declaration |
| qa-review `system_prompt.md` | Replace "fetch the diff" instructions with `{{pr_diff}}` injection site |

## Open Questions

*None — all design questions resolved during brainstorm.*

## Resolved Questions

1. **Strategy**: Remove LLM from fetch (not constrain LLM more tightly, not general workflow engine)
2. **Mechanism**: Declarative `[context]` in skill.toml (not shell pre-steps, not hardcoded in delegate_task)
3. **URL resolution**: Context type handler owns extraction (not explicit source fields in skill.toml)
4. **Large diffs**: Smart truncation with priority ordering (not fail-with-hold, not chunk-and-multi-turn)
5. **Multiple PR URLs**: Take first, document behavior
6. **Template engine**: Simple `{{var}}` replacement (no Handlebars)
7. **Scope**: qa-review only for now, general gates deferred
