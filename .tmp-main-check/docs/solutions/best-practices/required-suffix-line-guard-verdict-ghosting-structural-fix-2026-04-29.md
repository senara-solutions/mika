---
title: "Required-suffix-line EndTurn guard — structural fix for verdict ghosting under cognitive load"
date: 2026-04-29
category: best-practices
module: agent-loop, skills-manifest
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - A skill declares a structured output contract (verdict line, disposition, status)
  - Prompt-level "MUST" enforcement fails under cognitive load or token pressure
  - Downstream consumers silently fall through when expected structural lines are absent
  - Designing new skills with closed-alphabet output surfaces
related_components:
  - agent-loop
  - skills-manifest
  - mika-arch
tags:
  - verdict-ghosting
  - structural-guard
  - endturn-post-condition
  - required-suffix-lines
  - prompt-enforcement-fragile
  - skill-manifest
  - output-contract
---

# Required-suffix-line EndTurn guard — structural fix for verdict ghosting under cognitive load

## Context

Prompt-level output discipline does not bind under load. The mika-arch second-review skill's system prompt declares output MUST end with `Verdict: GROOMED` or `Verdict: ESCALATE`. Under cognitive load (token-budget pressure + self-correction loop after recognizing a procedural failure), the architect end-turned with no verdict line at all — just a meta-acknowledgment that read like a verdict but contained no parseable keyword.

Trace `03d3ec38-0839-47b6-9226-111b38d8b52b` shows the architect literally read its own core-memory catalogue of the prior occurrence and ghosted the verdict line anyway. The downstream disposition parser in `/mika-groom-ticket` Phase 4 silently fell through to a default path.

This is a structural problem: the skill's output contract lived only in prompt prose. The agent engine had no awareness of "this skill's output must end with a verdict line." Per `feedback_prompt_enforcement_fragile.md`, prompt-level enforcement rationalizes away under cognitive load. The fix must be structural.

## Guidance

### Manifest-driven opt-in via `[output]` section

Skills that need to enforce a verdict-line contract declare the accept-set in `skill.toml`:

```toml
[output]
required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
```

Key design decisions:

1. **Exhaustive literal list, not regex.** Closed alphabets (GROOMED|ESCALATE for second-review, READY|ITERATE|ESCALATE for first-pass) compose better with `validate_skill()` and avoid the regex-author footgun (forgotten anchors, unescaped colons, greedy capture).

2. **Last-3-non-empty-lines scan.** Assistant text often ends with markdown trailing whitespace, an empty line, or a code-fence close. Scanning the last 3 non-empty lines (after `trim()`) accommodates formatting tails while keeping the verdict near the end.

3. **Single-retry semantics.** On violation, the guard injects a corrective system message naming the accept-set and instruction to re-emit with the verdict appended. The guard fires once (via `required_suffix_line_retry_done` flag), matching all other EndTurn guards.

4. **End-of-chain position.** The guard runs after all other post-condition guards (text-tool-call, prose-style, required-tools, completion-claim, fabricated-action, intent-precondition, asserted-unavailability, persistence-eval). Other guards' rejections take precedence so a turn rejected for a more fundamental reason doesn't waste a suffix-line check.

5. **AlwaysOn + Keyword match scope.** Both `Keyword` and `AlwaysOn` matched skills contribute suffix requirements (unlike `required_tools` which is Keyword-only). Suffix-line contracts apply whenever the skill's prompt is active. `Dependency`-matched skills do not contribute.

### Validation at skill-load time

`validate_skill()` catches authoring errors early:
- Empty/whitespace entries → hard fail (skill won't load)
- Explicit empty list `required_suffix_lines = []` → warn (suspicious but not a correctness violation)

### Currently opted-in skills

| Skill | Accept-set |
|-------|-----------|
| `mika-arch-second-review` | `["Verdict: GROOMED", "Verdict: ESCALATE"]` |
| `mika-arch-groom-ticket` | `["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]` |

## Why This Matters

Prompt-level "MUST" enforcement is the most common structural assumption in skill design — and the most fragile. The failure mode is insidious: the agent reads its own failure history, acknowledges the pattern, and still ghosts the verdict line under load. The downstream parser silently falls through. No error is raised. The skill appears to have completed normally.

The structural counterpart (manifest-declared accept-set + EndTurn check) makes the contract enforceable by the engine rather than dependent on the model's attention under pressure. This is the same class of fix as `required_tools` (#270) and `asserted_unavailability` (#862) — prompt-level discipline replaced by structural enforcement.

## When to Apply

- Any skill with a closed-alphabet output surface (verdict, disposition, status, classification)
- Any downstream consumer that parses the last line of a skill's output for routing
- When prompt-level "MUST" language has been observed to fail under load (see `required-tools-gate-evasion-patterns-2026-04-28.md` for the pattern taxonomy)

Do NOT apply when:
- The skill legitimately has free-form output (most skills)
- The output surface is open-ended (natural language responses, analysis)
- Regex would be needed (the accept-set is not a finite literal list)

## Examples

### Before: prompt-only enforcement (fragile)

```markdown
<!-- system_prompt.md -->
Your response MUST end with exactly one of:
- Verdict: GROOMED
- Verdict: ESCALATE
NEVER return ITERATE.
```

Under cognitive load, the model rationalizes past this constraint and end-turns without the verdict line.

### After: manifest-driven enforcement (structural)

```toml
# skill.toml
[output]
required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
```

The engine checks the last 3 non-empty lines on EndTurn. Missing match → corrective re-prompt → retry. The model cannot rationalize past a structural check.

## Cross-references

- **mika#864** — implementation issue
- **mika#788** — concrete failure (verdict line ghosted under load)
- **Trace `03d3ec38`** — canonical pre-fix evidence
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — companion doc documenting the evasion pattern taxonomy
- `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — meta-pattern this instantiates
- `feedback_prompt_enforcement_fragile.md` — meta-rule on prompt-level enforcement fragility
