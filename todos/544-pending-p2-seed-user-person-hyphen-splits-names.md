---
status: pending
priority: p2
issue_id: "544"
tags: [code-review, bugfix, agent]
dependencies: []
---

# seed_user_person splits hyphenated names incorrectly

## Problem Statement

`seed_user_person` in `agent.rs` splits `user_summary` on `[',', '.', '\u{2014}', '-', '\n']` to extract the user's name. The hyphen `-` in the split set causes hyphenated names like "Jean-Pierre" or "Mary-Anne" to be truncated to "Jean" or "Mary".

## Findings

- **Source:** Code simplicity review agent
- **Location:** `crates/mika-agent/src/agent.rs` line 512
- **Evidence:** `split(&[',', '.', '\u{2014}', '-', '\n'][..])` — the `-` character splits "Jean-Pierre, engineer" into ["Jean", "Pierre, engineer"] and takes "Jean"

## Proposed Solutions

### Option A: Remove hyphen from split characters
- **Approach:** Change to `split(&[',', '.', '\u{2014}', '\n'][..])`
- **Pros:** One-character fix, handles common hyphenated names
- **Cons:** Edge case where dash is used as a separator (e.g., "Sam - software engineer") would extract "Sam " (with trailing space, but `.trim()` handles that)
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/agent.rs`

## Acceptance Criteria

- [ ] "Jean-Pierre, engineer" extracts "Jean-Pierre"
- [ ] "Sam - software engineer" extracts "Sam" (trim handles trailing space)
- [ ] Existing tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Hyphenated names are common enough to warrant this fix |

## Resources

- PR branch: `feat/unified-task-engine`
