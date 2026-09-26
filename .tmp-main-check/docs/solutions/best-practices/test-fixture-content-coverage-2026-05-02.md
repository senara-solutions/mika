---
module: mika-agent
tags: [testing, fixtures, permission-classifier, content-coverage, security]
problem_type: test-coverage-gap
category: best-practices
date: 2026-05-02
related_issues: [935, 937, 938]
---

# Test Fixture Content Coverage Discipline

## Problem

Test fixtures for parsers and classifiers that process user-controlled content inside structured commands must cover both **command structure** (flag ordering, compound separators, quoting variants) and **representative message content** (production-realistic payloads with their characteristic metacharacters). Covering only structure leaves the parser exposed to corner cases that only surface when real production traffic flows through.

## Grounding Cases (N=2)

### Case 1: PR #937 (mika#935 Unit 2) — Structure-only fixtures

The structural pre-classifier shipped in PR #937 with 22 test fixtures across 6 corner-case categories:

- Environment variable prefixes
- Compound command separators (`&&`, `||`, `;`)
- Flag re-ordering (`mika ask "msg" --agent mika-arch`)
- `--` double-dash separator
- `--agent=<peer>` equals form
- Quoting variants (double, single)

All fixtures used short ASCII messages (`"Test"`, `"hello"`, `"test"`). None contained production-representative message content.

**Result:** Canary v7 on mika#931 (2026-05-02 18:25 UTC, session `4cfc594f-008a-4038-b132-618751c0f569`) hit the blanket backtick/`$(` rejection when `/mika-ask-arch` sent a markdown brief containing 30 backticks (inline code spans like `` `docs/plans/<file>.md` ``). The pre-classifier's `command.contains('`')` check saw the backtick inside the quoted message argument and rejected the entire command — a false-positive that broke the two-pass groom pipeline.

### Case 2: mika#938 — Content-payload fixtures added

The remediation replaced blanket `String::contains` with quote-aware `contains_unquoted_metacharacter()` and added 23 new fixtures covering message-content variants:

- Backticks inside double-quoted message (markdown inline code)
- `$()` inside single-quoted message
- Escaped inner quotes with backticks (`\"escaped\" and \`backtick\``)
- Combined flags + session-id with backtick content
- Equivalent forms on `mika-dev`, `mika-qa` peers
- Unterminated quote conservative handling
- Negative cases: metacharacters outside quotes, TIER 3 inside quotes, compound injection

## Generalized Rule

When testing parsers or classifiers that process user-controlled CONTENT inside structured commands, fixtures must enumerate both:

**(a) Command-structure variants** — flag ordering, compound separators, quote types, env-var prefixes, pipe targets, absolute paths.

**(b) Content-payload variants** — representative production message shapes including their characteristic metacharacters (markdown backticks, dollar-paren in code spans, escaped quotes, file paths with special characters, heredoc content, etc.).

Skipping (b) leaves the parser exposed to corner cases that only surface when real production traffic flows through.

## When This Applies

- Permission classifiers processing tool commands with message arguments
- Prompt parsers that handle user-controlled content inside structured envelopes
- Any parser where the structural grammar is separate from the content payload

## Escalation

This doc covers N=2 grounding cases. When the next instance surfaces (N=3), append to this doc rather than authoring a new entry. If a third case lands in a different module (not `permission_pre_classifier.rs`), promote this from a module-specific pattern to a cross-cutting testing discipline.

## References

- PR #937: https://github.com/senara-solutions/mika/pull/937
- mika#935: structural pre-classifier origin issue
- mika#938: parser gap fix (this remediation)
- Canary v7 session log: `/var/log/claude-pilot/4cfc594f-008a-4038-b132-618751c0f569.log`
