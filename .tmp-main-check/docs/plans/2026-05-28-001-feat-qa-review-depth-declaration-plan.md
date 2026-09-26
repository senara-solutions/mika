# Plan: mika-qa review comments should declare review depth

**Issue:** mika#275
**Type:** feat
**Date:** 2026-05-28

## Problem

When mika-qa posts a PR review comment, there is no indication of whether the review was based on actual code analysis or just PR metadata. A review that only checked file names and CI status looks identical to one that read every line of the diff. This creates false confidence in review thoroughness.

## Analysis

### Current state

The qa-review skill's review process works as follows:

1. **Diff injection is engine-owned:** The engine pre-fetches the PR diff via `resolve_contexts()` in `skills/context.rs` before the LLM turn. The diff is injected as a `{{pr_diff}}` template variable into the skill's system prompt. The context declaration in `skill.toml` is `[context.pr_diff]` with `type = "gh_pr_diff"` and `required = false`.

2. **Three diff availability outcomes:**
   - **Full diff available:** `ContextBlock { content: <diff>, truncated: false }` — the LLM receives the complete diff.
   - **Truncated diff:** `ContextBlock { content: <partial>, truncated: true }` — the diff exceeded the 200K char budget. The injected content includes truncation markers like `--- Diff truncated at ~200K chars ---` with a list of omitted files.
   - **Context resolution failed:** Sentinel text `(Context unavailable: gh_pr_diff resolution failed)` replaces `{{pr_diff}}`. Since `required = false`, the skill still runs but has no diff content.

3. **Current review output format:** The verdict body includes a `DIFF ANALYSIS:` section with files reviewed count and key changes. But there is no declaration of whether the diff was actually available, truncated, or completely missing.

4. **Engine verdict validation:** The `required_tool_arg_suffixes` in `skill.toml` validates that the `--body` arg of `run_gh pr review` contains one of the known `VERDICT:` lines. This validation runs BEFORE subprocess spawn.

### Design approach

The review-depth declaration must be:
- **Accurate:** Reflect the actual diff availability, not a guess.
- **Engine-assisted where possible:** The engine knows the `ContextBlock.truncated` flag and whether resolution failed. Surfacing this metadata to the skill prompt removes reliance on the LLM self-reporting.
- **Enforced:** The declaration must appear in every verdict body, not just when the LLM remembers.

### Three-layer approach

1. **Engine layer:** Surface diff availability metadata alongside the injected content so the LLM has ground truth about what it received.
2. **Prompt layer:** Require the review-depth declaration in the verdict output format.
3. **Guard layer:** Add a structural guard (or extend existing `required_tool_arg_suffixes`) to enforce the declaration's presence before the review posts.

## Implementation

### Step 1: Surface context metadata in skill prompts

**File:** `crates/mika-agent/src/skills/context.rs`

Add a metadata annotation to the injected context content. When `apply_context_replacements()` replaces `{{pr_diff}}`, prepend a machine-readable metadata line that the LLM can reference:

```
<!-- context_meta: type=gh_pr_diff, status=full|truncated|unavailable, chars=N -->
```

This is injected by the engine (not the LLM), so it's ground truth.

**Changes:**
- Add a `ContextStatus` enum: `Full`, `Truncated`, `Unavailable`.
- Derive status from `ContextBlock`: `truncated=true` → `Truncated`; sentinel content → `Unavailable`; else → `Full`.
- In `apply_context_replacements()`, when replacing a `{{key}}` placeholder with a context block, prepend the metadata comment line.

**File:** `crates/mika-agent/src/agent.rs`

No changes needed — `apply_context_replacements()` is already called on skill prompts, so the metadata annotation flows through automatically.

### Step 2: Add review-depth declaration to the skill prompt

**File:** `skills/bundled/qa-review/system_prompt.md`

**2a. Define the depth taxonomy** (add after the Step Budget section):

Add a `### Review Depth Declaration` section that defines the three depth levels and their mapping to context status:

| Depth | Meaning | Context status |
|-------|---------|----------------|
| `code-level` | Full diff was available and reviewed | `status=full` |
| `code-level (partial)` | Diff was truncated; review covers included files only | `status=truncated` |
| `metadata-only` | Diff was unavailable; review is based on file list and PR metadata only | `status=unavailable` |

Instruct the LLM to read the `<!-- context_meta: ... -->` annotation from the injected `{{pr_diff}}` block to determine the depth. If the annotation is missing (pre-upgrade edge case), infer from content: presence of actual diff hunks → `code-level`; sentinel text → `metadata-only`.

**2b. Add the declaration to the verdict output format** (modify Step 5 and the Verdict Output section):

The verdict body must begin with:

```
VERDICT: <class>
DEPTH: <code-level|code-level (partial)|metadata-only>
REASON: <one-line summary>
```

The `DEPTH:` line goes between `VERDICT:` and `REASON:` — this matches the existing pattern of structured metadata at the top of the verdict body.

**2c. Add data integrity rule** for degraded depth:

When `DEPTH: metadata-only`, the maximum verdict is `hold[review]` — the agent must not approve a PR it couldn't read the code for. This reinforces the existing rule: "If any step was skipped due to a tool failure, the maximum verdict is `hold[review]`."

When `DEPTH: code-level (partial)`, the DIFF ANALYSIS must list which files were included vs omitted. The agent may still approve if the reviewed files cover all meaningful changes.

**2d. Update all verdict examples** in the system prompt to include the `DEPTH:` line.

### Step 3: Add engine-side guard for depth declaration

**File:** `skills/bundled/qa-review/skill.toml`

Extend the `[[output.required_tool_arg_suffixes]]` section. The existing guard validates `VERDICT:` lines. We cannot use the same mechanism for `DEPTH:` because `required_tool_arg_suffixes` checks for lines matching any entry in `required_lines` as a suffix — but `DEPTH:` has variable values.

Instead, use the existing `required_suffix_lines` mechanism on skill.toml's `[output]` section — but this also won't work because the depth line is not a suffix, it's in the middle of the body.

**Better approach:** Add a new manifest-level `[output]` field: `required_body_lines`. This is a list of regex patterns that must appear somewhere in the verdict body (not just suffix). The engine validates these against the `--body` argument of `run_gh pr review` before subprocess spawn, just like `required_tool_arg_suffixes`.

**However**, this adds a new engine feature for a single use case. A simpler approach that stays within existing mechanisms:

**Simplest approach — prompt-only enforcement with DIFF ANALYSIS guard:**

The existing Data Integrity Rule already states: "Your verdict output MUST include a `DIFF ANALYSIS` section." The DIFF ANALYSIS section already requires file counts and key changes. We add the `DEPTH:` line to the verdict format and rely on:

1. The prompt discipline (mika-qa has strong prompt compliance — 22 steps of structured process).
2. The existing `DIFF ANALYSIS` section requirement (if the LLM emits DIFF ANALYSIS, it's already parsing context to determine what it reviewed).
3. A lightweight post-condition check: scan the `--body` arg of `run_gh pr review` for `DEPTH:` presence. If missing, reject with a corrective error (same pattern as the existing verdict-trailer guard).

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (or wherever `run_gh` validates `pr_review_body`)

Add a `DEPTH:` line presence check to the existing `pr_review_body` validation path. The check is simple string containment (`body.contains("\nDEPTH: ")`), not regex. If missing, return a structured validation error with a corrective message.

### Step 4: Parse and surface depth in verdict handler

**File:** `crates/mika-agent/src/server/verdict.rs`

Add a `review_depth` field to the `PrReviewEvent` or `Verdict` struct. Parse it from the `DEPTH:` line in the verdict body using a simple regex: `^DEPTH:\s*(.+)$`. This makes review depth available for:
- Structured logging (operator observability).
- Future dashboard display.
- Task metadata (via `try_extract_callback_metadata`).

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

Log the parsed `review_depth` in the existing verdict-handling structured log events. No behavioral changes to verdict routing — depth is informational metadata, not a routing signal.

### Step 5: Tests

**5a. Unit test for context metadata injection:**
- `context.rs`: Test that `apply_context_replacements()` prepends the `<!-- context_meta: ... -->` annotation.
- Test all three statuses: full, truncated, unavailable (sentinel content).

**5b. Unit test for depth line validation:**
- Test that the `run_gh pr review` validator rejects bodies missing `DEPTH:`.
- Test that bodies with valid `DEPTH:` lines pass validation.

**5c. Unit test for verdict parsing:**
- `verdict.rs`: Test that `DEPTH:` line is parsed correctly from verdict bodies.
- Test missing depth (backward compat — should default to `None`, not error).

**5d. Grounding regression test (optional, recommended):**
- Add a scenario to `tests/eval/grounding_regressions/` that verifies the qa-review skill emits a `DEPTH:` line when given a diff context with known status.

## Scope boundaries

### In scope
- Engine-side context metadata annotation (Step 1)
- Skill prompt depth declaration requirement (Step 2)
- Lightweight engine-side `DEPTH:` presence validation on `pr review --body` (Step 3)
- Verdict parser extension for depth field (Step 4)
- Unit tests (Step 5)

### Out of scope
- Dashboard UI display of review depth (future: surface via task metadata or LLM call detail)
- Retroactive depth classification of past reviews
- Per-file depth tracking (file-level granularity beyond the truncation omitted-files list)
- New `[output]` manifest mechanism for body-line validation (rejected: too heavy for this use case)

## Risk assessment

**Low risk.** Changes are additive:
- Context metadata annotation is invisible to the LLM (HTML comment) and backward-compatible.
- Prompt changes add requirements but don't change existing behavior.
- The `DEPTH:` validation in `run_gh` is a pre-spawn check — same pattern as the existing verdict-trailer guard. Failure returns a corrective error, not a hard block.
- Verdict parser extension is additive (new optional field).

**Migration:** No schema changes. No breaking API changes. The `DEPTH:` line in verdict bodies is new — existing review comments won't have it, but the parser handles `None` gracefully.

## Acceptance criteria

- [ ] Every mika-qa review comment includes a `DEPTH:` line in the verdict body (between `VERDICT:` and `REASON:`)
- [ ] The depth value accurately reflects the diff availability: `code-level` when full diff was injected, `code-level (partial)` when truncated, `metadata-only` when unavailable
- [ ] Engine injects `<!-- context_meta: ... -->` annotation into context-replaced skill prompts so the LLM has ground truth about context status
- [ ] `run_gh pr review` body validation rejects verdict bodies missing the `DEPTH:` line with a corrective error
- [ ] `metadata-only` depth caps the maximum verdict at `hold[review]` (prompt-enforced)
- [ ] Verdict parser (`verdict.rs`) extracts the `DEPTH:` value into a structured field for logging
- [ ] Unit tests cover context metadata injection (3 statuses), depth validation (present/absent), and verdict parsing (with/without depth)
- [ ] No test regressions (`cargo test`)
