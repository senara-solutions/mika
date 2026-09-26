---
title: "always_on skill prompts need size headroom — iterative additions silently compound"
date: 2026-04-17
category: prompt-engineering
module: skills-engine, self-dev
problem_type: workflow_issue
component: skill-loading
applies_when:
  - "Adding routing, callback guards, or workflow branches to an always_on skill's system_prompt.md"
  - "Reviewing a stack of consecutive PRs that each add a few hundred bytes of prose to the same always_on skill"
  - "Setting max_prompt_size in skill.toml for a new always_on skill"
tags: [always-on, skill-loading, max-prompt-size, self-dev, size-budget, workflow]
---

# always_on skill prompts need size headroom — iterative additions silently compound

## Context

The self-dev prompt grew from ~29 KB to 33,868 bytes across two consecutive fixes (mika#626, mika#627). Both PRs added necessary content — routing table, callback context check, P3 JSON example, negative instructions. Each PR on its own looked modest (+20-67 lines). The running total crossed `max_prompt_size = 32768` in `skill.toml`, and on next deploy the engine refused to load the skill entirely.

`always_on` skills are the critical case: when the prompt exceeds `max_prompt_size`, `crates/mika-agent/src/skills/index.rs` skips them entirely with an ERROR. Without self-dev loaded, mika-dev loses the whole dev-loop orchestration — milestone routing, callback handling, webhook dispatch — all gone.

## Guidance

1. **Set `max_prompt_size` with real headroom, not at the current size.** For always_on skills, target at least 40% above the current prompt size. 64 KB is the engine ceiling; going up to ~48 KB for a prompt that's 25-35 KB today is reasonable. Treating the limit as a tight budget guarantees the next prompt change will trip it.

2. **When reviewing a PR that adds prose to an always_on skill's `system_prompt.md`, check the cumulative size.** `wc -c skills/bundled/<name>/system_prompt.md` against `max_prompt_size` in `skill.toml`. A PR that adds 600 bytes looks fine in isolation — but if it crosses the limit, CI happily merges it and the next deploy breaks production. This is a footgun until we replace size policing at the config level.

3. **Iterative guard/routing additions are a recurring pattern.** When the same skill gets two or three consecutive PRs that each add explicit "do NOT" negative instructions for an edge case, treat size growth as a design smell: there's duplication to dedup, or the workflow needs restructuring, not more prose.

4. **`always_on` and non-`always_on` fail differently.** Over-size `always_on` skills fail loudly (ERROR, skill not loaded). Over-size non-`always_on` skills fail silently — the prompt is emptied but the skill still loads with its tools defined. The silent failure is the worse case; it produces a skill that triggers and generates garbage. Track this class of bug under mika#630 (hard-skip symmetry).

## Why This Matters

`always_on` skills participate in every turn. When one doesn't load:
- Downstream agents that depend on it (webhook handlers, callback routers) degrade or misroute
- Test runs that exercise the workflow stop working
- Live sessions silently lose capabilities with no warning — the only signal is an ERROR line at startup

The cost of an emergency bump + redeploy is small (single-line change). The cost of not noticing until mika-dev misroutes a milestone is much larger. A review check that takes 10 seconds (`wc -c` vs `grep max_prompt_size`) prevents the whole class.

## When to Apply

- Editing `skills/bundled/<skill>/system_prompt.md` for any skill with `always_on = true`
- Setting initial `max_prompt_size` on a new always_on skill
- Reviewing PRs that add prose, negative instructions, or JSON examples to orchestration skills (self-dev, qa-review, permission-policy)
- Diagnosing "skill not loaded" ERRORs at agent startup

## Examples

**Before the bump (mika#628 state):**

```toml
# skills/bundled/self-dev/skill.toml
max_prompt_size = 32768
```

```
wc -c skills/bundled/self-dev/system_prompt.md
33868
```

Result: `ERROR always_on skill prompt exceeds size limit — skill NOT loaded.` on both mika-dev and mika-qa.

**After the bump:**

```toml
max_prompt_size = 49152
```

~45% headroom above the current 33,868 bytes, 25% below the 64 KB engine ceiling. Good for several more iterations before needing to revisit.

**Review-time check (drop into PR review checklists for `always_on` skill changes):**

```bash
# For each always_on skill's system_prompt.md modified in the diff:
for f in $(git diff --name-only main...HEAD | grep 'system_prompt.md$'); do
  skill_dir=$(dirname "$f")
  if grep -q 'always_on = true' "$skill_dir/skill.toml" 2>/dev/null; then
    limit=$(grep '^max_prompt_size' "$skill_dir/skill.toml" | awk '{print $3}')
    size=$(wc -c < "$f")
    printf "%s: %d / %d bytes (%d%% of limit)\n" "$f" "$size" "$limit" "$((size * 100 / limit))"
  fi
done
```

## Related

- mika#628 — emergency bump (this solution)
- mika#629 — move enabled state to DB (removes one category of silent filtering)
- mika#630 — hard-skip non-always_on overflow symmetrically + split startup log (removes the silent-empty-prompt footgun)
- mika#12 milestone — Skills loading cleanup (the architectural follow-up that removes the need to police size at the config level)
- `crates/mika-agent/src/skills/index.rs:488-540` — overflow handling
- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` — related prompt-engineering lesson on enforcement
