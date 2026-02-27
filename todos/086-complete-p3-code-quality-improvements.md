---
status: complete
priority: p3
issue_id: "086"
tags: [code-review, quality]
dependencies: []
---

# Code quality improvements from review

## Problem Statement
Multiple P3 code quality improvements identified across the codebase: duplicated reminder query closures, missing agent loop tests, missing features for agent completeness.

## Findings
1. **Reminder query duplication** — db.rs:671-733 has 3 functions with identical `query_map` closures. Also `list_active_reminders` is trivial alias for `get_pending_reminders`.
2. **AsyncDatabase boilerplate** — async_db.rs has 40+ near-identical delegation methods that could use a macro.
3. **No agent loop integration tests** — agent.rs has no `#[cfg(test)]` module. Core control flow untested.
4. **Inconsistent error message quoting** — Some tools use `"'field' is required"`, others use `"field is required"`.
5. **search_memory doesn't include reminders** — user can't search reminders via search_memory tool.
6. **list_reminders doesn't include created_at** — user can't see when reminder was set.
7. **No update_reminder tool** — rescheduling requires cancel+create (2 tool calls).
8. **CLI /help doesn't mention reminders** — no `/reminders` slash command.
9. **Silent mode doesn't inject conversation summary** — heartbeat agent lacks recent conversation context.
10. **Hardcoded channel types** in agent.rs:99 — `["cli", "telegram"]` should be configurable.
11. **Blocking std::fs reads** in async functions — agent.rs:79,299 reads soul.md synchronously.

## Proposed Solutions
Address items individually as time permits. Highest value:
- Extract shared `query_reminders(where_clause)` helper (saves ~45 lines)
- Add mock-based agent loop tests
- Add `/reminders` CLI command

**Effort:** Various, 30min-2hrs each | **Risk:** Low

## Acceptance Criteria
- [x] At least reminder query duplication addressed
- [x] Agent loop has basic test coverage

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
**Actions:** Consolidated 11 minor findings from 7 review agents

### 2026-02-27 - Resolution
**By:** Claude Code
**Actions:**
- Verified reminder query duplication already resolved via `ReminderFilter` enum + `query_reminders()` helper (db.rs:848-877)
- Added 13 unit tests to agent.rs `#[cfg(test)]` module covering: LoopMode variant properties (3 tests), check_onboarding async behavior (3 tests), build_skill_tool_map (2 tests), max_skill_timeout (2 tests), inject_skills_and_resolve_tools (3 tests including deduplication and empty snippet handling)
