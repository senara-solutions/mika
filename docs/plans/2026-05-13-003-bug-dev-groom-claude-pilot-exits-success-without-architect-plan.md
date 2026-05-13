---
title: "dev-groom claude-pilot exits Success at ~12 turns without calling architect"
ticket: mika#1097
date: 2026-05-13
type: bug
component: claude-pilot, dev-groom, dispatch-lib
status: planned
---

# mika#1097 — dev-groom claude-pilot exits Success at ~12 turns without calling architect

## Problem statement

When the autonomous loop or `mika ask --agent mika-dev "groom <ref>"` dispatches the `dev-groom` skill via `run_claude_pilot`, the spawned claude-pilot child intermittently exits `status="success"` at ~12 turns / ~60 s / ~$0.40 **without calling `mika-arch`, without invoking `/ce:plan`, and without committing any plan file.** The dispatch-lib post-flight checks (HEAD-unchanged from mika#1033, plan-file ≥500 bytes) catch the empty artifact and rewrite the result to `PIPELINE FAILURE`, so the parent groom task ends `failed` with `callback_delivered_without_pr_url`.

This is the second leaf in the `dev-groom` early-exit family. The first leaf (mika#1033) addressed drift **INTO** unrelated work — the LLM treated imperative verbs in the ticket body as direct commands and ran them in executor mode, leaving an empty or stub plan behind. This leaf addresses drift **OUT of all work** — the LLM emits `stop_reason="end_turn"` after a small number of turns without producing any visible text, tool calls, or artifacts.

The 2026-05-13 morning mass-dispatch window produced eight grooming attempts in 3 minutes; **6 of 8 sessions exhibited this exact zero-artifact, zero-tool-call shape**, burning ~$2.80 for zero output. Single-dispatch grooming on the same morning (mika#886, prior-day mika#1088) worked. Queue depth is therefore a possible variable but unproven without LLM-side evidence.

## Why prior fixes do not cover this

- **mika#1033** (dev-groom drift detection) — fixed drift INTO executor mode; this is drift OUT of all work. The post-flight plan-validation block from mika#1033 *catches* the failure but does not prevent it; orchestrator-Claude still pays the per-session cost.
- **mika#1058** (callback-safe deferred dispatch) — fixed callback-context gating in mika-agent; this is downstream of dispatch, inside the spawned claude-pilot session.
- **mika#864** (required-suffix-line guard) — guards the mika-agent EndTurn boundary for mika-arch's outputs; the failing surface here is Claude Code's session inside claude-pilot, not a mika-agent turn, so the existing guard does not see this surface.
- **mika#940 family** (dev-groom Phase 5 narrative-exit) — closer family. mika#1033 was the family fix but missed this variant.

## Layer correction to the ticket's scope-proposal

The ticket's "Scope (proposed)" section says: *"the LLM driving these failures is kimi-k2.5, not sonnet"* on the strength of `project_skill_override_scope_gap`. **That claim is incorrect for this surface.** The scope-gap memory describes mika-dev's *own* LLM (kimi-k2.5 on autonomous-loop turns, where `skill_overrides.llm_provider/model` does not fire). But the failure shape documented here — `[prompt] /mika-groom-ticket mika#1057` followed by `[done] Success | 12 turns | $0.40 | 59s` — is a **claude-pilot child-process log**, written by the Python CLI in `claude-pilot-py/`. That child wraps `claude-agent-sdk`, which calls the Anthropic API with `MIKA_ANTHROPIC_API_KEY`. The LLM that "exited at 12 turns" is therefore Claude Code's model (Sonnet by SDK default), not mika-dev's autonomous-loop kimi.

Practical consequence: the model-swap direction (option 2 in the ticket) is still a live lever, but the lever is on **claude-pilot's Anthropic model selection**, not on mika-dev's base-model override. Phase 0 must confirm the model that actually ran inside the failing session (`[init] Session , model unknown` in the reference log is itself a missing-data signal — see Phase 0 deliverable D1).

## Phase 0 — Reproduce with full event-stream capture (REQUIRED)

The ticket is explicit: *"Without seeing what the LLM actually says in those 12 turns, root cause is invisible."* The current `--verbose --log-dir` plumbing writes only lifecycle markers (`[config]`, `[guardrails]`, `[init]`, `[prompt]`, `[tool:request]`, `[tool]`, `[done]`) to the file sink. The 441-byte reference log for mika#1057 contains NO `[tool:request]` lines between `[prompt]` and `[done]` — the 12 SDK "turns" produced zero permission callbacks AND zero text content blocks visible to `log_text`. That gap is the data we need to close before choosing a fix layer.

### D0 — Pick a verbosity-augmentation strategy

Three options for capturing the missing data; pick one in Phase 0 step 1.

**Option A: Extend claude-pilot's file-logger sink.** Add a `log_assistant_block(block)` helper that file-logs every content block on every `AssistantMessage` regardless of type — `text`, `thinking`, `tool_use`, `tool_result`. Today `_text_of` in `agent.py` returns `None` for non-text blocks, silently swallowing them. The patch goes in `agent.py` around line 104 and `ui.py` (a new `log_assistant_block` helper). Pro: persists evidence into the same log file the operator already grep's. Con: changes claude-pilot logging shape, needs a `--trace` flag to opt in so we don't double the on-disk volume for all sessions.

**Option B: Persist stderr from `dispatch-lib.sh` independently of success/crash path.** Today (line 396-401) stderr is appended to `RESULT` only as a 10 KB tail. If we redirect stderr to a per-task file (`/var/log/claude-pilot/<task_id>.stderr`) AND keep it on success path, the existing stderr-rendered tool/text events (which go through `write_log` → stderr+file) are preserved post-hoc. Pro: zero-change in claude-pilot. Con: assumes `write_log` actually emits the missing data to stderr — which the reference incident did not exhibit (stderr tail in the parent task's `result` field is empty for the 6 wasted sessions). Confirm before committing to this option.

**Option C: Enable SDK-level debug tracing.** `claude-agent-sdk` likely exposes a debug-trace hook (the SDK-internal `partial_message` stream + `system.subtype` events). Wire `--trace-sdk` in `cli.py` to a new sink that dumps every raw SDK event to JSONL. Pro: most-complete capture. Con: largest patch; deepest API coupling; depends on SDK version we have installed.

**Recommended starting point: Option A** — smallest patch, lives in code we own, preserves the operator workflow of `cat /var/log/claude-pilot/<task_id>.log`. Phase 0 step 1 lands the patch with a `--trace` opt-in flag.

### D1 — Why `[init] Session , model unknown`?

The reference log shows the init message with empty `session_id` and `model = "unknown"`. Inspection of `agent.py` lines 91-95 + `_extract_session_id` / `_extract_model` indicates the SystemMessage init event had no `session_id` attribute and no `model` attribute. That is itself a missing-data symptom — either the SDK changed its init-event shape or partial-message ordering is putting init before the session metadata is populated. Phase 0 must instrument this (log `repr(message)` for the init event under `--trace`) so we know whether the missing model is a logging bug or an SDK-event-ordering signal.

### D2 — Reproduction protocol

Once D0 lands and is deployed:

1. Pick one of the six unrecovered wasted tickets — recommend **mika#1057** because it's the canonical case in the issue body.
2. From the orchestrator-Claude session, **single-dispatch** (do NOT mass-dispatch — that defeats the queue-depth signal):
   ```
   mika ask --agent mika-dev "groom mika issue#1057"
   ```
3. Watch `/var/log/claude-pilot/<task_id>.log` while it runs.
4. After completion, capture:
   - Full log content
   - The `tool_calls` table rows for that session (`SELECT * FROM tool_calls WHERE session_id = ? ORDER BY id`)
   - The `llm_calls` rows for that session (model name, tokens, reasoning column)
   - The parent task's `metadata.claude_pilot.session_id`, `turns`, `cost_usd`, `duration_ms`

### D3 — Branch on the captured data

Three outcomes from Phase 0 reproduction:

**Outcome 1 — Reproduces with zero `[tool:request]` and only `thinking` blocks (or empty content).** The Anthropic model is producing extended-thinking turns and then exiting `end_turn` without any visible action. This is the "model gives up on the task" failure mode. Direction: fix lives in claude-pilot (Option A guard) or in the `/mika-groom-ticket` slash-command spec (move a mandatory `Bash` call to the first phase so the LLM cannot exit without trying).

**Outcome 2 — Reproduces with `[tool:request]` for tools that fail or get denied at Tier-1.** The model IS trying to act but each tool call is being silently rejected (e.g., `mika ask --agent mika-arch` rejected because mika-arch is missing, or `gh issue view` rejected). Direction: fix the tool-permission/tier-1 path, not the LLM.

**Outcome 3 — Does NOT reproduce; single-dispatch always works; only mass-dispatch fails.** The variable is queue depth, not Claude Code's behavior. Direction: rate-limit dev-groom dispatch concurrency at the orchestration layer (mika-dev's `validate_dispatch_readiness` already enforces per-class slot — extend or document).

Phase 0 deliverable: a written report at `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` that names which outcome applies, with citations to the captured log.

### D4 — Phase-0 acceptance signal

Phase 0 is complete when **one** of the three outcomes is named with evidence. We do NOT proceed to Phase 1 fix-layer selection before Phase 0 lands a report.

## Phase 1 — Fix-layer selection (gated on Phase 0)

Three independent fix layers, ranked by anticipated effectiveness. Pick after Phase 0 data lands.

### Layer A — Slash-command spec hardening (`/mika-groom-ticket`)

The slash command's existing structure has the architect call buried in Phase 3 (step 8). If the LLM voluntarily exits before reaching Phase 3, no architect call ever happens. Hardening shape:

- Move a tiny mandatory `Bash` step to Phase 1 (e.g., `gh issue view <n>` already lives there) and **require the assistant to verbalize the issue number before continuing** — this guarantees a tool call lands in turn 1 and the session cannot exit at zero-action.
- Reorder Phase 2 so the worktree is created (step 5 `git worktree add`) before any "decide whether to proceed" reasoning. Plan content can be drafted after the worktree exists.
- Add an explicit "if you exit before Phase 5, the parent task is marked failed and burns operator time — do not give up early" clause near the top of the spec. (Prompt-only and therefore fragile — see `feedback_prompt_enforcement_fragile`. Defense-in-depth, not the primary fix.)

Coupled change: the `dev-groom` skill prompt at `mika/skills/bundled/dev-groom/system_prompt.md` already references `/ce:plan` and `mika ask --agent mika-arch`. Keep those references aligned with whatever Layer A reorders.

### Layer B — Structural early-exit guard in `claude-pilot`

Analogous to mika#864's `required_suffix_lines` guard but inside the claude-pilot Python process — claude-pilot inspects the ResultMessage at end-of-session and rejects `status="success"` with `num_turns < N` AND zero `Bash`/`Edit`/`Write` tool calls observed across the session. On rejection, claude-pilot re-prompts the SDK with `"You exited without taking any action. The task requires you to invoke /ce:plan, create a worktree, and call mika ask --agent mika-arch. Continue."` Single retry.

Implementation surface:
- `agent.py` tracks tool-call count via `permission_handler` callback bookkeeping (counter incremented inside `create_permission_handler.handler`).
- On `ResultMessage` with `subtype == "success"`, check the counter. If `< N` (suggest N=3), re-prompt once via `client.query("...")` and continue the loop.
- One retry only; on second early-exit, emit a `status="terminated"` ResultJson with `subtype="early_exit_zero_action"` so dispatch-lib can record the structural reason in the parent task result.

Cost: ~30 lines in `agent.py` + `permissions.py` (counter wiring) + 1 test. Honest about scope: this is the smallest structural defense and it's reusable for the dev-pilot path too (see Layer A "open question 2" below).

### Layer C — Anthropic model selection on dev-groom dispatches

If Phase 0 shows the failing sessions ran on Sonnet 4.5 (or an older model), pin `MIKA_ANTHROPIC_MODEL` (or its claude-pilot equivalent) to Sonnet 4.6 for `dev-groom` dispatches. If Phase 0 shows Sonnet 4.6 also fails, swap to Opus 4.7 for `dev-groom` only — that surface is low-volume (≤5 dispatches/day in steady state) and the per-call cost premium is paid back the first time it avoids a $0.40 wasted session.

Implementation surface:
- `dispatch-lib.sh` already sets `--command` per skill via a case switch. Extend it to set `ANTHROPIC_MODEL` via env-var per skill (or via a new `--model` flag if claude-pilot supports one — check during implementation).

### Layer D — Mass-dispatch rate-limit (deferred)

Out-of-scope for this ticket per the issue body: *"Manual recovery of the 6 wasted tickets — those go through fresh single-dispatch once the fix lands."* If Phase 0 outcome 3 (queue-depth-only) lands, file a sibling ticket for the rate-limit work — do NOT bundle it here.

## Phase 2 — Implementation

Pick one or two layers from Phase 1 (A+B is the recommended bundle; B is the structural minimum). Implementation order:

1. **Layer B first** — narrowest blast radius, smallest patch, lives in code we own, regression-testable.
2. **Layer A second** — slash-command edit + dev-groom skill prompt edit. No code change.
3. **Layer C only if Phase 0 outcome 1 with model-specific signature** — and only after a measurement cycle from the post-Layer-B-deploy world.

## Tests

- **Layer B unit test (claude-pilot-py):** Mock a ResultMessage with `subtype="success"` and zero observed tool calls; assert the agent re-prompts; assert that a second early-exit emits `status="terminated"` with `subtype="early_exit_zero_action"`.
- **Layer A integration:** No code, no automated test. Manual reproduction protocol identical to Phase 0 D2.
- **End-to-end regression:** After Layer B lands and deploys, re-dispatch one of the previously-wasted tickets (e.g., mika#894). Expect success OR a structured `early_exit_zero_action` failure, NOT a silent `Success | 12 turns` with empty artifact.

## Acceptance criteria

- [ ] Phase 0 produces a written outcome report at `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` naming outcome 1, 2, or 3.
- [ ] One or more Phase 1 layers land per the outcome-to-layer mapping above.
- [ ] Layer B unit test (if Layer B lands) asserts the guard fires on simulated zero-tool-call sessions.
- [ ] After deploy, single-dispatch grooming of one previously-wasted ticket (mika#894 recommended) either succeeds with a plan-on-branch + GROOMED verdict OR fails with `subtype=early_exit_zero_action` — never the silent zero-artifact Success shape.
- [ ] Closing-ticket compound doc landed at `docs/solutions/best-practices/dev-groom-claude-pilot-early-exit-defense-2026-05-13.md` (or similar) once the fix is verified.

## Out of scope (verbatim from issue body)

- **Phase 5 writeback gap** on the 3 architect-was-called tickets (mika#1090, #899) — `callback_delivered_without_pr_url` after successful grooming. Separate ticket.
- **Manual recovery of the 6 wasted tickets** — fresh single-dispatch once the fix lands.
- **mika#1058's callback-context fix** — already in production.

## Resolved-during-drafting (not open questions)

- **`/mika-groom-ticket` slash-command expansion path.** Initial concern was whether `/mika-groom-ticket` being absent from `TIER1_SAFE_SKILLS` in `claude-pilot-py/src/claude_pilot/tier1.py` was part of the failure path. Resolved: the slash command is passed to the spawned Claude CLI subprocess as the leading user prompt (`claude-pilot --command /mika-groom-ticket -- "mika#1057"` → SDK's `query("/mika-groom-ticket mika#1057")` → the SDK uses `SubprocessCLITransport` which spawns the `claude` CLI; that CLI expands slash commands from `.claude/commands/*.md` natively). The `Skill` tool inside the session is unrelated to slash-command-as-prompt expansion. TIER1_SAFE_SKILLS is not on the failing path here.

## Open questions

- **OQ1 — `[init] Session , model unknown`.** Empty session_id + unknown model in the reference log. Is this an SDK init-event ordering issue, a logging bug, or a real session-state failure? Phase 0 D1 must answer.
- **OQ2 — Does this surface also affect dev-pilot dispatches?** dev-pilot is the implementation skill; its slash command (`/mika`) has its own phase structure. If Layer B is added to claude-pilot, it would also fire for dev-pilot — is that a wanted side effect, or do we need a per-skill threshold? Tentative answer: it IS wanted — early-exit zero-action is bad for dev-pilot too — but the value of N (turn threshold) may differ. Decide during Layer B implementation.
