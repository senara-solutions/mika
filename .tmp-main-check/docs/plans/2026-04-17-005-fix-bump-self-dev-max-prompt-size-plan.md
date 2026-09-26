---
title: "fix: Bump self-dev max_prompt_size to unblock agent loading"
type: fix
status: active
date: 2026-04-17
---

# fix: Bump self-dev max_prompt_size to unblock agent loading

## Overview

Raise `max_prompt_size` in `skills/bundled/self-dev/skill.toml` from 32768 to 49152 so the engine stops refusing to load the self-dev skill. Emergency unblock; architectural cleanup lives in #629 and #630.

## Problem Frame

`skills/bundled/self-dev/system_prompt.md` is 33,868 bytes. The skill's `max_prompt_size = 32768` in `skill.toml` is now exceeded. Because self-dev is `always_on`, the engine refuses to load it entirely (`crates/mika-agent/src/skills/index.rs:488-540`). Self-dev is broken on both mika-dev and mika-qa, which blocks the dev loop orchestration (milestone/project workflows, callback routing, webhook handlers).

## Requirements Trace

- R1. mika-dev loads self-dev successfully after deploy (no "prompt exceeds size limit" ERROR at startup; name appears in `mika skills --agent mika-dev list`)
- R2. mika-qa loads self-dev successfully after deploy
- R3. Value stays well under the 64 KB engine ceiling with headroom for expected growth

## Scope Boundaries

- Does NOT restructure the self-dev prompt to reduce its size
- Does NOT change the engine's overflow handling (that's #630)
- Does NOT move `enabled` state to DB (that's #629)

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/skill.toml` — the file being modified
- `crates/mika-agent/src/skills/index.rs:488-540` — the overflow check that rejects the skill today
- Other bundled skills' `skill.toml` — reference for `max_prompt_size` conventions

### Institutional Learnings

- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` — prompt size growth from iterative fixes is a recurring pattern

## Key Technical Decisions

- **Choose 49152 (48 KB) over a tighter bump like 36864 (36 KB):** Leaves real headroom — roughly 40% above current size — so the next incremental prompt change doesn't immediately trip the limit again. Still 25% below the 64 KB engine ceiling, which is the hard ceiling enforced in `index.rs`.
- **Don't trim the prompt in this PR:** The size growth reflects genuine routing and callback guard content added in #626 and #627. Trimming under time pressure risks regression. #629 and #630 remove the need to police prompt size at the config level.

## Implementation Units

- [ ] **Unit 1: Raise max_prompt_size**

**Goal:** self-dev loads at startup on both mika-dev and mika-qa.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`

**Approach:**
- Change `max_prompt_size = 32768` to `max_prompt_size = 49152`

**Patterns to follow:**
- Other skills' `skill.toml` files for valid TOML syntax

**Test scenarios:**
- Test expectation: none — single-line config change with no behavioral code. Verified via runtime: after deploy, `mika skills --agent mika-dev list` and `mika skills --agent mika-qa list` both show `self-dev` in the `names` array without the "always_on skill prompt exceeds size limit" ERROR.

**Verification:**
- File diff is exactly the numeric change on the `max_prompt_size` line
- Post-deploy runtime check: both agents load self-dev cleanly

## System-Wide Impact

- **Interaction graph:** self-dev is `always_on` — when loaded, its prompt is part of every mika-dev and mika-qa turn. Raising the limit doesn't change content, only admits the current content.
- **Unchanged invariants:** No change to the engine's overflow handling, the matcher, the runtime filter, or any other skill's config.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Limit grows again on the next prompt change | Issues #629 (DB-backed state) and #630 (hard-skip + log split) remove the need to police size at the config level |

## Sources & References

- Related issue: #628
- Related milestone: #12 (Skills loading cleanup)
- Related issues: #629 (P1), #630 (P2)
- Engine overflow logic: `crates/mika-agent/src/skills/index.rs:488-540`
