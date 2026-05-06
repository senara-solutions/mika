---
ticket: mika#991
type: fix
title: Post-callback turn should advance queue autonomously, not narrate state and wait
date: 2026-05-06
seq: 008
---

# Plan: post-callback turn advances queue autonomously (mika#991)

## Verified state (post-architect-pass-1)

- **F1 (INTENT_GUARDS evaluation mode) addressed.** Pinned at `agent.rs:1283-1322`: linear iteration over the const array, per-guard `trigger(input) AND NOT satisfied(tools)` rejection check, single-retry tracking via `intent_guard_retries: HashSet<&'static str>` keyed by label. Guards compose **independently** — each guard fires on its own trigger/satisfaction predicate; multiple guards can fire on the same turn (each gets its own retry slot). The new `callback_milestone_advance` guard composes with existing `callback_terminal_action` (entry e) by adding a second independent constraint: a milestone-context callback turn must satisfy BOTH guards' predicates. Compositional satisfaction analysis below in Phase 1.
- **F2 (halt-token removed, structural-only) addressed.** The `[halt-reason: ...]` text-pattern detection in send_message has been removed. Per mika#862's structural-tool-call invariant (`docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`), text-pattern detection as a satisfaction path is exactly the failure class that compound warns against. The halt path is now **`update_task_status(parent, status='blocked', note=<reason>)`** — a tool call, not a text token. The reason carries in the `note` field. Phase 1's satisfaction predicate becomes two paths instead of three: (a) `run_claude_pilot` for next child (advance), or (b) `update_task_status(parent, blocked|completed)` (halt or finish).
- **F3 (Phase 2 rationale reframe) addressed.** The `PostCallbackAdvance` trigger is reframed from "give the LLM another chance" (which is the framing `feedback_prompt_enforcement_fragile.md` warns against) to "engine-side structural backstop that fires regardless of LLM behavior." Two-turn separation is justified on **engine-side guarantee** grounds, not LLM-cooperation grounds. The trigger does NOT depend on the prior turn's prompt drift; it fires unconditionally when the engine observes that no advance happened. The prompt is informed of the trigger via the `[advance: ...]` prefix but cannot suppress it.
- **F4 (heartbeat trigger location) addressed.** Pinned: heartbeat is `SilentTrigger::Heartbeat` enum variant in `crates/mika-agent/src/agent.rs:2839+`, fired from `crates/mika-agent/src/task_engine/dispatcher.rs:667`. The trigger produces `[heartbeat trigger]` user-message prefix at `agent.rs:2976`. **There is no separate heartbeat skill prompt** — heartbeat behavior is handled inline in `self-dev/system_prompt.md` via the always-on self-dev skill. The "doesn't resume milestones" gap from `project_heartbeat_milestone_phantom.md` is therefore a self-dev prompt-level gap. Phase 5's fix adds a heartbeat-trigger section to `self-dev/system_prompt.md` (specific insertion point pinned in Phase 5 below).

## Why

mika-dev's post-callback turn deliberates instead of advancing the queue. After three documented incidents on 2026-05-06 (closed-issue stall #985, milestone wedge cancellation #666, heartbeat-doesn't-resume gap), the chronic shape is clear: *the engine has a queue ready to advance, the LLM is the only thing standing between the queue and progress, and the LLM elects to deliberate instead of trust.* mika#988 fixed the closed-issue path by changing handler exit semantics so the callback delivers a clean `auto_skipped` result. mika#991 fixes the broader pattern: even with a clean callback, mika-dev posts confirmation questions and waits.

Per `feedback_prompt_enforcement_fragile.md`, prompts are not the right place for hard structural rules — LLMs rationalize crossing them. The existing post-callback prompt rules in `self-dev/system_prompt.md` lines 113–129 already say *"do NOT pick up unrelated issues, do NOT review the backlog, return to Step M4 to advance"* — and this still failed in practice. The fix must be structural at the engine level, with prompt rules serving as the documented contract rather than the enforcement surface.

The fix bar: after any clean terminal callback (success, structured `auto_skipped`, structured failure that has exhausted retries), the engine forces queue advancement before mika-dev's session can idle. The chronic-stall class is closed structurally, not by hoping the prompt holds.

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against worktree at HEAD `514c24d2` (origin/main post 2026-05-06 cascade merges + kg doc rebase).

### Existing post-callback infrastructure

**`mika/skills/bundled/self-dev/system_prompt.md`:**

- **Lines 101–129 — Callback Entry Point.** Three guard rules already live here in prose:
  - Line 113: "SCOPE RULE: Post-callback turns handle ONLY the task that triggered the callback."
  - Line 115: "MILESTONE/PROJECT CONTEXT: If the callback task's parent has `type='milestone'` or `type='project'`, you are in a milestone loop. After extracting metadata and closing out the child, return to Step M4."
  - Line 117: "MILESTONE/PROJECT CONTEXT CHECK (MANDATORY before processing the callback)."
  These are prompt-level rules. **They already exist; they already fail.** The 2026-05-06 wedge happened despite them. This is the empirical evidence backing `feedback_prompt_enforcement_fragile.md` for this surface.

- **Lines 620–669 — Step M4 (Serial execution loop).** Currently expects mika-dev to "Loop to next child" after callback handling. The looping is mika-dev's responsibility — engine does not enforce it.

**`mika/crates/mika-agent/src/agent.rs`:**

- **Line 4370–4378 — `callback_terminal_action` intent-precondition guard.** Existing guard fires on `[callback:` user messages, REQUIRES BOTH `update_task_status` AND `send_message` before EndTurn. **This composes with mika#991's fix** — the new guard adds a third requirement on top of the existing two.
- **Line 1557 — guard satisfaction check** for `callback_terminal_action` in the EndTurn chain. mika#991's new guard slots into the same registry pattern.
- **Lines 2845–3076 — `run_silent_agent` callback handling.** `SilentTrigger::Callback { label, .. }` produces the `[callback: {label}]` user-message prefix. The trigger is what fires the callback turn; advancing to the next task is currently mika-dev's responsibility, not the engine's.

**`mika/crates/mika-agent/src/task_engine/`:**

- **Milestone parent task spawns children sequentially via mika-dev's M4 loop.** The engine has no "milestone parent advancement" code path of its own — the parent task sits `in_progress` and mika-dev returns to it via M4 step 3 ("Loop to next child").
- **No SilentTrigger variant exists for "post-callback queue advance."** Adding one requires extending the `SilentTrigger` enum and the engine's callback delivery path.

### Existing post-callback siblings (also affected)

- **`mika/skills/bundled/self-dev-webhook-ci/system_prompt.md`** — CI webhook callback turn. Same chronic pattern: handles a CI failure callback, can deliberate vs. dispatch the iteration.
- **`mika/skills/bundled/self-dev-webhook-qa/system_prompt.md`** — QA webhook callback turn (verdict handling). Same pattern.
- **`mika/skills/bundled/qa-review-build-callback/system_prompt.md`** — QA build callback. Same pattern.
- **Heartbeat trigger** — recurring task that fires daily. Currently has the documented "doesn't resume milestones" gap (`project_heartbeat_milestone_phantom.md`).

### Engine guard registry shape

Per `crates/mika-agent/CLAUDE.md` § "Intent-precondition registry (#702)": `INTENT_GUARDS` is a const array of `IntentPrecondition` entries with `trigger`, `satisfied`, and `correction_message`. Adding mika#991's guard means appending an entry. This is the same pattern mika#988's (now-shipped) PR #993 used for handler exits, and the same pattern mika#996's plan uses for the `[output] required_suffix_lines` guard via the existing #864 machinery.

## Scope

**In scope:**

- **Phase 1 — Engine guard:** add `callback_milestone_advance` intent-precondition entry to `INTENT_GUARDS`. Triggers on callback turns whose parent task is a milestone or project. Requires that the agent EITHER (a) dispatch the next pending child via `run_claude_pilot`, (b) mark the milestone `blocked`/`completed` via `update_task_status`, or (c) explicitly halt with a structured `next_action: halt` reason in `send_message`. Rejects EndTurn otherwise.
- **Phase 2 — Engine post-callback advance trigger:** new `SilentTrigger::PostCallbackAdvance { parent_task_id }` fired by the engine immediately after a callback turn ends, IF the parent is a milestone/project AND the callback turn did NOT advance to the next child. This is the structural backstop — even if the prompt drifts and mika-dev idles, the engine fires a follow-up turn explicitly framed as "advance the milestone queue or mark blocked." Two-turn sequence (callback turn → advance turn) replaces single-turn deliberation.
- **Phase 3 — Prompt hardening (self-dev):** rewrite the Callback Entry Point section (lines 101–129) to explicitly forbid the deliberation pattern with citation to mika#991. Remove ambiguity about "summarize and ask" — the only permitted post-callback actions are metadata extraction, milestone-loop advance, and pipeline-failure retry.
- **Phase 4 — Sibling skill prompts:** mirror Phase 3's hardening in `self-dev-webhook-ci/system_prompt.md`, `self-dev-webhook-qa/system_prompt.md`, `qa-review-build-callback/system_prompt.md`. Each gets the same post-callback discipline citing mika#991.
- **Phase 5 — Heartbeat task fix:** harden the heartbeat skill prompt (or wherever heartbeat's silent-trigger handler lives in self-dev) to explicitly call M4 advance for any milestone with `in_progress` parent + callback-completed children.
- **Phase 6 — Tests:** integration tests at the eval-harness level for the three documented incident classes.
- **Phase 7 — Documentation + out-of-scope follow-ups.**

**Out of scope (explicitly):**

- **Full engine bypass of mika-dev for callback handling.** The ticket body proposes Option A as "engine schedules next pending task automatically without involving the LLM at all." This would require re-architecting where post-callback work happens (metadata extraction, cost tracking, deploy hook decisions, milestone summary) — that work currently lives in mika-dev's prompt and would need to move to engine code. **Too large for this PR.** Filed as Phase 7 follow-up if the hybrid proves insufficient under audit.
- **Changing the existing `callback_terminal_action` guard.** mika#991's new guard composes with it (adds a third requirement on milestone/project callbacks); does not replace.
- **Changing how callbacks are delivered to mika-dev's session.** The `[callback:` prefix and `SilentTrigger::Callback` variant are unchanged.
- **Auto-grooming on callback-trigger ungroomed tickets.** That's mika#996's territory; mika#991 only handles already-dispatched-and-completed callbacks.

**Position on Option A vs B vs Hybrid (defended):**

The ticket explicitly delegates this pick to the architect. Recommended pick: **Hybrid C — engine guard (structural enforcement) + post-callback advance trigger (structural backstop) + prompt hardening (documented contract).**

- **Pure Option A (engine bypass):** rejected for scope. Requires moving metadata extraction, deploy hooks, milestone-loop logic into engine Rust code. Substantial re-architecting; ROI unclear against the surgical hybrid.
- **Pure Option B (prompt-only):** rejected per `feedback_prompt_enforcement_fragile.md`. The existing prompt rules already say "advance, don't deliberate" and they already fail. Adding more prompt rules has diminishing returns.
- **Hybrid C:** the engine guard rejects the EndTurn when mika-dev tries to deliberate without advancing; the advance trigger fires a second turn if mika-dev still doesn't advance; the prompt rules document the contract. Three layers of defense, each surgical.

## Phase 1 — Engine guard `callback_milestone_advance`

**File touched:** `mika/crates/mika-agent/src/agent.rs`, `INTENT_GUARDS` const array (around lines 4307–4380).

**New `IntentPrecondition` entry shape (append to `INTENT_GUARDS`):**

```rust
IntentPrecondition {
    label: "callback_milestone_advance",
    trigger: callback_milestone_advance_trigger,
    satisfied: callback_milestone_advance_satisfied,
    correction_message: "[Your response was rejected. This is a callback turn for a \
         milestone/project child task. Per mika#991 you MUST either: (1) dispatch the \
         next pending child via run_claude_pilot, OR (2) mark the milestone/project \
         parent as `blocked` (with a reason in the note field) or `completed` via \
         update_task_status. Posting a confirmation question or summary without one \
         of these two tool calls is the deliberation-stall pattern documented in \
         mika#991. Re-read the callback result and either advance the queue or halt \
         the milestone explicitly via update_task_status.]",
},
```

**`callback_milestone_advance_trigger` predicate:**
- Triggers ONLY on `SilentTrigger::Callback` turns (matches existing `callback_terminal_action` trigger shape — parsed from `[callback:` prefix in user message).
- AND the callback's parent task has `type='milestone'` OR `type='project'` (looked up via `check_task` data available in the trigger context, OR via DB query if not).

**`callback_milestone_advance_satisfied` predicate (structural-only, F2 reframe):**

Returns true if any of the following appears in `all_tool_summaries`:

- **Path A (advance):** successful `run_claude_pilot` call (any skill — `dev-pilot`, `dev-groom`, `deploy_mika`). Indicates next child or deploy hook is dispatching.
- **Path B (halt or finish):** `update_task_status` call with `task_id` matching the parent milestone/project task AND `status` in `{"blocked", "completed"}`. The `note` field carries the halt reason or completion summary.

**No third path.** The earlier `[halt-reason: ...]` text-pattern detection on `send_message` content has been removed (architect F2). Per mika#862's structural-tool-call invariant from `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`: text-pattern detection as a satisfaction path drifts under LLM load and is exactly the failure class the compound warns against. The halt path is now a tool call (`update_task_status(parent, status='blocked', note=<reason>)`), not a text token.

**Compositional satisfaction with existing `callback_terminal_action` (F1 analysis):**

Per `agent.rs:1283-1322`, INTENT_GUARDS evaluates linearly with each guard firing independently on its own predicate. Multiple guards can fire on the same turn (each tracked separately in `intent_guard_retries`).

A milestone-context callback turn must satisfy BOTH:
- `callback_terminal_action`: requires `update_task_status` AND `send_message` (AND-shape, both must appear).
- `callback_milestone_advance`: requires Path A OR Path B (OR-shape).

**Reachable satisfaction sets:**

| Scenario | Tools called | `terminal_action` | `milestone_advance` |
|---|---|---|---|
| Advance to next child | `update_task_status(child, completed)` + `send_message` + `run_claude_pilot(skill=dev-pilot, prompt=<next>)` | ✓ (both required) | ✓ (Path A) |
| Halt milestone | `update_task_status(parent, blocked, note=<reason>)` + `send_message` | ✓ (both required) | ✓ (Path B) |
| Finish milestone | `update_task_status(parent, completed)` + `send_message` | ✓ (both required) | ✓ (Path B) |

The `update_task_status` of the existing guard can serve as the milestone status update of the new guard when it targets the parent (Path B). When it targets the child (advance scenario), the new guard's Path A satisfies via the `run_claude_pilot` for the next child. Either way, the satisfaction sets compose without bloat.

**Why guard, not just prompt:** the existing prompt rules at lines 113-129 already say to advance; they fail under load (verified empirically on 2026-05-06). The guard enforces structurally. mika#988 (now shipped via PR #993) used the same pattern for handler exits. mika#996 (groomed, ready) uses the same pattern for output contracts via `required_suffix_lines`. mika#991 extends the pattern to LLM-loop-cooperation invariants.

## Phase 2 — Post-callback advance trigger (`SilentTrigger::PostCallbackAdvance`)

**Files touched:**
- `mika/crates/mika-agent/src/silent.rs` (or wherever `SilentTrigger` is defined — verify at implementation; likely `crates/mika-agent/src/agent.rs` or `crates/mika-common/src/...`).
- `mika/crates/mika-agent/src/task_engine/` (or the callback delivery path — verify at implementation).

**New `SilentTrigger` variant:**

```rust
PostCallbackAdvance {
    parent_task_id: TaskId,
    parent_kind: ParentKind, // milestone | project
    last_child_outcome: ChildOutcome, // completed | failed | blocked | cancelled
}
```

**Trigger firing logic (engine):**
- After the callback turn ends successfully (mika-dev's `EndTurn` was accepted by the guard chain).
- The engine inspects: did the agent advance the milestone? (Same satisfaction check as Phase 1's guard, looking at the just-completed turn's tool calls.)
- If NO advance happened (despite the guard accepting the turn — possible if mika-dev satisfied via send_message halt-token or via metadata-only update_task_status), fire `PostCallbackAdvance`.
- If YES advance happened (run_claude_pilot or milestone-blocked/completed update), do nothing — the cycle continues normally.

**The PostCallbackAdvance turn:**
- mika-dev's session re-enters with the prefix `[advance: <repo> milestone#<n>] previous child <ref> outcome: <outcome>. Advance to next pending child or mark milestone halted with explicit reason.`
- The same `callback_milestone_advance` guard fires on this turn type, ensuring the advance happens.
- If mika-dev still doesn't advance (third deliberation attempt), the engine marks the milestone `blocked` itself with note "auto-blocked: mika-dev failed to advance after callback + advance turn (mika#991)" and notifies operator.

**Why two-turn separation (architect F3 reframe):** the trigger is an **engine-side structural backstop**, not a "give the LLM another chance" mechanism. Per `feedback_prompt_enforcement_fragile.md`, LLM-cooperation framings drift under load — that's exactly what mika#991 fixes for the first callback turn. The second turn is unconditional: the engine decides whether to fire it based on observed tool calls in the prior turn, not based on prompt-level signals. mika-dev's prompt cannot suppress the trigger.

The first callback turn legitimately needs metadata extraction, deploy-hook checks, child outcome interpretation. The second turn handles the orthogonal "advance the queue" obligation — making them separate phases of an engine-driven sequence, not one turn that the LLM can confuse. If mika-dev's first turn happened to advance (called `run_claude_pilot`), the engine observes this and skips the second turn. If it didn't advance (called only `update_task_status` for the child), the engine fires the second turn unconditionally. Either way, the queue advances structurally — engine-driven, not LLM-cooperation-driven.

**Cost concern:** PostCallbackAdvance fires every milestone-context callback. If the first turn DID advance, no second turn fires (engine-side check). Worst case: every other callback in a milestone fires a second turn at ~0.1-0.2K tokens per turn (tiny prompt — just the advance instruction). Cumulative cost is negligible vs. the chronic-stall cost.

## Phase 3 — Prompt hardening (`self-dev/system_prompt.md`)

**File touched:** `mika/skills/bundled/self-dev/system_prompt.md`, lines 101–129 (Callback Entry Point section).

**Change shape:** rewrite the section to explicitly forbid the deliberation pattern with citation to mika#991. Replace the existing prose with structured contract language.

**New text (replaces lines 101–129, exact line range verifies at implementation):**

```markdown
### Callback Entry Point (post background task)

**Engine contract (mika#991):** this turn is enforced by the `callback_milestone_advance` intent-precondition guard. You MUST advance the queue or halt explicitly. The deliberation pattern ("Task X done, want me to proceed?") is structurally rejected by the engine and will cause your `EndTurn` to be re-prompted.

**Permitted post-callback actions (exhaustive list):**
1. **Metadata extraction** — extract session_id, turns, cost, PR URL from the callback payload (per existing flow).
2. **Milestone/project advance** — for milestone-context callbacks, immediately call `run_claude_pilot` for the next pending child OR `update_task_status` to mark the parent `blocked`/`completed`.
3. **Pipeline-failure retry** — re-dispatch claude-pilot with the same task_id (existing retry semantics, capped per Rule 6).
4. **Explicit halt** — if and only if the callback indicates an unrecoverable blocker that requires operator decision (e.g., security review block, ambiguous AC), call `update_task_status(parent_task_id, status='blocked', note=<one-sentence reason>)` AND `send_message` to notify the operator. The `update_task_status(blocked)` tool call is the engine-recognized halt signal — the `note` field carries the reason for downstream parsing.

**Forbidden actions:**
- Confirmation questions to operator without a corresponding `update_task_status(blocked)` tool call. The engine rejects these structurally — `send_message` alone does not satisfy the milestone-advance guard.
- Reviewing the broader backlog (`list_tasks` for unrelated work). Out of scope per the SCOPE RULE.
- Picking up unrelated issues. Same.
- "Summary + wait" pattern. Same.

**MILESTONE/PROJECT CONTEXT CHECK (mandatory):** call `check_task(parent_task_id)` to confirm parent type. If `type='milestone'` or `type='project'`, this turn is engine-guarded and you must advance per action 2 above.
```

**Why this change vs. just keeping the existing prose:** the existing prose says "advance, don't deliberate" but in narrative form. The new shape names the four permitted actions, explicitly forbids the deliberation pattern, and cites the engine guard so future readers see the structural enforcement. Per `feedback_prompt_enforcement_fragile.md`, prompts can drift; the citation to mika#991 + engine guard makes drift visible to next-reader.

## Phase 4 — Sibling skill prompts

**Files touched:**
- `mika/skills/bundled/self-dev-webhook-ci/system_prompt.md`
- `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md`
- `mika/skills/bundled/qa-review-build-callback/system_prompt.md`

Each of these gets the same engine-contract callout + permitted/forbidden actions list adapted to its specific callback domain. Webhook handlers also get the cite to `webhook_no_unauthorized_dispatch` guard (mika#910) — sibling enforcement so the picture is consistent.

**Pre-flight verification (mandatory before Phase 4 edits):**

```bash
grep -l "post-callback\|callback turn\|after the callback" \
    mika/skills/bundled/self-dev-webhook-ci/system_prompt.md \
    mika/skills/bundled/self-dev-webhook-qa/system_prompt.md \
    mika/skills/bundled/qa-review-build-callback/system_prompt.md
```

If any of these files do NOT have a callback-turn section, the implementer adds one structured per Phase 3's shape. **Halt and surface to operator** if the surface area is wider than expected (e.g., undocumented sibling skills exist that also handle callbacks).

## Phase 5 — Heartbeat milestone-resume fix

**File touched (architect F4 — pinned):** `mika/skills/bundled/self-dev/system_prompt.md`. The heartbeat trigger has no dedicated section in self-dev today — the only existing references at lines 99 and 181 cite heartbeat as the owner of "sprint progress checks" and "delivery retry on failed sends," both passive observations rather than a heartbeat-handling block. Phase 5 ADDS a new `### Heartbeat Trigger` section to self-dev's system prompt, inserted directly after the existing `### Ready-Label Dispatch` section (lines 242-278 per the Phase 0 pin in mika#996's plan, post-mika#996 deployment) so the heartbeat handler sits alongside the other engine-trigger handlers as a peer.

**Change shape:** the new `### Heartbeat Trigger` section adds an explicit milestone-resume step to the heartbeat trigger flow. When heartbeat fires (user message prefix `[heartbeat trigger]`), before doing anything else, mika-dev queries `list_tasks(status="in_progress", type="milestone")` to find any in-flight milestones. For each, queries the most recent callback child's status. If the callback-completed child has not been advanced (i.e., milestone `in_progress` with completed child but no `in_progress` next child and pending children remain), heartbeat MUST advance via `run_claude_pilot` for the next pending child OR mark the milestone `blocked` if the situation requires operator decision.

**This is the heartbeat-doesn't-resume gap from `project_heartbeat_milestone_phantom.md`.** The plan acknowledges the prompt-only nature of this fix (no engine guard for heartbeat — out of scope here). The risk is the same drift class. Mitigation: heartbeat fires daily; even if it drifts, the `PostCallbackAdvance` from Phase 2 catches the per-callback path. Heartbeat only catches the older-than-one-day stalls.

**If Phase 5's heartbeat scope creeps:** halt and surface. The Phase 0 pin confirms heartbeat handling is self-dev-prompt-only (no separate skill, no engine-side heartbeat handler beyond the trigger fire); if implementation discovers a hidden surface (e.g., a sibling reflection skill that also needs the milestone-resume logic), file as a sub-PR rather than expanding this PR's scope.

## Phase 6 — Tests

**Test 1 (Phase 1 guard fires):** eval-harness scenario where mika-dev receives a milestone-context callback and ends the turn with only `update_task_status` (no advance, no halt-token). Assert: engine rejects EndTurn, correction message fires, mika-dev's retry advances.

**Test 2 (Phase 1 guard accepts structural halt):** mika-dev receives a milestone-context callback and ends with `update_task_status(parent, blocked, note="blocked by external dep")` + `send_message(...)`. Assert: engine accepts EndTurn (Path B satisfied), no retry.

**Test 3 (Phase 2 trigger fires):** mika-dev satisfies Phase 1 guard via metadata-only `update_task_status` (no actual milestone advance). Engine fires `PostCallbackAdvance`. Assert: second turn re-prompts mika-dev with advance instruction.

**Test 4 (Phase 2 trigger does NOT fire):** mika-dev advances via `run_claude_pilot` for next child. Assert: no second turn fires.

**Test 5 (Phase 4 sibling skills):** for each of self-dev-webhook-ci/qa, qa-review-build-callback, simulate the deliberation pattern. Assert: prompt-level rejection (or test-skip if those skills don't have engine-guard coverage in this PR).

**Test 6 (Phase 5 heartbeat):** simulate heartbeat fire with an in-progress milestone whose latest child is completed-but-not-advanced. Assert: heartbeat advances the milestone (calls `run_claude_pilot` for next child).

**Halt threshold (sibling of mika#988 Phase 3, mika#996 Phase 4):** if any test requires more than **80 lines** of harness setup beyond existing patterns, halt and surface. The eval harness should support these tests — guards are tested in the existing `tests/eval/grounding_regressions/` style.

## Phase 7 — Documentation + out-of-scope follow-ups

**Documentation:**
- `mika/CLAUDE.md` autonomous-loop section: one-paragraph note on the new guard + advance trigger.
- `mika/crates/mika-agent/CLAUDE.md` § "Intent-precondition registry (#702)": add `callback_milestone_advance` entry (f) to the list.
- `mika/docs/solutions/best-practices/callback-advance-2026-05-06.md` (new compound at PR-close): the principle: "When a queue is ready to advance and an LLM is the only thing in the way, the engine must enforce advance structurally — prompt-level rules drift."

**Follow-ups filed at PR-merge time:**
1. **Full engine bypass (Option A from ticket).** If the hybrid proves insufficient under post-PR audit (i.e., a fourth chronic-stall incident occurs despite the guard + advance trigger), file as a follow-up to migrate metadata extraction and milestone-loop logic into engine Rust code. Hard-line ticket: don't open speculatively.
2. **Halt-reason taxonomy.** Phase 1's halt path uses `update_task_status(blocked, note=<reason>)`. The `note` field is free-form prose. A future ticket could enforce a structured taxonomy (e.g., `blocked-external-dep`, `blocked-security`, `blocked-ambiguous-ac`) on the note via a `reason_class` field on the task or a structured-JSON note format, for dashboard analytics. Defer until operator workflow demands it.
3. **Companion mika-platform PR.** None needed — this fix is mika-internal.
4. **Apply the engine guard pattern to other "queue advance" surfaces.** E.g., team-engine's child callback returns. Audit at PR-merge time; file separately if surfaces exist.

## Acceptance criteria (from the ticket)

- [x] After ANY terminal callback (success, structured skip including `auto_skipped`, structured failure, or HANDLER CRASH), the next pending task in the agent's queue fires within ≤60s without operator intervention. **Phase 1 guard + Phase 2 trigger together** — guard rejects deliberation; trigger forces second-turn advance if the first slipped through.
- [x] mika-dev does not post a confirmation question to its session after a callback unless the callback explicitly requests operator input. **Phase 1 guard requires `run_claude_pilot` (advance) OR `update_task_status(parent, blocked|completed)` (halt or finish)** — `send_message` confirmation questions WITHOUT a parent-status update are rejected. The structural opt-out is the `update_task_status(blocked)` tool call, not a text token.
- [x] Test coverage at the team-engine integration level. **Phase 6 Tests 1–6.**
- [x] If Option A: a Rust unit test for the scheduler's post-callback advancement logic. **Phase 6 Test 3 (PostCallbackAdvance trigger).**
- [x] If Option B: transcript-replay tests for each affected skill prompt. **Phase 6 Test 5.**

## Risks and known unknowns

- **Risk: the `parent_task_id` lookup in Phase 1's trigger predicate adds a per-turn DB query.** Mitigation: the existing `callback_terminal_action` guard already does similar lookups; reuse the pattern. If lookup latency becomes a concern, cache the parent kind in the `SilentTrigger::Callback` envelope itself.
- **Resolved at plan time (was the halt-token risk):** removed per architect F2. The halt path is now structural (`update_task_status(parent, blocked, note=<reason>)`) — no text-pattern detection in the satisfaction predicate. Reason field is free-form prose but doesn't gate guard satisfaction.
- **Risk: `PostCallbackAdvance` trigger fires recursively.** If the second turn ALSO fails to advance, the engine fires a third? Mitigation: cap at one retry. If second turn also fails, engine marks milestone `blocked` itself with a structured note. The cap is engine-side; not mika-dev's responsibility.
- **Resolved at plan time (was the heartbeat-location unknown):** pinned per architect F4. Heartbeat is `SilentTrigger::Heartbeat` (engine variant), handled inline in self-dev/system_prompt.md. Phase 5 adds a new `### Heartbeat Trigger` section after `### Ready-Label Dispatch`. No separate skill needed.
- **Unknown: whether project-context callbacks (vs. milestone-context) need separate handling.** The ticket says both are affected. Phase 1's trigger handles BOTH (`type='milestone' OR type='project'`). Verify at implementation that Step P4 (project) loop has the same shape as M4 (milestone).
- **Unknown: interaction with mika#996's auto-groom flow.** When mika#996 ships, the auto-groom callback (returning `Verdict: GROOMED`) is itself a callback that needs to advance to dispatch. Phase 1's guard treats this correctly because the auto-groom callback's `parent_task_id` is the milestone child (not a separate groom task — per mika#996's M4 reuse pattern). The advance action in this case is `run_claude_pilot(skill="dev-pilot")`. Verified compatible at plan time; no special-case logic needed.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/callback-advance-2026-05-06.md`. Title: **"Engine-enforced queue advance: structural over prompt for chronic-stall classes."** Principle:

> When a workflow has a "must-advance" obligation that depends on LLM cooperation, and the LLM is observed to drift on it under load (= prompt-level rules failed twice or more in production), the fix is engine-level: an intent-precondition guard that rejects EndTurn unless the advance happened, plus a backstop trigger that fires a second turn if the first slipped through. Prompt-level rules then serve as the documented contract for next-readers, not the enforcement surface.

Contrapositive: mika#988's auto-skip uses the same pattern at a smaller scale (handler exit semantics, not LLM behavior). mika#996's required-suffix-line guard uses the same pattern for output contracts. mika#991 extends the pattern to LLM-loop-cooperation invariants.
