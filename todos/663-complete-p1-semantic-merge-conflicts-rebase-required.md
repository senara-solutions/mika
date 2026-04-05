---
status: wont_fix
priority: p1
issue_id: "663"
tags: [code-review, merge, compilation]
dependencies: []
---

# Semantic Merge Conflicts — Rebase Required Before Merge

## Problem Statement

PR #71 (`feat/71/multi-provider-llm-support`) has diverged 63 commits from main. A trial merge reveals 1 textual conflict and ~24 semantic conflicts where git auto-merge succeeds but the result **will not compile**. Main has added significant features (trace_id propagation, multi-agent Telegram delivery, team workspace restructure, session parent tracking, log format, dashboard endpoints) that touch the same struct constructors and function signatures this PR modifies.

## Findings

### Missing struct fields (compilation errors)

| File | Struct/Call | Missing from PR |
|------|-----------|-----------------|
| `server/handlers.rs:220-240` | `AgentParams` | `trace_id` field |
| `cli/commands/chat.rs:237,326` | `AgentParams` | `trace_id` field |
| `cli/commands/ask.rs` | `AgentParams` | `trace_id` field |
| `teams/engine.rs:872-887` | `TeamAgentParams` | `message_sender`, `trace_id` fields |
| `tools/delegate_task.rs:144-159` | `TeamAgentParams` | `message_sender`, `trace_id` fields |
| `tools/delegate_task.rs` struct def | `DelegateTaskTool` | `http_client` field |
| `task_engine/dispatcher.rs` (4 sites) | `SilentAgentParams` | `trace_id` field |
| `teams/engine.rs` | `TeamEngine` | `reference_run_id` field |
| `test_utils.rs:170-196` | `dummy_settings()` | `log_format` field |
| `server/mod.rs:test_state()` | `Settings` literal | `log_format` field |

### Missing function parameters (compilation errors)

| File | Function | Missing param |
|------|----------|---------------|
| `server/handlers.rs:211-217` | `GatewayMessageSender::new()` | `agent_name`, `chat_id` |
| `server/mod.rs:178-184` | `GatewayMessageSender::new()` | `agent_name`, `chat_id` |
| `server/handlers.rs:536-542` | `GatewayMessageSender::new()` | `agent_name`, `chat_id` |
| `cli/init.rs:171-177` | `GatewayMessageSender::new()` | `agent_name`, `chat_id` |
| `cli/commands/chat.rs:88` | `management_tools_if_needed()` | `http_client` |
| `cli/commands/chat.rs:103-104` | `make_message_sender()` | `agent_name` |
| `cli/commands/ask.rs:53-54` | `make_message_sender()` | `agent_name` |
| `teams/engine.rs:67-71` | `init_resources()` | `run_id`, `reference_workspace` |
| `teams/mod.rs:20-27` | `run_team()` | `reference_run_id` |
| `cli/commands/chat.rs:619,653-660` | `run_team()` calls | `run_id`, `reference_run_id` |

### Signature/behavior mismatches

| File | Issue |
|------|-------|
| `teams/engine.rs:171` | `new_for_resume` must be `async` (main changed it), missing trace_id restoration logic |
| `teams/engine.rs:73` | Uses `workspace_dir()` instead of `workspace_run_dir()` |
| `server/mod.rs` | Missing `is_pretty`/`print_banner` startup logging |
| `server/mod.rs` | Missing dashboard routes (`/tasks`, `/tasks/{task_id}`, `/team-runs`) |
| `task_engine/dispatcher.rs` | Missing `write_execution_trace()`, `end_session()` calls, `create_session_with_parent` |

## Proposed Solutions

### Option 1: Rebase onto main (Recommended)

```bash
git fetch origin main
git rebase origin/main
```

**Pros:** Each commit replayed cleanly, easy to verify; git history is linear.
**Cons:** More conflict resolution steps (9 commits).
**Effort:** Medium (2-3 hours). Most fixes are mechanical field additions.

### Option 2: Merge main into branch

```bash
git merge origin/main
```

**Pros:** Single merge commit, faster.
**Cons:** Large merge commit harder to review; hides individual changes.
**Effort:** Medium (1-2 hours).

## Recommended Action

Rebase (Option 1). Resolution is mechanical: at each conflict point, keep the PR's `llm`/`LlmProvider` abstraction AND add the new fields/params from main. The PR's core abstractions are sound — the issue is purely temporal divergence.

## Technical Details

- **Affected files:** ~15 files across mika-agent, mika-cli, mika-common
- **Components:** server, CLI, teams engine, task dispatcher, delegate_task tool
- **Database changes:** None (no schema conflicts)

## Acceptance Criteria

- [ ] `cargo build` succeeds after rebase
- [ ] `cargo test` passes (~1317+ tests)
- [ ] `cargo clippy` clean
- [ ] All main features preserved: trace_id propagation, agent_name in senders, workspace_run_dir, dashboard routes, log_format, reference_run_id
- [ ] PR diff reviewed post-rebase to confirm no regressions

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-16 | Initial discovery via trial merge and semantic analysis | 63 commits on main since branch point; auto-merge hides most issues |

## Resources

- PR #71: multi-provider LLM support
- Main features since branch: PRs #157, #159, #163, #167, #168, #169, #171, #172, #174
