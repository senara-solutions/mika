---
title: Shared dispatch library for claude-pilot skills
date: 2026-04-29
last_updated: 2026-05-17
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new skill that dispatches headless Claude Code sessions via claude-pilot
  - Modifying worktree setup, slug derivation, env scrubbing, or callback delivery for any dispatch skill
  - Debugging divergent behavior between dispatch skills (different branches, different worktree paths for the same ticket)
tags:
  - skills
  - claude-pilot
  - dispatch
  - shared-library
  - dev-pilot
  - dev-groom
  - slug-derivation
  - worktree
---

# Shared dispatch library for claude-pilot skills

> **SUPERSEDED IN PART (2026-05-17, mika#1173).** The union-enum-on-host
> tool registration described below ("dev-pilot owns the only `run_claude_pilot`
> tool with `skill.enum = ["dev-pilot", "dev-groom"]`; dev-groom is prompt-only,
> no `tools.json`, no handler") regressed five times between 2026-05-02 and
> 2026-05-17. It was reverted to per-skill tool registration in mika#1173.
>
> Current guidance for **tool registration**: see
> `per-skill-tool-registration-for-dispatch-family-2026-05-17.md`. Each dispatch
> skill owns its own tool (`run_claude_pilot` for dev-pilot,
> `run_claude_pilot_groom` for dev-groom). Adding a new sibling now requires
> creating that sibling's own `tools.json` + `handlers/run.sh` (still a thin
> 6-line wrapper).
>
> The rest of this doc — the **shared library itself** (`_shared/dispatch-lib.sh`),
> centralized slug derivation, env scrubbing, EXIT trap, callback delivery,
> worktree reuse — remains correct and load-bearing. Only the tool-registration
> shape changed. The case switch on `$SKILL` inside the lib still routes both
> tools' handlers; per-skill ownership is about the surface mika-dev sees, not
> the plumbing.

## Context

dev-pilot and dev-groom are conceptually the same dispatch shape: both wrap a headless Claude Code session via claude-pilot, both create/reuse a worktree on a derived branch, both pass operator-supplied context as the entry prompt. The only meaningful difference is the slash command claude-pilot enters with: `/mika` (dev-pilot) vs `/mika-groom-ticket` (dev-groom).

Before this refactoring (#893), they were maintained as independent copies. dev-pilot's handler was 441 LOC, dev-groom's was 133 LOC. The divergence produced concrete drift:
- dev-groom's slug derivation was inline (not using the centralized `scripts/derive-branch-name`), causing branch name mismatches between skills for the same ticket (#892).
- dev-groom's `tools.json` was empty `[]`, meaning its `run_claude_pilot` tool worked only by accident (global tool namespace collision from dev-pilot's registration).
- Bug fixes to one handler (e.g., mika#887 BASH_XTRACEFD trace injection) had to be manually ported to the other.

## Guidance

> **Tool registration shape changed in mika#1173** — see banner above and
> the superseding doc. The description below reflects the pre-2026-05-17
> design (union-enum on host). The shared library plumbing it describes
> is still current; only the tool-ownership model changed.

All claude-pilot dispatch skills share a single library at `skills/bundled/_shared/dispatch-lib.sh`. (Pre-mika#1173:) One host skill (dev-pilot) owned the `run_claude_pilot` tool with a union-enum `skill` parameter (`["dev-pilot", "dev-groom"]`); sibling skills (dev-groom) were prompt-only — they provided `skill.toml` + `system_prompt.md` but no `tools.json` or handlers. The host skill's handler sourced the library and called the single entrypoint:

```bash
#!/bin/bash
set -e
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot  # entry command derived from $SKILL via case switch in lib
```

The shared library provides:
- **`dispatch_claude_pilot`** — single API surface (no args; derives entry command from `$SKILL`)
- JSON input parsing, skill validation, UUID format warning
- GitHub App auth setup (with GH_TOKEN env var detection per mika#520)
- Env var scrubbing (MIKA_* secrets)
- Centralized slug derivation via `scripts/derive-branch-name` (mika-platform#58)
- Worktree creation with idempotent reuse and rebase-or-abort guard
- BASH_XTRACEFD diagnostic trace (mika#887)
- EXIT trap for crash-recovery callback delivery
- claude-pilot subprocess invocation with stdout/stderr capture
- Post-flight diff check (zero-commit detection)
- PR URL discovery for callback enrichment (mika#138)
- Result truncation (90KB cap for 100KB callback limit)
- Callback delivery via `mika ask --task-id`

Internal helpers are underscore-prefixed (`_parse_input_json`, `_set_up_worktree`, etc.) and not part of the contract.

### Build-time discovery exclusion

The `_shared/` directory is excluded from bundled skill discovery via two layers:
1. **Primary:** `bundled_skills_discover.rs` skips directories starting with `_` (convention-reserved for non-skill support directories).
2. **Defense-in-depth:** Directories without `skill.toml` are already skipped by the existing `toml_is_real_file` check.

### Tool registration

Tool names in mika-agent are globally namespaced (dedup via `seen.insert()` in `inject_skills_and_resolve_tools()`). Only dev-pilot registers the `run_claude_pilot` tool — with a union enum `["dev-pilot", "dev-groom"]` in the `skill` parameter. Sibling skills (dev-groom) do not register their own `run_claude_pilot`, avoiding the schema collision that previously made dev-groom unreachable (mika#932). The skill→entry-command mapping lives in `_shared/dispatch-lib.sh` as a `case` switch.

## Why This Matters

Without the shared library:
- Every bug fix to dispatch logic must be manually ported to all sibling skills.
- Slug derivation drift causes branch name mismatches, leading to duplicate worktrees and merge conflicts.
- New sibling skills (e.g., a hypothetical `dev-explore`) require copying 400+ LOC and keeping it in sync.

With it:
- Adding a third sibling skill requires only `skill.toml` + `system_prompt.md` (prompt-only), a new arm in the lib's `case` switch, and widening `dev-pilot/tools.json` `skill.enum`. No new handler or `tools.json` needed.
- Slug derivation is centralized by construction — drift is structurally impossible.
- Bug fixes land once and propagate to all consumers.

## When to Apply

- **Adding a new dispatch skill:** Create a prompt-only skill (`skill.toml` + `system_prompt.md`), add a `case` arm in `_shared/dispatch-lib.sh`, widen `dev-pilot/tools.json` `skill.enum`, and update `self-dev/system_prompt.md` to teach mika-dev when to dispatch. No handler or `tools.json` needed on the sibling.
- **Fixing dispatch behavior:** Edit `_shared/dispatch-lib.sh` once. All skills pick up the fix.
- **Adding a new non-skill support directory:** Use the `_` prefix convention (e.g., `_templates/`, `_fixtures/`) to keep it excluded from bundled skill discovery.

## Examples

**Before (dev-groom inline slug derivation — diverged from centralized script):**
```bash
# Inline in dev-groom/handlers/run.sh (133 LOC)
case "$LABELS" in
    *enhancement*) TYPE="feat" ;;
    *bug*)         TYPE="fix" ;;
    *)             TYPE="chore" ;;
esac
SLUG=$(printf '%s' "$SLUG_BODY" | tr '[:upper:]' '[:lower:]' | ...)
BRANCH="${TYPE}/${ISSUE}/${SLUG}"
```

**After (centralized via shared library, mika#932):**
```
# dev-groom is now prompt-only (skill.toml + system_prompt.md only).
# No handlers/, no tools.json. The host skill (dev-pilot) owns the
# run_claude_pilot tool and the lib derives the entry command from $SKILL.
```

```bash
# dev-pilot/handlers/run.sh (6 LOC) — the only handler
#!/bin/bash
set -e
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot  # entry command derived from $SKILL
```

Slug derivation calls `$PLATFORM_DIR/scripts/derive-branch-name` inside `_set_up_worktree()`, matching all sibling skills by construction.

## Related

- mika#932 — consolidated `run_claude_pilot` to single host skill (dev-pilot) with union enum; dev-groom became prompt-only
- mika#893 — original refactoring that introduced `_shared/dispatch-lib.sh`
- mika#892 — slug derivation drift in dev-groom (closed by construction)
- mika#887 — BASH_XTRACEFD trace injection (migrates to shared lib naturally)
- mika-platform#58 — centralization of slug derivation scripts
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — related skills system documentation
