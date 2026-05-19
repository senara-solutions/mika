# Plan: post-crash send_message guard + agent-switch teardown hardening (mika#1150 F2, PR #1185)

> **Status: retroactive** — this plan documents work shipped in PR #1185 on 2026-05-17, retroactively groomed on 2026-05-19 to unblock qa-review's pipeline gate (mika-platform-qa was emitting `block[pipeline]` on the missing plan-callout). F1 and F4 from the original mika#1150 cohort remain in that ticket for separate dispatch.

type: bug (lifecycle hardening)
ticket: mika#1150 (F2 + partial F3 only)
date: 2026-05-19 (retroactive grooming; implementation shipped 2026-05-17)
pr: mika#1185
branch: `feat/1150/send-message-guard-cohort`
groomed-via: peer-Claude (mika-arch blocked on milestone-workflow class by mika#1207)

## Scope statement (load-bearing)

This plan covers **F2 (post-crash send_message guard) plus partial F3 (Quit-first agent-switch teardown)** from mika#1150's original four-item cohort filed 2026-05-16. **F1 and F4 remain in mika#1150** for separate grooming + dispatch cycles. The "salvage from #1181" framing in PR #1185's body deferred their grooming explicitly; this plan does not implicitly fold them in.

## Problem

mika#1149 closed the panic-path silent-drop class by introducing a supervisor around the TUI agent-worker `tokio::spawn`. The `/ce:review` pass on PR #1149 surfaced a cohort of lifecycle wiring holes (F1–F4) that compose with the same root pattern (`agent_tx` / `agent_rx` / `worker_crashed` / `shutdown_initiated` state machine).

This plan addresses F2 specifically:

`crates/mika-cli/src/tui/app.rs` ~ line 904 — after `worker_crashed = true` is set by the supervisor (mika#1149), nothing guards `send_message_with_thinking`. The supervisor's cloned `agent_tx` keeps the receiver count above zero, so the `let _ = self.agent_tx.send(...)` succeeds; the message sits in the unbounded buffer forever, the "Thinking" spinner appears and never clears. **Same silent-drop class mika#1149 was designed to close, narrowed to the post-crash send window.**

And F3 partially:

`crates/mika-cli/src/commands/chat.rs` ~ line 708 — agent-switch sets `shutdown_initiated=true` and waits 2s for the supervisor, but the old worker's `while let` cannot exit because `app.agent_tx` is still live until line 733. The 2s timeout fires → old worker becomes a background zombie → MCP `shutdown()` never runs → MCP stdio child processes leak. **Fix shape:** send `AgentRequest::Quit` immediately after setting `shutdown_initiated=true`, before the 2s wait.

## Scope of changes

- `crates/mika-cli/src/tui/app.rs` (+50/-0)
  - `send_message_with_thinking`: early-return guard when `worker_crashed=true`, push system `ChatMessage` pointing at `/restart`
  - `WorkerCrashed` tick handler: `tokio::spawn` fire-and-forget `log_audit_event("agent_worker_crashed", reason)` for session-scoped grep observability (complements the structured `agent_worker_silenced` log emitted by the supervisor)
- `crates/mika-cli/src/commands/chat.rs` (+17/-4)
  - Agent-switch path: send `AgentRequest::Quit` before the 2s drain await (lets old worker break out of its `while let`, run post-loop `mcp.shutdown().await` cleanly)
  - Reset `app.worker_crashed` and `app.pending_restart` to defaults on new-worker wire-up success (prevents crash-state leaking from agent A into agent B)
  - Warn log message extended with MCP-shutdown-skipped note
- `crates/mika-cli/src/tui/input.rs` (+49/-0)
  - `test_send_after_crash_surfaces_system_line_and_does_not_send`: guard fires, system line renders, no `AgentRequest` leaks onto dead channel
  - `test_send_while_healthy_dispatches_to_worker`: guard does not flip polarity

## Out of scope

- **F1** (agent-switch failure path leaves dead channel with no `/restart` affordance) — tracked in mika#1150 for separate dispatch
- **F4** (`/restart` + `/switch` double-dispatch in same tick → agent identity split) — tracked in mika#1150 for separate dispatch
- mika-platform-qa pipeline-check vs `scripts/verify-pipeline.sh` asymmetry — separate working-note ticket (independent fix surface)

## Acceptance criteria

- **AC1.** `send_message_with_thinking` early-returns when `worker_crashed=true`, pushes a system `ChatMessage` line pointing the operator at `/restart`. No `AgentRequest` is sent onto the dead channel.
- **AC2.** Tests cover guard polarity: `test_send_after_crash_surfaces_system_line_and_does_not_send` (refusal path with system line and no send) AND `test_send_while_healthy_dispatches_to_worker` (healthy path still dispatches).
- **AC3.** Agent-switch path sends `AgentRequest::Quit` to `app.agent_tx` immediately after setting `shutdown_initiated=true`, before the 2s drain await. Mirrors the `run()` cleanup ordering.
- **AC4.** `app.worker_crashed` and `app.pending_restart` reset to their healthy defaults when a new worker is wired up successfully (agent-switch success branch).
- **AC5.** `WorkerCrashed` tick handler spawns a fire-and-forget `log_audit_event("agent_worker_crashed", reason)` write on a fresh task, so the tick loop never awaits DB back-pressure on the input path.
- **AC6.** `cargo clippy -p mika-cli --all-targets -- -D warnings` clean. `cargo fmt --check` clean. All 298 tests pass (`cargo test -p mika-cli --bins`).
- **AC7.** ⏭️ **Deferred to operator runtime verification:** TTY smoke-test confirms agent-switch Quit-first behavior in an interactive session (Quit-induced exit lands inside the 2s window, MCP shutdown handshake completes, no zombie worker, no stdio leak). Code matches the prescribed F3 fix shape; unit tests cover the polarity at the channel level; TTY-level verification is an operator boundary not a CI boundary. Same `⏭️` deferred-marker pattern used by the qa-review plan-AC verdict work.

## Verification

PR #1185 reports 298 passing tests + clean clippy + clean fmt at commit `1d8fc4ee` (the F2 implementation commit). The Pipeline-Exempt trailer (`5e2b8feb`) and merge-from-main (`c225e081`) commits don't alter the source surface; they only address pipeline-check gating.

## Related

- mika#1149 — supervisor primitive (closed the panic-path silent-drop)
- mika#1150 — lifecycle hardening cohort filing (this plan covers F2 + partial F3; F1 + F4 remain open under #1150)
- mika#1181 — original salvage source (closed-as-duplicate of #1151)
- mika#1207 — milestone-close-claim guard misfire (why mika-arch couldn't groom this; peer-Claude was the architect rail)
- PR #1185 — implementation
- `crates/mika-cli/src/tui/app.rs:~904` — F2 guard site
- `crates/mika-cli/src/commands/chat.rs:~708` — F3 Quit ordering site
