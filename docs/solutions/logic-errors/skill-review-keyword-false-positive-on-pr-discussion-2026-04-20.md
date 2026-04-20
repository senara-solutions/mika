---
title: "skill-review fires on PRs that discuss skill-review (keyword false positive)"
module: skills/matcher, skills/skill-review
date: 2026-04-20
problem_type: logic_error
component: assistant
severity: high
symptoms:
  - "skill-review activates during mika-qa PR review turns when PR title/body mentions 'skill-review'"
  - "mika-qa calls review_skill tool and writes qa-review variants during a normal PR review"
  - "required_tools = ['review_skill'] constraint enforced on turns where no skill review was requested"
root_cause: config_error
resolution_type: config_change
tags:
  - keyword-matching
  - substring-false-positive
  - skill-review
  - webhook
  - meta-skill
---

# skill-review fires on PRs that discuss skill-review (keyword false positive)

## Problem

`skill-review` is a keyword-matched skill with keywords `["review skill", "adapt skill", "generate variant", "tune prompt", "skill variant"]`. When a GitHub webhook delivers a PR event whose title or body discusses the skill-review feature (e.g., PRs #573, #574 which fix skill-review itself), the substring matcher in `match_skills()` triggers on phrases like "review skill" appearing in the PR body. This loads skill-review's full system prompt and enforces `required_tools = ["review_skill"]`, causing the agent to run variant generation when it should be doing a normal PR review.

## Symptoms

- `loaded_skills` includes `skill-review` during mika-qa's PR review turn
- Agent calls `review_skill` tool on `qa-review` unprompted
- The `required_tools` constraint forces the agent to call `review_skill` even when the turn intent is a normal PR review
- Confirmed via: `sqlite3 ~/.mika/data/mika.db "SELECT tool_name, skill_name FROM tool_calls WHERE trace_id='<turn-trace>'"`

## What Didn't Work

- **PR #574 (`review_filter`):** Added `apply_review_filter()` to exclude the *reviewed* skill from keyword matching when skill-review is active. This prevents collateral activation of other skills during a review turn, but does NOT prevent skill-review itself from falsely triggering. The filter assumes skill-review was correctly keyword-triggered and cleans up downstream — it cannot prevent the upstream false activation.
- **Name-in-keywords validation (#510):** `validate_skill()` rejects skills that list their exact name as a keyword. This prevents `"skill-review"` as a keyword but not partial matches like `"review skill"` (the reverse).

## Solution

Remove all keyword triggers from `skill-review`, making it dependency-only and explicit-invocation-only.

**Before** (`skills/bundled/skill-review/skill.toml`):
```toml
[triggers]
keywords = ["review skill", "adapt skill", "generate variant", "tune prompt", "skill variant"]
```

**After:**
```toml
[triggers]
keywords = []
```

The `review_skill` tool remains registered and callable. The skill loads only when:
1. Another skill declares `dependencies = ["skill-review"]` in its `skill.toml`
2. A user directly invokes the `review_skill` tool

The `required_tools = ["review_skill"]` constraint is intentionally kept but becomes unreachable — it only fires on `MatchReason::Keyword` (#265), and with no keywords it never triggers.

## Why This Works

The root cause is that keyword matching uses pure substring matching (`message_lower.contains(kw)`) in `matcher.rs`. Any keyword containing common words like "skill" or "review" will inevitably false-match on meta-discussion about the skill system itself. For a meta-skill whose entire job is to operate on other skills, keyword-based activation is inherently dangerous — the skill's domain vocabulary overlaps with normal discussion about skills.

Making skill-review dependency-only eliminates the false-positive class entirely. No substring match can trigger it because there are no substrings to match.

## Prevention

1. **Meta-skills should avoid keyword triggers.** Skills that operate on other skills (variant generation, review, analysis) should use dependency-based activation or explicit tool invocation, not keyword matching. Their domain vocabulary inherently overlaps with normal discussion.
2. **Known pattern:** This is a documented class of bug — see `project_keyword_substring_false_positives.md` in auto memory. The substring matching engine remains unchanged; any future skill with common-word keywords is susceptible.
3. **Test coverage:** Four unit tests in `matcher.rs` now verify that a skill with empty keywords is not keyword-matched by PR discussion, old keyword phrases, or meta-references — and that dependency loading still works.

## Related

- Issue: [#576](https://github.com/senara-solutions/mika/issues/576)
- PR #574: review_filter (fixes the collateral activation problem, not the false trigger)
- PR #513: review-target exclusion filter (same area)
- Solution doc: `docs/solutions/architecture-patterns/review-target-exclusion-filter.md`
- Solution doc: `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`
- Solution doc: `docs/solutions/architecture-patterns/skill-system-quality-validation-enforcement.md`
