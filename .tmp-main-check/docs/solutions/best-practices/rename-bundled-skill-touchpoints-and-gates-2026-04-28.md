---
title: "Renaming a bundled skill: touchpoints, classification rubric, and deploy gates"
date: 2026-04-28
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Renaming a bundled skill in skills/bundled/
  - Decoupling skill identity from app/binary identity
  - Adding a required parameter to an existing skill tool
tags:
  - skill-rename
  - bundled-skills
  - skill-registry
  - dev-pilot
  - classification-rubric
---

# Renaming a bundled skill: touchpoints, classification rubric, and deploy gates

## Context

Bundled skills in `skills/bundled/` have identity spread across many surfaces: the directory name, `skill.toml` `name=` field, Rust skill-ID string literals in disabled_skills lists, dependency arrays, test fixtures, KG seed entries, sibling-skill prompts, live docs, and GitHub issue bodies. Renaming one requires a systematic audit to avoid missing a reference — and more importantly, to avoid renaming references that look similar but have a different meaning (e.g., the binary name vs. the skill name).

This pattern was established during mika#844 (renaming `claude-pilot` skill to `dev-pilot`), where the skill name overlapped with the app binary name (`claude-pilot`), the log channel marker (`[claude-pilot]`), and a metadata namespace (`tasks.metadata.claude_pilot.*`).

## Guidance

### 1. Classification rubric (D2 pattern)

Before renaming anything, classify every reference into three categories:

- **App-meaning (KEEP):** references to the binary, log channel, log paths, metadata namespace, upstream repo, tool name
- **Skill-meaning (RENAME):** references to the dispatch role, the bundled skill directory, the `skill_id` value, skill-descriptor in KG seeds
- **Ambiguous (FLAG):** could read either way — add clarifying context and flag for PR review

Run `grep -rn "old-name\|old_name" <file>` on every in-scope file and annotate each match. The annotated grep output is the auditable artifact in the PR description.

### 2. Atomic commit for directory + registry

The skill directory rename (`git mv`), `skill.toml` `name=` update, and all Rust skill-ID literal changes must land in one atomic commit. Splitting them creates an interim broken-build state where the registry references a directory that no longer exists.

### 3. Touchpoints checklist

| Surface | Files | What changes |
|---------|-------|-------------|
| Skill directory | `skills/bundled/<name>/` | `git mv` to new name |
| Skill manifest | `skill.toml` `name=` field | Update to new name |
| System prompt | `system_prompt.md` | Update self-references |
| Tools schema | `tools.json` | Update description text (tool name stays if it names the app) |
| Well-known agents | `well_known_agents.rs` disabled_skills lists | Update all occurrences |
| Agent tests | `agent.rs` test fixtures | Update skill name in test data |
| Skills module | `skills/mod.rs` tests + doc comments | Update name, variable names, dependency arrays, skill_dir paths |
| Sibling skill deps | Other skills' `skill.toml` `dependencies` arrays | Update dep name |
| KG seeds | `db/kg_schema.rs` | Audit for skill-descriptor entries (tool entries stay) |
| Eval fixtures | `tests/eval/` | Update skill-meaning references; keep app-meaning metadata |
| Live docs | `CLAUDE.md`, crate-level `CLAUDE.md`, `docs/*.md` | Update skill-meaning only |
| GitHub issues | Open issues referencing old name in skill-meaning context | Edit body or add comment |

### 4. Deploy gate: pre-deploy SQL probe

When doing a clean cutover (no backwards-compat alias), run this probe against the live database before deploying:

```sql
SELECT max(strftime('%s','now') - strftime('%s', updated_at)) AS max_age_seconds
FROM tasks
WHERE status IN ('in_progress','pending');
```

If `max_age_seconds >= 1800` (30 min), halt deploy — a long-running task may reference the old skill ID and would lose work to a fail-fast rejection.

### 5. KG domain graph auto-rebuilds

The domain builder is the sole writer of `skill:*` entity keys, runs once per server boot, and is idempotent. No explicit KG re-index command is needed — `make deploy` → restart auto-rebuilds from the in-memory `SkillRegistry`.

## Why This Matters

Missing a skill-meaning reference creates a runtime failure (skill not found in registry). Incorrectly renaming an app-meaning reference breaks log parsing, metadata queries, or binary invocation. The classification rubric prevents both classes of error by forcing explicit per-line judgment.

## When to Apply

- Renaming any bundled skill in `skills/bundled/`
- Adding a family of skills where naming coherence matters (e.g., `dev-pilot` + `dev-groom` vs. `claude-pilot` + `dev-groom`)
- Any rename where the old name has semantic overlap with a binary, repo, or metadata namespace

## Examples

**Before:** Skill directory `skills/bundled/claude-pilot/` with `name = "claude-pilot"` — same name as the `claude-pilot` binary, creating ambiguity in prompts and docs.

**After:** Skill directory `skills/bundled/dev-pilot/` with `name = "dev-pilot"` — names the role (dispatch), while the tool `run_claude_pilot` still names the launcher app.

**Classification example:**
```
# App-meaning (KEEP):
"Launching claude-pilot" → describes the binary
"[claude-pilot]" → log channel marker
"metadata.claude_pilot.branch" → app-level metadata namespace

# Skill-meaning (RENAME):
dependencies = ["claude-pilot"] → skill dependency
disabled_skills: "claude-pilot" → skill registry reference
```

## Related

- mika#844 — the rename that established this pattern
- mika#806 — parent debt for broader skill-routing eval coverage
- `docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md` — scope verification discipline used before filing
