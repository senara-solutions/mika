---
title: "Deterministic skill context injection via engine-owned pre-fetch"
category: architecture-patterns
date: 2026-03-26
severity: high
tags: [skills, context, qa-review, template-injection, github-api, truncation]
modules: [skills/manifest.rs, skills/context.rs, skills/index.rs, agent.rs]
---

# Deterministic skill context injection via engine-owned pre-fetch

## Problem

On 2026-03-26, mika-qa posted a fabricated review on PR #281 — describing changes from a completely different PR, issuing a false "pass" verdict for code it never read. Turn audit revealed zero diff-fetching tool calls. The agent hallucinated a plausible review from context/memory.

Prompt-level fixes (echo-back enforcement, DIFF ANALYSIS requirements) were already applied on 2026-03-25 and failed within 24 hours. The existing engine-level `required_tools` gate had structural gaps: tool name mismatch (`run_shell` vs `run_gh`), no argument validation, single retry.

**Root cause:** The LLM controlled whether to fetch the data it reviews. If the LLM doesn't fetch, the review is fabricated.

## Root Cause

Prompt enforcement is advisory — the model can comply or not. The `required_tools` gate checks that a tool was *called*, not what it was called *with*. Both defense layers failed simultaneously because they relied on the LLM cooperating.

## Solution

Remove the LLM from the fetch step entirely. The engine pre-fetches data before the LLM turn and injects it into the system prompt as context the LLM cannot skip.

### 1. Declarative `[context]` section in skill.toml (`skills/manifest.rs`)

```toml
[context.pr_diff]
type = "gh_pr_diff"
required = true
```

`ContextRequirement` struct with `context_type: String` and `required: bool` (defaults to `true`). Stored as `HashMap<String, ContextRequirement>` on `SkillManifest` with `#[serde(default)]` for backward compatibility. Follows the exact pattern of `Constraints` and `LlmOverride`.

### 2. Context resolution module (`skills/context.rs`)

New module with:
- `resolve_contexts(matched, message, github_token)` — collects requirements from matched skills, deduplicates (same key + same type = fetch once, same key + different type = warn + skip both), dispatches to handlers by type, returns resolved context map + indices of skills to exclude (when `required = true` and fetch fails).
- `apply_context_replacements(prompt, context)` — single-pass `{{key}}` template replacement using a static `LazyLock<Regex>` matching `\{\{(\w+)\}\}`. Injection-safe: resolved content is never re-scanned.
- `extract_pr_url(message)` — extracts first GitHub PR URL from message text.
- `fetch_pr_diff(owner, repo, number, token)` — fetches diff via GitHub REST API with Bearer auth. 15s timeout.
- `truncate_diff(raw_diff, char_budget)` — smart truncation with churn-density scoring. Non-generated files get 100x priority over lockfiles/generated output. UTF-8-safe via `floor_char_boundary()`.

### 3. Engine integration (`agent.rs`)

Context resolution runs in the pre-LLM pipeline:

```
match_skills() → resolve_contexts() → exclude failed skills → resolve_skill_llm_override() → collect_required_tools() → inject_skills_and_resolve_tools()
```

**Critical ordering:** Context resolution runs *before* LLM override resolution. If a skill with `[context]` AND `[llm]` is excluded due to failed context, its LLM override does not apply.

Wired into `run_agent_inner()` (conversation) and `run_team_agent_inner_impl()` (team). Silent mode skips context resolution — `safe_always_on_skills()` returns builtin-handler-only skills that don't declare `[context]`.

### 4. Validation (`skills/index.rs`)

Three new validation checks in `validate_skill()`:
- Unknown context type → Fail
- `{{key}}` placeholder without matching `[context.key]` → Fail
- `[context.key]` without `{{key}}` in any prompt variant → Warn

## Key Decisions

- **Engine-owned fetch over LLM-controlled fetch:** If the LLM doesn't control the fetch, it can't skip it. Eliminates the entire failure class.
- **Declarative `[context]` over shell pre-steps:** Context fetches are typed engine operations, not shell commands in the agent trust boundary.
- **Smart truncation over fail-on-large:** Truncation with churn-density scoring keeps the most relevant content within budget. Generated files are deprioritized.
- **Single-pass regex replacement:** Static `\{\{(\w+)\}\}` regex ensures injected content (which may contain `{{...}}` text) is never re-expanded.
- **Context before LLM override:** Excluded skills should not influence model selection.
- **`required = false` sentinel:** When optional context fails, a descriptive message replaces the placeholder instead of leaving raw `{{key}}` syntax.

## Prevention

1. **When an agent must read data before acting on it**, move the fetch to the engine layer. Don't rely on prompt instructions to ensure the agent reads.
2. **When adding pre-turn processing to the agent loop**, maintain the ordering: match → context → LLM override → required tools → inject. Check all three paths (conversation, team, silent).
3. **When truncating content for prompt injection**, use `floor_char_boundary()` for UTF-8 safety. Deprioritize generated files. Include a truncation notice listing omitted files.
4. **When adding template variables to prompts**, validate bidirectionally: placeholders must have declarations, declarations must have placeholders.

## Related

- Issue: #265 (mika announces but doesn't execute — led to the qa-review fabrication discovery)
- PR #281: Conditional required_tools enforcement (prerequisite, merged)
- Brainstorm: `docs/brainstorms/2026-03-26-deterministic-skill-context-injection-brainstorm.md`
- Plan: `docs/plans/2026-03-26-002-feat-deterministic-skill-context-injection-plan.md`
- Prior fix: `mika-skills/docs/solutions/prompt-engineering/qa-review-mandatory-diff-read-enforcement.md` (prompt-level fix that failed in 24h)
- Required tools gate: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- UTF-8 truncation: `docs/solutions/runtime-errors/callback-result-too-large-causes-agent-timeout.md`
