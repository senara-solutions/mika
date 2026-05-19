---
title: Skill-scoped argv validation at handler dispatch layer
date: 2026-05-18
category: architecture-patterns
module: skills
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - A builtin handler (run_gh, run_gws, git_ops, etc.) has different security scopes per skill
  - Removing a per-skill handler shadow (mika#1168) widens the reachable subcommand surface
  - Need to narrow a tool's reach for a specific skill without re-introducing handler shadowing
  - Adding structural enforcement that complements prompt-level "MUST NOT" rules
tags:
  - skills
  - security
  - run-gh
  - qa-review
  - allowlist
  - handler-dispatch
  - structural-enforcement
  - prompt-parity
related_components:
  - tooling
---

# Skill-scoped argv validation at handler dispatch layer

## Context

When mika#1168 b2 removed qa-review's per-skill `run_gh` exec handler to fix a dispatch-ack shadowing bug, the fix was structurally correct but widened qa-review's reachable subcommand surface to the global `GH_ALLOWED_SUBCOMMANDS` (9 subcommands including `pr merge`, `api`, `issue close`). The only remaining barrier was the qa-review system prompt's "Do NOT merge PRs" instruction — per `feedback_prompt_enforcement_fragile.md`, prompt-level rules don't bind structurally against adversarial prompt injection.

The challenge: re-introduce the narrow scope without re-introducing the handler-shadowing footgun that caused the original dispatch-ack breakage.

## Guidance

**Add a skill-scoped validator at the handler dispatch layer, not the per-skill handler layer.** The pattern is:

1. **Constant** — declare the skill's narrow allowlist as a `const` table (subcommand+verb pairs)
2. **Detection predicate** — check `ToolContext.active_skill_paths` for the skill name
3. **Validator function** — if the skill is active, enforce the narrow allowlist; otherwise early-return Ok
4. **Insertion point** — wire into the builtin handler *after* global input validation, *before* any side effects (subprocess spawn, dedup, audit)

```rust
const QA_REVIEW_SKILL_NAME: &str = "qa-review";
const QA_REVIEW_GH_ALLOWED: &[(&str, &str)] = &[
    ("pr", "review"), ("pr", "diff"), ("pr", "list"), ("issue", "view"),
];

fn validate_qa_review_gh_scope(args: &[String], ctx: &ToolContext<'_>) -> Result<(), ToolOutput> {
    let qa_review_active = ctx.active_skill_paths.iter()
        .any(|info| info.skill_name == QA_REVIEW_SKILL_NAME);
    if !qa_review_active { return Ok(()); }
    // ... check args against QA_REVIEW_GH_ALLOWED ...
}
```

**Key design decisions:**
- **Separate function, not parameter addition to `validate_gh_input`.** Skill-scoping is orthogonal to argv-parsing. The existing `validate_gh_input` has 8+ test callers that exercise pure input parsing — threading `ctx` into them inflates the change without benefit.
- **Before dedup, not after.** A scope violation should produce a scope error, not a `duplicate_pr_review` error masking the scope issue.
- **String literal extracted to constant.** Prevents silent mismatch if the skill is renamed.

## Why This Matters

Per-skill handler shadowing (the pre-mika#1168 shape) breaks the single-dispatch-authority invariant: when two handlers share a tool name, the dispatch order determines which runs, and always-on skills shadow the global handler for all other skills on the same turn. Handler-layer scoping preserves single dispatch authority while adding per-skill narrowing.

The pattern also maintains **prompt-and-runtime parity**: the system prompt documents exactly what the runtime enforces. The previous shape promised wider access than was safe (the prompt said "Permitted: pr, issue, run, workflow..." while the security requirement was "only pr review/diff/list, issue view").

## When to Apply

- **Use this pattern** when a builtin handler needs per-skill scope narrowing and handler shadowing is not acceptable.
- **Don't use this pattern** when the skill needs entirely different handler behavior (different subprocess, different auth) — use per-skill tool registration with distinct tool names instead (see `per-skill-tool-registration-for-dispatch-family-2026-05-17.md`).
- **Known limitation:** `active_skill_paths` is `&[]` in silent/team/investigate modes. The scope guard only fires in conversation mode. This is acceptable when the skill's dangerous tool surface is only reachable in conversation mode (qa-review's verdict-posting flow). If a future skill needs scope narrowing in silent mode, the detection predicate must be anchored to agent identity (e.g., `ctx.home_dir` basename) rather than active skill paths.

## Examples

**Before (mika#1168 b2):** qa-review's per-skill `run_gh.sh` handler shadowed the global builtin, blocking dispatch-ack `issue edit --remove-label ready` on the same turn.

**After (mika#1196):** `validate_qa_review_gh_scope` at the handler dispatch layer narrows qa-review to 4 subcommand+verb pairs while the global handler serves all other skills unchanged.

**Test coverage pattern (two-layer):**
- **Layer A** — 10 inline unit tests in `builtin_handlers.rs` testing the validator function directly with various `active_skill_paths` configurations
- **Layer B** — 2 eval-suite integration tests via `EvalHarness` + `MockLlmProvider` testing the end-to-end wiring (active_skill_paths → ToolContext → run_gh → validator)

## Related

- `per-skill-tool-registration-for-dispatch-family-2026-05-17.md` — sibling pattern for when skills need distinct tool names
- `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the broader principle that structural guards beat prompt rules
- `prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — why prompt-level "MUST NOT" is insufficient
- mika#1196 — the implementing issue
- mika#1168 — the predecessor fix that motivated this pattern
