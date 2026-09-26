---
title: "feat: deterministic skill context injection via engine-owned pre-fetch"
type: feat
status: active
date: 2026-03-26
origin: docs/brainstorms/2026-03-26-deterministic-skill-context-injection-brainstorm.md
---

# feat: Deterministic Skill Context Injection

## Overview

Add a declarative `[context]` section to `skill.toml` that lets skills declare typed data requirements. The engine pre-fetches data before the LLM turn and injects it into the system prompt via `{{var}}` template replacement. The LLM analyzes data it *received*, not data it *chose to fetch*.

First application: qa-review skill declares `[context.pr_diff] type = "gh_pr_diff"`. The engine extracts the PR URL from the task message, fetches the diff via GitHub API, truncates smartly, and injects it as `{{pr_diff}}`. The model cannot skip reading the diff because fetching isn't its job.

## Problem Statement / Motivation

On 2026-03-26, mika-qa posted a fabricated review on PR #281 — describing changes from a different PR, issuing a false "pass" verdict for code it never read. Turn audit showed zero diff-fetching tool calls.

Prompt-level fixes (echo-back enforcement, DIFF ANALYSIS requirements) were already applied on 2026-03-25 and failed within 24 hours. The existing engine-level `required_tools` gate has structural gaps: tool name mismatch (`run_shell` vs `run_gh`), no argument validation, single retry. Prompt enforcement is advisory; the model can comply or not.

**Root cause:** The LLM controls whether to fetch the data it reviews. Removing that control eliminates the entire failure class.

(See brainstorm: `docs/brainstorms/2026-03-26-deterministic-skill-context-injection-brainstorm.md`)

## Proposed Solution

### Architecture

```
skill.toml declares:     [context.pr_diff]
                          type = "gh_pr_diff"
                          required = true

system_prompt.md uses:    {{pr_diff}}

Engine flow:              match_skills()
                            ↓
                          resolve_contexts()     ← NEW: pre-fetch, exclude failed-required skills
                            ↓
                          resolve_skill_llm_override()
                            ↓
                          collect_required_tools()
                            ↓
                          inject_skills_and_resolve_tools()  ← template {{var}} replacement here
                            ↓
                          run_loop()
```

**Critical ordering decision:** Context resolution runs *before* LLM override resolution. If a skill with `[context]` AND `[llm]` is excluded due to failed context, its LLM override does not apply. This prevents the wrong model being selected when the skill that requested it isn't participating.

### Components

| Component | File | Change |
|-----------|------|--------|
| `ContextRequirement` struct | `skills/manifest.rs` | New type, follow `Constraints` pattern |
| `context` field on `SkillManifest` | `skills/manifest.rs` | `HashMap<String, ContextRequirement>` with `#[serde(default)]` |
| Context resolution module | `skills/context.rs` (new) | `ContextBlock`, `resolve_contexts()`, `gh_pr_diff` handler |
| Template replacement | `agent.rs` | Single-pass `{{var}}` replacement in `inject_skills_and_resolve_tools()` |
| Engine integration | `agent.rs` | Wire into conversation + team paths (skip silent) |
| Validation | `skills/index.rs` | Context type + placeholder cross-checks in `validate_skill()` |
| qa-review skill | `mika-skills/qa-review/` | Add `[context.pr_diff]`, rewrite prompt, remove `required_tools` |

## Technical Approach

### Phase 1: Types and Manifest (`skills/manifest.rs`)

Add after the `Constraints` struct (line ~82):

```rust
// skills/manifest.rs

/// A single context requirement declared in [context.*] sections.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextRequirement {
    /// Engine-owned context type (e.g., "gh_pr_diff"). Matched to a handler at runtime.
    #[serde(rename = "type")]
    pub context_type: String,
    /// If true, the skill is excluded from the turn when this context cannot be resolved.
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool { true }
```

Add to `SkillManifest`:

```rust
pub struct SkillManifest {
    pub skill: SkillInfo,
    pub triggers: Triggers,
    pub llm: LlmOverride,
    pub constraints: Constraints,
    #[serde(default)]
    pub context: HashMap<String, ContextRequirement>,  // NEW
}
```

TOML parsing: `[context.pr_diff]` with `type = "gh_pr_diff"` naturally deserializes into `HashMap<"pr_diff", ContextRequirement { context_type: "gh_pr_diff", required: true }>`.

**Tests** (follow patterns at lines 574-621):
- Parse with `[context.pr_diff]` present
- Parse without `[context]` (backward compat — existing skills must not break)
- Coexistence with `[constraints]`, `[llm]`, `[triggers]`
- Multiple context keys in one skill
- `required` defaults to `true` when omitted

**Test helper updates:** Add `context: HashMap::new()` to `make_entry()` in `matcher.rs` (line ~87) and `make_skill_entry()` in `mod.rs` (line ~188).

### Phase 2: Context Resolution Module (`skills/context.rs` — new file)

```rust
// skills/context.rs

use std::collections::HashMap;
use anyhow::Result;

/// Known context type identifiers.
pub const KNOWN_CONTEXT_TYPES: &[&str] = &["gh_pr_diff"];

/// The default character budget for context injection (~50K tokens at 4 chars/token).
pub const DEFAULT_CONTEXT_CHAR_BUDGET: usize = 200_000;

/// Resolved context block ready for template injection.
pub struct ContextBlock {
    pub name: String,
    pub content: String,
    pub truncated: bool,
}

/// Resolve all context requirements for matched skills.
/// Returns: (resolved context map, indices of skills to exclude).
pub async fn resolve_contexts(
    matched: &[&SkillEntry],
    message: &str,
    github_token: Option<&str>,
) -> (HashMap<String, ContextBlock>, Vec<usize>) {
    // 1. Collect all unique (key, requirement) pairs across matched skills
    // 2. Dedup: same key + same type = fetch once. Same key + different type = warn, skip both
    // 3. For each unique requirement, dispatch to handler by context_type
    // 4. On failure: if required, mark declaring skill indices for exclusion
    // 5. Return resolved map + exclusion indices
}
```

**`gh_pr_diff` handler:**

```rust
/// Extract the first GitHub PR URL from a message.
/// Anchored to: https://github.com/{owner}/{repo}/pull/{number}
pub fn extract_pr_url(message: &str) -> Option<(String, String, u64)> {
    // Regex: https://github\.com/([^/]+)/([^/]+)/pull/(\d+)
    // Returns (owner, repo, number). Takes first match.
}

/// Fetch PR diff from GitHub API.
pub async fn fetch_pr_diff(
    owner: &str,
    repo: &str,
    number: u64,
    github_token: Option<&str>,
) -> Result<String> {
    // GET https://api.github.com/repos/{owner}/{repo}/pulls/{number}
    // Accept: application/vnd.github.v3.diff
    // Authorization: Bearer {token} (if available)
    // User-Agent: mika-agent
    // Timeout: 15s
    // Error handling: 404, 403, 401 → meaningful error messages
}
```

Reuse `reqwest` (already a dependency) and the `HTTP_CLIENT` lazy static from `builtin_handlers.rs`, or create a dedicated client with a 15s timeout (consistent with `check_work_item.rs` pattern).

**Smart truncation:**

```rust
/// Truncate a unified diff to fit within a character budget.
/// Priority: non-generated files first, highest churn density first.
/// Truncates within large files before dropping entire files.
pub fn truncate_diff(raw_diff: &str, char_budget: usize) -> ContextBlock {
    // 1. Parse into file-level hunks: Vec<DiffFile { path, content, added, removed, is_generated }>
    // 2. Classify generated files by pattern (see GENERATED_PATTERNS)
    // 3. Score: non-generated get base priority; within non-generated, sort by churn density
    //    churn_density = (added + removed) as f64 / content.len() as f64
    // 4. Include files in priority order until budget is reached
    // 5. If last file doesn't fit entirely, truncate it at a hunk boundary
    // 6. Append truncation notice listing omitted files with their +/- counts
    // 7. Use floor_char_boundary() for UTF-8 safety
}

const GENERATED_PATTERNS: &[&str] = &[
    "Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml",
    "Gemfile.lock", "poetry.lock", "composer.lock",
    ".generated.", ".gen.", "_generated.",
    ".pb.go", ".pb.rs",
    "schema.rb", "structure.sql",
];

fn is_generated_file(path: &str) -> bool {
    // Check if path matches any generated pattern
    // Also check directory prefixes: dist/, build/, node_modules/, target/
}
```

**UTF-8 safety:** Use `str::floor_char_boundary(n)` (stable since Rust 1.82, edition 2024). Follow the pattern from `format_callback_framing()` in agent.rs. Log with `warn!` when truncation activates.

**Tests:**
- `extract_pr_url`: GitHub PR URL shapes, non-PR URLs, multiple URLs (takes first), no URL
- `fetch_pr_diff`: mock reqwest responses (success, 404, 403, empty diff)
- `truncate_diff`: under budget (no truncation), over budget with generated files deprioritized, UTF-8 boundary handling, empty diff
- `is_generated_file`: lockfiles, generated markers, directory prefixes

### Phase 3: Template Variable Replacement (`agent.rs`)

Modify `inject_skills_and_resolve_tools()` to accept resolved context:

```rust
fn inject_skills_and_resolve_tools(
    matched: &[&SkillEntry],
    tools: &ToolRegistry,
    system: &mut String,
    provider_name: &str,
    model_name: &str,
    resolved_context: &HashMap<String, ContextBlock>,  // NEW
) -> Vec<ToolDefinition> {
    for entry in matched {
        let mut prompt = entry.resolve_prompt(provider_name, model_name);
        if !prompt.is_empty() {
            // Single-pass template replacement — no recursion
            for (key, block) in resolved_context {
                let placeholder = format!("{{{{{}}}}}", key); // {{key}}
                prompt = prompt.replace(&placeholder, &block.content);
            }
            write!(system, "\n<context type=\"skill\" trust=\"local\">\n## {} Skill\n{}\n</context>\n",
                entry.manifest.skill.name, prompt).unwrap();
        }
        // ... tool collection unchanged
    }
}
```

**Single-pass guarantee:** `String::replace()` operates on the original string, not the result of prior replacements. Since we iterate context entries and call `replace` for each key sequentially, and `replace` returns a new string with all occurrences of that key substituted, there is no recursion. A diff containing `{{pr_diff}}` as text would only be affected if `pr_diff` is also a resolved context key — but since the diff *is* `pr_diff`, the replacement already happened (the diff replaced `{{pr_diff}}`, and the diff's own `{{pr_diff}}` text is just text at that point).

Wait — actually there's a subtle issue. If we do:
1. `prompt = prompt.replace("{{pr_diff}}", &diff_content)` — this replaces `{{pr_diff}}` with the diff
2. If `diff_content` itself contains `{{some_other_key}}` and we then do `prompt = prompt.replace("{{some_other_key}}", &other_content)` — that WOULD replace inside the diff

**Fix:** Do all replacements on the original prompt, collecting changes. Or: resolve in a single pass using a regex that matches all `{{key}}` patterns and replaces based on the map.

```rust
// Safe single-pass replacement using regex
fn apply_context_replacements(prompt: &str, context: &HashMap<String, ContextBlock>) -> String {
    if context.is_empty() {
        return prompt.to_string();
    }
    // Build regex matching any declared placeholder: {{key1}}|{{key2}}|...
    let pattern = context.keys()
        .map(|k| format!(r"\{{\{{{}\}}\}}", regex::escape(k)))
        .collect::<Vec<_>>()
        .join("|");
    let re = regex::Regex::new(&pattern).unwrap();
    re.replace_all(prompt, |caps: &regex::Captures| {
        let matched = caps.get(0).unwrap().as_str();
        let key = &matched[2..matched.len()-2]; // strip {{ and }}
        context.get(key).map(|b| b.content.as_str()).unwrap_or(matched)
    }).into_owned()
}
```

This is a true single-pass: the regex scans the original string once, replaces matched placeholders with resolved content, and the replaced content is never re-scanned. Injection-safe.

**`max_prompt_size` interaction:** `max_prompt_size` (default 16KB, ceiling 64KB) applies to the raw `prompt_snippet` loaded at scan time (before replacement). Context truncation has its own budget (`DEFAULT_CONTEXT_CHAR_BUDGET = 200,000`). These are independent limits — the raw snippet is small, the post-replacement result is large.

### Phase 4: Engine Integration (`agent.rs`)

Wire context resolution into the two interactive agent paths. **Skip silent mode entirely** — `safe_always_on_skills()` returns builtin-handler-only always-on skills, which are not expected to declare `[context]`. Any `{{var}}` placeholders in their prompts would remain as literal text, which is harmless.

**In `run_agent_inner()` (conversation mode):**

```rust
// Current flow (simplified):
let matched = params.skills.match_message(user_message);           // line ~1011
let llm_override = resolve_skill_llm_override(&matched, ...);     // line ~1014
inject_skills_and_resolve_tools(&matched, ..., &system, ...);     // line ~1022

// New flow:
let mut matched = params.skills.match_message(user_message);       // line ~1011
let (resolved_ctx, exclude) = resolve_contexts(                    // NEW
    &matched, user_message, params.github_token
).await;
// Remove excluded skills (iterate in reverse to preserve indices)
for &idx in exclude.iter().rev() {
    matched.remove(idx);
}
let llm_override = resolve_skill_llm_override(&matched, ...);     // AFTER exclusion
inject_skills_and_resolve_tools(&matched, ..., &system, ..., &resolved_ctx); // pass context
```

**In `run_team_agent_inner_impl()` (team mode):**

Same pattern. The `task_message` (delegation message) is used as the extraction source instead of `user_message`. This is correct — when mika-dev delegates "review PR #281" to mika-qa, the delegation message contains the PR URL.

```rust
let mut matched = params.skills.match_message(task_message);
let (resolved_ctx, exclude) = resolve_contexts(
    &matched, task_message, params.github_token
).await;
for &idx in exclude.iter().rev() {
    matched.remove(idx);
}
// ... rest unchanged, pass resolved_ctx to inject_skills_and_resolve_tools
```

**In `run_silent_inner()` (silent mode):**

Skip context resolution. Pass an empty `HashMap` to `inject_skills_and_resolve_tools()`.

**Logging:**
- `tracing::info!("context resolved: key={key}, type={type}, bytes={len}, truncated={truncated}")` on success
- `tracing::warn!("context resolution failed: key={key}, type={type}, error={err}, skill={skill_name} excluded")` on required failure
- `tracing::debug!("context resolution skipped: silent mode")` in silent path

**Timeout:** Context resolution gets a 15s timeout per requirement (consistent with `check_work_item.rs`). Use `tokio::time::timeout()`. If the timeout fires, treat as a failed resolution.

### Phase 5: Validation (`skills/index.rs`)

Add to `validate_skill()` after the existing `[constraints]` validation (line ~561):

```rust
// Validate [context] section
for (key, req) in &manifest.context {
    // Check context type is known
    if !context::KNOWN_CONTEXT_TYPES.contains(&req.context_type.as_str()) {
        diags.push(SkillDiagnostic::fail(format!(
            "[context.{}] declares unknown type '{}'. Known types: {:?}",
            key, req.context_type, context::KNOWN_CONTEXT_TYPES
        )));
    }
}

// Check for {{key}} placeholders in prompt that don't have [context.*] declarations
let placeholder_re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
let all_prompts = std::iter::once(prompt_snippet.as_str())
    .chain(model_prompts.values().map(|s| s.as_str()));
let mut all_placeholders: HashSet<&str> = HashSet::new();
for p in all_prompts {
    for cap in placeholder_re.captures_iter(p) {
        all_placeholders.insert(cap.get(1).unwrap().as_str());
    }
}
// Placeholders without context declarations → Fail
for ph in &all_placeholders {
    if !manifest.context.contains_key(*ph) {
        diags.push(SkillDiagnostic::fail(format!(
            "Prompt uses {{{{{}}}}} but no [context.{}] section declares it. \
             Add [context.{}] to skill.toml or remove the placeholder.",
            ph, ph, ph
        )));
    }
}
// Context declarations without placeholders → Warn
for key in manifest.context.keys() {
    if !all_placeholders.contains(key.as_str()) {
        diags.push(SkillDiagnostic::warn(format!(
            "[context.{}] declared but no {{{{{}}}}} placeholder found in any prompt variant. \
             The context will be fetched but never used.",
            key, key
        )));
    }
}
```

### Phase 6: qa-review Skill Update (`mika-skills/qa-review/`)

**`skill.toml`:**

```toml
[skill]
name = "qa-review"
description = "Quality gate for PR review"
version = "0.2.0"
always_on = true

[llm]
provider = "anthropic"
model = "claude-sonnet-4-6"

[context.pr_diff]
type = "gh_pr_diff"
required = true

# required_tools removed — diff is now engine-provided, no need to
# gate on run_gh. The agent still has access to run_gh for posting
# comments but is not required to call it for the diff.
```

**`system_prompt.md`:** Rewrite to reference injected diff instead of fetch instructions.

Key changes:
- Remove Step 3's `run_shell("gh pr diff ...")` instruction entirely
- Replace with: `## PR Diff (provided by engine — do not re-fetch)\n<context type="pr_diff" trust="untrusted">\n{{pr_diff}}\n</context>`
- Keep Steps 1, 2, 4, 5 (pr view, pipeline compliance, compound doc check, auto-merge)
- Update DIFF ANALYSIS section to reference "the diff above" instead of "fetch the diff"
- Add notice: "The diff above was fetched by the engine. Do not attempt to re-fetch it."
- Update Step 1 to still use `run_gh` for `gh pr view` (metadata, CI status) — this is NOT the diff fetch

## System-Wide Impact

### Interaction Graph

1. `match_skills()` runs, qa-review matches (always_on + keyword)
2. `resolve_contexts()` runs: extracts PR URL from message → calls GitHub API → truncates → returns `ContextBlock`
3. If resolution fails: qa-review excluded from matched set → its `[llm]` override does not apply → its prompt not injected → its tools still registered (via other skills or builtins)
4. If resolution succeeds: `inject_skills_and_resolve_tools()` replaces `{{pr_diff}}` in qa-review's prompt → full diff in system prompt → LLM analyzes it
5. `run_loop()` runs with the diff in context → LLM uses `run_gh` for `gh pr comment` (posting the review) but never needs to fetch the diff itself

### Error Propagation

- `extract_pr_url()` returns `None` → `resolve_contexts()` marks skill for exclusion → `tracing::warn!` → skill silently excluded
- GitHub API returns error (404/403/401/500) → same exclusion path
- GitHub API timeout (15s) → `tokio::time::timeout` fires → same exclusion path
- `github_token` is `None` → fetch attempted unauthenticated (public repos work, private repos return 404) → exclusion if fails
- Truncation errors → should not happen (operates on valid UTF-8 from API) → if they do, return the raw diff without truncation

### State Lifecycle Risks

No persistent state changes. Context resolution is stateless — fetch, process, inject, discard. No DB writes. No side effects beyond the GitHub API GET request (read-only).

### API Surface Parity

Two interfaces expose skill matching:
1. `match_skills()` in `matcher.rs` — returns full matched set. Context resolution filters this post-match.
2. `safe_always_on_skills()` in `mod.rs` — returns restricted set. Context resolution is skipped.

Both paths call `inject_skills_and_resolve_tools()`, which now takes a `resolved_context` parameter. The safe path passes an empty map.

## Acceptance Criteria

### Functional Requirements

- [ ] `skill.toml` with `[context.pr_diff]` parses correctly into `SkillManifest`
- [ ] Existing skills without `[context]` continue to parse (backward compatibility)
- [ ] `extract_pr_url()` extracts owner/repo/number from GitHub PR URLs in task messages
- [ ] Engine fetches PR diff via GitHub API before the LLM turn
- [ ] Fetched diff is injected as `{{pr_diff}}` in the skill's system prompt
- [ ] If diff exceeds 200K chars, smart truncation applies (non-generated first, churn density priority)
- [ ] If `required = true` and fetch fails, the declaring skill is excluded from the turn
- [ ] Template replacement is single-pass (no recursive expansion — injection-safe)
- [ ] `validate_skill()` catches: unknown context types (Fail), orphaned `{{var}}` (Fail), unused `[context.*]` (Warn)
- [ ] qa-review skill updated: `[context.pr_diff]` declared, prompt uses `{{pr_diff}}`, `required_tools` removed

### Non-Functional Requirements

- [ ] Context resolution timeout: 15s per requirement
- [ ] UTF-8-safe truncation (use `floor_char_boundary()`)
- [ ] Logging: info on success, warn on failure/exclusion, debug on skip
- [ ] No DB writes — context resolution is stateless

### Quality Gates

- [ ] Unit tests for manifest parsing, URL extraction, truncation, template replacement
- [ ] Integration test: mock GitHub API, verify diff injection in system prompt
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean
- [ ] `mika skills validate --name qa-review` passes

## Dependencies & Risks

| Risk | Mitigation |
|------|------------|
| `regex` crate dependency | Already in mika-agent's dep tree (used by other modules). No new dependency. |
| GitHub API rate limits (5000/hr authenticated) | qa-review runs on delegation, not every turn. Well within limits. |
| Large diff blows up system prompt | Smart truncation with 200K char budget. Prompt size was already ~30K before context; 230K total is within Claude's 1M context. |
| Backward compat: old skills without `[context]` | `#[serde(default)]` + empty HashMap means zero behavior change for existing skills |

## Future Considerations

- **New context types:** `gh_pr_metadata` (CI status, labels, reviewers), `gh_issue_body` (for issue-driven planning). Each type is a Rust function in `context.rs`.
- **Pre-action gates:** If a second skill needs step-locking (e.g., "don't call X until Y was called"), revisit the gate mechanism from the brainstorm. Not needed today.
- **Context caching:** Currently re-fetches every turn. For long conversations about the same PR, session-scoped caching could reduce API calls. Deferred — qa-review is single-turn via delegation.

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-26-deterministic-skill-context-injection-brainstorm.md](docs/brainstorms/2026-03-26-deterministic-skill-context-injection-brainstorm.md). Key decisions carried forward: (1) engine-owned pre-fetch over LLM-controlled fetch, (2) declarative `[context]` in skill.toml over shell pre-steps, (3) smart truncation over fail-with-hold.

### Internal References

- Manifest parsing: `crates/mika-agent/src/skills/manifest.rs` — `Constraints` struct at line ~69 is the template
- System prompt: `crates/mika-agent/src/prompt.rs:206` — `build_system_prompt()`
- Skill injection: `crates/mika-agent/src/agent.rs:2353` — `inject_skills_and_resolve_tools()`
- Required tools: `crates/mika-agent/src/agent.rs:2339` — `collect_required_tools()`
- GitHub API pattern: `crates/mika-agent/src/tools/check_work_item.rs:81` — `fetch_github_pr_status()`
- URL parser: `crates/mika-agent/src/tools/check_work_item.rs:34` — `parse_github_ref()`
- UTF-8 truncation: `crates/mika-agent/src/agent.rs` — `format_callback_framing()` pattern
- Skill validation: `crates/mika-agent/src/skills/index.rs:380` — `validate_skill()`

### Institutional Learnings Applied

- Required tools enforcement gate (`docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`) — name-based tracking is insufficient; argument validation not checked
- Callback result truncation (`docs/solutions/runtime-errors/callback-result-too-large-causes-agent-timeout.md`) — truncate at injection time, not API layer; UTF-8-safe boundary walking
- UTF-8 byte-slicing panic (`docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`) — use `floor_char_boundary()`, never `&s[..N]`
- qa-review diff enforcement (`mika-skills/docs/solutions/prompt-engineering/qa-review-mandatory-diff-read-enforcement.md`) — echo-back pattern works for format but not for data fetching
- Grounding rule six prompt paths (`docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md`) — checked: only conversation + team paths need context; silent paths skip it
- Per-skill LLM override (`docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md`) — `#[serde(default)]` pattern, conflict resolution
