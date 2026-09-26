---
status: complete
priority: p2
issue_id: 297
tags: [code-review, skills, quality]
dependencies: []
---

# Remove "cat" keyword from file-reader skill triggers

## Problem Statement

The file-reader skill's keyword list includes `"cat"`, which is only 3 characters and matched via case-insensitive substring matching. This triggers false positives on any message containing "cat" as a substring: "my cat is sick", "category", "concatenate", "catalog", "education", etc. The skill would activate unnecessarily, injecting the file-reader tool and system prompt into the agent's context.

## Findings

- **Code Simplicity Reviewer:** `"cat"` is a false-positive magnet with substring matching on 3 characters. Other keywords are multi-word phrases that are far more specific.
- **Pattern Recognition Specialist:** The file-reader now has 9 keywords vs the 4-5 norm for other skills. The expansion is defensible for better activation coverage, but `"cat"` stands out as too short and ambiguous.
- Keyword matcher in `crates/mika-agent/src/skills/matcher.rs` does `message_lower.contains(&kw)` — any substring match fires.

## Proposed Solutions

### Option 1: Remove "cat" keyword (Recommended)
- **Pros:** Eliminates all false positives from the short keyword. Users who want to read a file will use one of the other 8 keywords naturally.
- **Cons:** Users who literally type "cat /etc/hosts" won't trigger the skill. But "read file" or "show file" or "content of" will still match.
- **Effort:** Small (one keyword removal)
- **Risk:** Low

### Option 2: Replace "cat" with "cat file"
- **Pros:** More specific, avoids false positives while preserving the "cat" intent.
- **Cons:** Won't match "cat /etc/hosts" (no "file" word), but that's a niche use case.
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **File:** `templates/skills/file-reader/skill.toml` line 9
- **Matcher:** `crates/mika-agent/src/skills/matcher.rs`

## Acceptance Criteria

- [ ] `"cat"` keyword removed or replaced with a more specific phrase
- [ ] Manual test: message "my cat is cute" does NOT trigger file-reader skill
- [ ] Manual test: message "read the file ~/.bashrc" still triggers file-reader skill

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Substring matching on short keywords causes false positives |
