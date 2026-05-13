---
ticket: mika#933
type: fix
status: GROOMED (pending)
date: 2026-05-13
branch: fix/933/self-dev-webhook-fallthrough-does-not
related:
  - mika#583  # prior fallthrough incident (2026-04-15)
  - mika#841  # ready-label positive-consent contract
  - mika#847  # webhook_ready_label_dispatch guard
  - mika#910  # webhook_no_unauthorized_dispatch guard (post-hoc EndTurn)
  - mika#932  # the dispatched-without-ready-label incident
  - mika-platform#74  # companion: closing-comment template fix
---

# fix(self-dev): webhook fallthrough must reject dispatch at the tool boundary, not post-hoc

## Problem

`webhook_no_unauthorized_dispatch` (mika#910, agent.rs:4743) is an **intent-precondition guard** that fires at `EndTurn`. It rejects the assistant's response and re-prompts when `[GitHub]`-prefixed webhook turns that are NOT `[GitHub] Issue labeled ready on` successfully called `run_claude_pilot`. The rejection is post-hoc:

```text
[GitHub] webhook arrives → LLM responds with create_task + run_claude_pilot → 
  → executor spawns claude-pilot subprocess → LLM EndTurns → 
  → INTENT_GUARDS evaluates → #910 fires → correction sent for NEXT turn
```

At the point #910 fires, the dispatch has **already executed**: the callback task exists, the subprocess is running, claude-pilot is doing real work. The correction is too late — it teaches the model for the next turn but does not undo the side effects.

The live incident at mika#932 (2026-05-02T10:55:47) reproduces this gap. mika#910 shipped 2026-04-30 (commit `01f89474`) and was deployed when the incident occurred. The guard fired (recorded in tool-call traces) but did not prevent the dispatch.

The companion prompt-side rule at `skills/bundled/self-dev/system_prompt.md` lines 309–322 (Webhook Fallthrough section) and Rule 9 (lines 497–503) is prose-only — keyword-rich operator comments (e.g., `/mika-groom-ticket` Phase 5 step 20 closing text) defeat the prose rule by activating the model's dispatch heuristic before it reaches the SCOPE RULE.

## Goal

Convert the webhook → no-dispatch rule from a **post-hoc EndTurn correction** into a **tool-boundary block** that rejects `run_claude_pilot` synchronously, before the subprocess spawns or the callback task is created. The prompt-side rule becomes belt-and-suspenders (observable in tool-call logs) rather than load-bearing.

## Non-goals

- Changing mika#841's ready-label contract or its label-vs-event semantics.
- Touching `self-dev-webhook-qa` / `self-dev-webhook-ci` dispatch paths — those are keyword-matched handler skills with their own entry points and are out of scope.
- Operator-facing closing-comment template — that's mika-platform#74.
- Replacing the post-hoc #910 INTENT_GUARD — it stays as a redundant safety net. Removing it would weaken defense in depth without justification.

## Approach

Two complementary tactics, both shipped in this PR:

1. **Engine-side tool-boundary gate** (load-bearing). Add a webhook-source check to `validate_dispatch_readiness` in `crates/mika-agent/src/skills/executor.rs`. The check rejects with `error: "unauthorized_webhook_dispatch"` when the originating user message is in the **Webhook Fallthrough domain** (see Phase 0 — Gateway event-prefix surface, below) — i.e., `[GitHub]` issue/comment/unknown events that lack a dedicated keyword-matched handler skill. PR-event and check-suite prefixes (qa/ci skill territory) are explicitly allowlisted because those skills have their own legitimate `run_claude_pilot` dispatch flows. The check requires plumbing the originating user-message text into `LongRunningContext`.

2. **Prompt-side hardening** (belt-and-suspenders). Replace the prose-only SCOPE RULE in `skills/bundled/self-dev/system_prompt.md` Webhook Fallthrough section with a **structural pre-call check**: before any `run_claude_pilot` on a webhook-fallthrough turn (NOT qa/ci handler turns), the agent must call `run_gh issue view <n> --json labels --repo <repo>` and confirm `ready` is in the label list. Falsifiable in tool-call logs.

The engine-side gate is the durable structural fix. The prompt-side change closes the gap on any future event class the engine prefix-check might miss (e.g., a hypothetical new gateway-formatted prefix that isn't `[GitHub]`).

## Phase 0 — Pinned source

### `LongRunningContext` struct (current, verbatim)

`crates/mika-agent/src/skills/executor.rs:93-103`:

```rust
pub struct LongRunningContext {
    pub db: AsyncDatabase,
    pub agent_name: String,
    pub session_id: String,
    pub trace_id: String,
    /// Per-turn dispatch counter (#583). Only one long-running dispatch is
    /// permitted per agent turn. Atomic for interior mutability through `&self`.
    pub dispatch_count: AtomicU32,
}
```

`LongRunningContext` does **not** derive `Default`. Every construction site is an explicit struct literal. Adding `originating_message: Option<String>` is therefore a hard compile error at every existing construction site — the implementer must update each one. The struct has no `#[non_exhaustive]` attribute, so no special handling is needed.

### Construction sites — complete enumeration

| # | File:line | Context | New field value |
|---|-----------|---------|-----------------|
| 1 | `crates/mika-agent/src/agent.rs:2357` | Conversation mode (`run_loop` setup) | `Some(latest_user_text(&request.messages))` |
| 2 | `crates/mika-agent/src/agent.rs:3431` | Silent `SilentTrigger::DeferredDispatch` only | `None` (deferred-dispatch retry has no fresh user input) |
| 3 | `crates/mika-agent/src/skills/executor.rs:2392` | Test: `long-running spawn happy path` | `None` |
| 4 | `crates/mika-agent/src/skills/executor.rs:2447` | Test: `long-running spawn with input` | `None` |
| 5 | `crates/mika-agent/src/skills/executor.rs:2519` | Test: `long-running skipped when long_running=false` | `None` |
| 6 | `crates/mika-agent/src/skills/executor.rs:2558` | Test: `long-running estimated_duration` | `None` |
| 7 | `crates/mika-agent/src/skills/executor.rs:2733` | Test helper `make_lr_ctx(db)` (used by all `dispatch_guard_*` tests) | `None` (override in new tests via separate constructor; see Tests section) |

Seven sites total. Sites 1 and 2 are production paths; sites 3–7 are tests that don't exercise the new check. Adding `originating_message: None` to sites 3–7 is mechanical.

### `validate_dispatch_readiness` current check ordering

`crates/mika-agent/src/skills/executor.rs:775-956`:

1. Task existence check (lines 782–801, returns `task_not_found` on miss).
2. Task status `pending|in_progress` (lines 803–818, returns `task_not_dispatchable` otherwise).
3. No active callback children (lines 820–855, returns `task_active_dispatch` otherwise).
4. Per-class dispatch slot free (lines 857–904, returns `global_dispatch_active` otherwise).
5. GitHub blocked-by check (lines 906–953, returns `dispatch_blocked_by` otherwise).

### Gateway event-prefix surface (what `[GitHub]`-prefixed strings exist)

`crates/mika-gateway/src/github.rs::format_event_text` (lines 299–438) emits the following prefix shapes:

| # | Event type | Action | Prefix shape | Handler today | New gate behavior |
|---|------------|--------|--------------|---------------|-------------------|
| A | `issues` | `labeled` (label=ready) | `[GitHub] Issue labeled ready on <repo>#<n> — ...` | mika#841 ready-label dispatch | **Allow** — authorized dispatch path |
| B | `issues` | `labeled` (label=other) | `[GitHub] Issue labeled <name> on <repo>#<n> — ...` | Webhook Fallthrough | **Reject** |
| C | `issues` | `opened|closed|assigned|...` | `[GitHub] Issue <action>: <repo>#<n> — ...` | Webhook Fallthrough | **Reject** |
| D | `issue_comment` | `created` | `[GitHub] New comment on <repo>#<n> (...) by @...` | Webhook Fallthrough (the mika#932 incident class) | **Reject** |
| E | `pull_request` | `opened|closed|review_requested|...` | `[GitHub] PR <action>: <repo>#<n> — ... (branch: ...)` | self-dev-webhook-qa skill | **Allow** — skill owns dispatch |
| F | `pull_request_review` | `submitted` | `[GitHub] PR review (<state>) on <repo>#<n> (...)` | self-dev-webhook-qa skill | **Allow** — skill owns dispatch |
| G | `check_suite` | `completed` | `[GitHub] Check suite <conclusion> on <repo> (branch: ...)` | self-dev-webhook-ci skill | **Allow** — skill owns dispatch |
| H | Unknown event types | Any | `[GitHub] <event_type>.<action> on <repo>` | Webhook Fallthrough (no handler) | **Reject** (fail-closed for unknown surfaces) |

The rejection scope is exactly the Webhook Fallthrough domain — issue events (B, C), issue comments (D), and unknown event catchall (H). The qa/ci handler skills (E, F, G) and the authorized ready-label dispatch (A) are allowlisted.

This is a **broader contract** than just fixing the comment-body fallthrough — it codifies "only ready-label issue events and qa/ci handler paths may dispatch `run_claude_pilot` from a `[GitHub]` source." The ticket's "Expected Behavior" section states this contract explicitly ("ANY `[GitHub]` webhook event that does NOT activate a webhook-specific skill MUST result in ack-and-stop").

**Why allowlist by prefix, not by loaded-skill detection.** Loaded-skill detection would require plumbing the active skill set into `validate_dispatch_readiness`. The prefix surface is already the gateway's load-bearing format and is the canonical signal the qa/ci skills' keyword-matching is keyed off. Using the prefix preserves the single-source-of-truth invariant; loaded-skill detection would introduce a second authoritative source that could drift.

### Cross-ticket composition with mika#919

`mika#919` (branch `fix/919/self-dev-agent-operator-cli-dispatch`, GROOMED 2026-05-13) inserts a **grooming-marker check** into the same `validate_dispatch_readiness` function. Per its plan (`mika/docs/plans/2026-05-13-001-fix-dispatch-grooming-marker-engine-guard-plan.md` § Insertion point), the new check lands **between** the existing per-class dispatch slot check (current step 4) and the blocked-by check (current step 5). The two tickets are orthogonal — different rejection conditions, different error codes — but both touch the same function body.

**Combined check ordering after both ship (any merge order):**

```
1. unauthorized_webhook_dispatch     ← new (mika#933) — pure string check, no DB
2. task exists                        ← existing
3. task status pending|in_progress    ← existing
4. no active callback children        ← existing
5. per-class dispatch slot free       ← existing
6. grooming_marker_check              ← new (mika#919) — GitHub API call
7. blocked-by check                   ← existing
```

**Merge sequencing.** Neither ticket needs to revise its plan to accommodate the other. The merge-second implementer rebases against the merge-first ticket's structural changes:

- **If mika#919 lands first:** mika#933's insertion is purely additive at position 1 (before all other checks). No interaction with mika#919's check at position 6.
- **If mika#933 lands first:** mika#919's plan section "Insertion point" already names step 5 (per-class dispatch slot) as the predecessor for its new check, so its insertion logic is unchanged. mika#919's hoisting of `let github_ref = task.reference_url.as_deref().and_then(parse_github_ref)` above the blocked-by check is also unaffected — that lives below mika#933's position 1.

**Function signature compatibility.** mika#919 does not change the signature of `validate_dispatch_readiness`. mika#933 adds an `originating_message: Option<&str>` parameter. The merge-second implementer must thread that parameter through any new call paths mika#919 introduces (the plan reads suggest mika#919 does not add new call sites — it only inserts new checks inline). If mika#919 adds a helper that calls `validate_dispatch_readiness` directly, mika#933's parameter must be added to that helper too. Verify at rebase time.

## Design — engine-side gate

### Where the check lives

`crates/mika-agent/src/skills/executor.rs::validate_dispatch_readiness` (line 775) is the existing tool-boundary gate for `run_claude_pilot`. It runs **before** subprocess spawn (line 1210 in `execute_long_running`) and returns a structured JSON error that propagates as the tool's error output.

The new check is the **cheapest** of all dispatch-readiness checks (pure string-prefix match, no DB read, no API call). It runs **first**, before `validate_task` / status / blocked-by / global-dispatch-slot.

### Plumbing originating message into `LongRunningContext`

`LongRunningContext` (`crates/mika-agent/src/skills/executor.rs:93`) currently carries `db`, `agent_name`, `session_id`, `trace_id`, `dispatch_count`. Add one field:

```rust
pub struct LongRunningContext {
    // ... existing fields ...
    /// Originating user-message text for this turn, when available.
    ///
    /// Populated in conversation-mode turns (the actual user/webhook input).
    /// `None` for silent triggers (`SilentTrigger::DeferredDispatch`, callback
    /// continuation turns) where there is no fresh user input — those paths
    /// have already passed an upstream gate.
    pub originating_message: Option<String>,
}
```

Construction sites in `crates/mika-agent/src/agent.rs`:

- **Conversation mode (~line 2353)**: populate from the latest user-role message in `request.messages` using the same extraction logic that already produces `user_input_text` at line 841 (which today runs *inside* `run_loop`). We extract it once at the construction site and pass the same string into both `LongRunningContext` and (via the existing `user_input_text` definition inside `run_loop`) the INTENT_GUARDS path. To avoid duplication, hoist the extraction into a small helper `latest_user_text(&request.messages) -> String` in `agent.rs`.

- **Silent `DeferredDispatch` mode (~line 3431)**: keep `originating_message: None`. Deferred-dispatch retries are engine-initiated and have no `[GitHub]`-prefixed user turn. The agent.rs `deferred_dispatch_trigger` already gates them; the executor doesn't need a second check.

- **Test sites (`make_lr_ctx` at line 2733 and call sites in `agent.rs:2357` / `agent.rs:3431`)**: default to `None` in `make_lr_ctx`. Add a `with_originating_message` builder helper to keep test ergonomics light. Existing tests that don't care about the new field keep working unchanged.

### Predicate sharing between agent.rs and executor.rs

Both modules need:
- `READY_LABEL_DISPATCH_MARKER` (currently `agent.rs:4694`)
- A predicate that decides "is this a webhook-fallthrough dispatch turn?"

Refactor: extract a new module `crates/mika-agent/src/webhook_dispatch.rs` exposing:

```rust
/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`. The
/// authoritative format string lives at
/// `crates/mika-gateway/src/github.rs:331` (line 331 in `format_event_text`).
/// Drift between the two locations is a contract violation; the gateway
/// side is the producer, this side is the consumer. See mika#842, mika#910.
pub(crate) const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";

/// True when the message is a `[GitHub]` webhook event in the
/// **Webhook Fallthrough** domain — i.e., a turn that MUST NOT call
/// `run_claude_pilot`. The fallthrough domain is the complement of:
/// (a) the authorized ready-label dispatch marker, and (b) the qa/ci
/// handler-skill territory (PR events, check suites).
///
/// Allowlist rationale: the gateway emits `[GitHub] PR ...` and `[GitHub]
/// Check suite ...` prefixes specifically for events that `self-dev-webhook-qa`
/// and `self-dev-webhook-ci` activate on. Those skills own legitimate
/// `run_claude_pilot` dispatch flows (CI-fix iteration, QA hold retries) and
/// must not be blocked. The fallthrough rejection scope is exactly the
/// `[GitHub] Issue ...` / `[GitHub] New comment on ...` / unknown-catchall
/// surface where no handler skill activates — the same scope the self-dev
/// prompt's Webhook Fallthrough section governs.
///
/// Mutually exclusive with `is_ready_label_dispatch_marker` on the
/// `[GitHub]` domain (mika#910).
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}

pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

**Important:** `agent.rs::webhook_no_unauthorized_dispatch_trigger` (the INTENT_GUARD's trigger predicate) has **different semantics** than the new `is_unauthorized_webhook_dispatch` predicate — the former is intentionally over-broad (matches `[GitHub] PR review`, `[GitHub] Check suite`, etc., per its existing test matrix at `agent.rs:8459-8530`). It generates a confusing post-hoc correction for qa/ci skill flows today but does not break them (the dispatch already happened by EndTurn). Tightening that predicate to match the new tool-boundary predicate is **out of scope for this PR** — it is a separate latent bug class (false-positive corrections, no side effect) and conflating it here risks regressing #910's existing post-hoc safety net for the fallthrough domain.

Within this PR:
- The new module `webhook_dispatch.rs` exports `READY_LABEL_DISPATCH_MARKER` and the new tight predicates.
- `agent.rs` imports `READY_LABEL_DISPATCH_MARKER` from `webhook_dispatch.rs` (replaces the private `const` at line 4694).
- `agent.rs::ready_label_dispatch_trigger` delegates to `webhook_dispatch::is_ready_label_dispatch_marker` (same semantics).
- `agent.rs::webhook_no_unauthorized_dispatch_trigger` retains its **current** over-broad shape (`starts_with("[GitHub]") && !starts_with(READY_LABEL_DISPATCH_MARKER)`) — unchanged behavior. The shared constant is the only thing pulled.

Follow-up ticket (out of scope here, file before merging this PR): tighten `webhook_no_unauthorized_dispatch_trigger` to use the same allowlist-shape predicate so the post-hoc INTENT_GUARD stops emitting confusing corrections on legitimate qa/ci dispatches. Estimated 30-line change to agent.rs + test matrix update; sibling to this PR.

### The tool-boundary check

In `validate_dispatch_readiness`, BEFORE `validate_task`'s re-fetch:

```rust
if let Some(msg) = originating_message
    && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
{
    return Err(serde_json::json!({
        "error": "unauthorized_webhook_dispatch",
        "task_id": task_id,
        "reason": "This turn was initiated by a [GitHub] webhook event in the \
                   Webhook Fallthrough domain (issue events, comments, or \
                   unknown event types). Only `[GitHub] Issue labeled ready on` \
                   webhooks (authorized dispatch) and PR / Check-suite events \
                   handled by self-dev-webhook-qa / self-dev-webhook-ci skills \
                   may dispatch claude-pilot. All other webhook events must use \
                   Webhook Fallthrough: acknowledge without dispatching \
                   (mika#841 positive-consent contract, mika#933)."
    })
    .to_string());
}
```

`originating_message` threads in as a new parameter to `validate_dispatch_readiness` so the test sites can pass `None` and the production sites pass `ctx.originating_message.as_deref()`.

Function signature change:

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,  // NEW
) -> Result<String, String>
```

All three call sites updated:
- `execute_long_running` (line 1210) — passes `ctx.originating_message.as_deref()`.
- `test_dispatch_guard_*` (lines 3783, 3799, 3815) — pass `None` since they test unrelated guards.

### Order of checks in `validate_dispatch_readiness`

1. **unauthorized_webhook_dispatch** (new — cheapest, no DB)
2. task re-fetch + task_not_found
3. task_not_dispatchable (status check)
4. task_active_dispatch (child callback check)
5. global_dispatch_active (cross-task slot check)
6. dispatch_blocked_by (GitHub API call — most expensive)

This order is preserved because the new check is pure string match. The decision was deliberate during #910's design — see the comment in agent.rs:4730 — and we extend the same ordering rationale. See Phase 0 § Cross-ticket composition with mika#919 for the combined ordering when both PRs land.

### Error code naming

`unauthorized_webhook_dispatch` (not `webhook_no_unauthorized_dispatch`) — the executor-side errors use noun-phrase tense; the agent.rs guards use predicate-phrase tense. Matches the existing pattern (`task_not_dispatchable`, `global_dispatch_active`).

## Design — prompt-side hardening

### Section to edit

`skills/bundled/self-dev/system_prompt.md` — the **SCOPE RULE** callout in the Webhook Fallthrough section (currently lines 311–322) plus Rule 9 (lines 497–503). **Scope:** Webhook Fallthrough only — the prompt-side pre-dispatch label gate does NOT apply to qa skill (`self-dev-webhook-qa`) or ci skill (`self-dev-webhook-ci`) handler turns. Those skills activate via keyword on `[GitHub] PR ...` / `[GitHub] Check suite ...` prefixes and own their own dispatch flows (PR reviews dispatching CI-fix iterations, etc.). The prose change is confined to the fallthrough section so qa/ci skill prompts are unaffected.

### Expected frequency (cost justification — F8)

The `run_gh issue view --json labels` pre-call adds one GitHub API roundtrip per webhook-fallthrough turn. Webhook-fallthrough turns are rare relative to the gateway's dominant `[GitHub] Issue labeled ready on` and `[GitHub] PR ...` traffic: in current operation, issue-comment fallthrough events fire roughly when an operator posts a comment on an open issue (single-digits per day across the workspace, typically during grooming and post-merge close-out). The label query takes ~200ms over the gateway's GitHub token; the dispatch turn itself runs many seconds. The observability gain (tool-call log shows the agent's label check before deciding) outweighs the cost at this frequency. Document the expected cost in the new prompt callout so operators understand why the call is mandated.

### New prose

Replace the prose-only "Do NOT call run_claude_pilot" line with a **structural check** the model must execute:

```
> **SCOPE RULE (HARD GATE)** — This turn handles ONLY the webhook event. The
> permitted-actions list below is exhaustive; nothing else is allowed.
>
> **Pre-dispatch label gate** — If the message references an issue and you
> believe `run_claude_pilot` is appropriate, you MUST first run
> `run_gh("issue view <n> --json labels", repo="senara-solutions/<repo>")` and
> verify `ready` appears in the returned label list. If `ready` is absent,
> abort the dispatch and post `send_message` to Vincent describing the event.
> The engine enforces the same gate at the tool boundary
> (`unauthorized_webhook_dispatch`); the label query is your evidence that
> you considered the gate before acting.
>
> Permitted actions:
> 1. Acknowledge the event
> 2. If the event correlates to an existing active task ... [unchanged]
> 3. If the event requires Vincent's attention ... [unchanged]
> 4. Stop — do NOT proceed to the generic Workflow section above
```

Rule 9 (line 497) gets a single new sentence at the end:

```
**Engine backstop:** The engine rejects `run_claude_pilot` at the tool
boundary with `error: unauthorized_webhook_dispatch` when this rule is
violated (mika#933). Treat this as a hard contract, not a soft preference.
```

The "incident" footnote gets one new line appended:

```
**Incident:** mika#583 ... [unchanged]
mika#932 on 2026-05-02 — `issue_comment.created` webhook with dispatch-class
keywords in body bypassed the prose rule; mika#910 post-hoc EndTurn guard
fired but could not undo the dispatch.
```

## Tests

### Unit tests (executor.rs)

Append three tests to the existing `dispatch_guard` block in `crates/mika-agent/src/skills/executor.rs`:

1. `test_dispatch_guard_rejects_unauthorized_webhook` — `originating_message = Some("[GitHub] Comment on senara-solutions/mika#933: ...")`, expect `is_error: true` and parsed `error == "unauthorized_webhook_dispatch"`. Asserts the new check runs **before** any DB-level check by using a task that would otherwise pass.

2. `test_dispatch_guard_allows_ready_label_webhook` — `originating_message = Some("[GitHub] Issue labeled ready on senara-solutions/mika#933")`, expect dispatch proceeds (no `unauthorized_webhook_dispatch` error). Continues to subsequent checks normally.

3. `test_dispatch_guard_allows_no_originating_message` — `originating_message = None` (callback continuation / silent trigger), expect dispatch proceeds. Confirms the new guard is opt-in by context, not a blanket block.

Add a fourth helper test asserting the predicate directly:

4. `test_is_unauthorized_webhook_dispatch_predicate` in `crates/mika-agent/src/webhook_dispatch.rs` — exhaustive matrix mapped to the Phase 0 prefix surface table:
   - Row A — `"[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"` → **false** (authorized ready-label dispatch)
   - Row B — `"[GitHub] Issue labeled bug on senara-solutions/mika#999"` → **true** (fallthrough — non-ready label)
   - Row B — `"[GitHub] Issue labeled p1-important on senara-solutions/mika#999"` → **true**
   - Row C — `"[GitHub] Issue opened: senara-solutions/mika#100 — title"` → **true** (fallthrough — issue action)
   - Row C — `"[GitHub] Issue assigned: senara-solutions/mika#100 — title"` → **true**
   - Row D — `"[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko"` → **true** (the mika#932 incident class)
   - Row E — `"[GitHub] PR opened: senara-solutions/mika#1000 — title (branch: foo)"` → **false** (qa skill territory)
   - Row E — `"[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"` → **false**
   - Row F — `"[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer"` → **false** (qa skill territory)
   - Row F — `"[GitHub] PR review (changes_requested) on senara-solutions/mika#1000 ..."` → **false**
   - Row G — `"[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)"` → **false** (ci skill territory)
   - Row G — `"[GitHub] Check suite success on senara-solutions/mika (branch: main)"` → **false**
   - Row H — `"[GitHub] discussion.created on senara-solutions/mika"` → **true** (unknown event catchall — fail-closed)
   - Non-domain — `"[claude-pilot] callback ..."` → **false** (not a `[GitHub]` prefix)
   - Non-domain — `""` → **false**
   - Non-domain — `"Implement mika#933"` (direct `mika ask` prompt) → **false**

Each row of the table maps to at least one assertion. New event types added to the gateway's `format_event_text` MUST add a row to this matrix.

### Integration test (eval harness)

Add `crates/mika-agent/tests/eval/test_unauthorized_webhook_dispatch_tool_boundary.rs` modeled on the existing `test_webhook_no_unauthorized_dispatch_guard.rs`. The key difference: the existing eval covers the post-hoc INTENT_GUARD; this new eval drives the agent through a real `[GitHub] Comment ...` turn where the LLM is mocked to call `create_task` + `run_claude_pilot`, and asserts:

1. `run_claude_pilot`'s `ToolOutput.is_error` is `true`.
2. The error JSON parses with `error == "unauthorized_webhook_dispatch"`.
3. No callback task was created in the DB (the spawn never started).
4. `tool_calls` row exists with `success = 0` and the error message visible in the `output` column — matches the ticket's verification criterion ("the row exists with success=0 and an error message naming the missing-label gate").

### Regression coverage for ready-label happy path

Add a parallel positive test in the same eval file confirming that on `[GitHub] Issue labeled ready on senara-solutions/mika#933` the dispatch succeeds (no new error introduced for the canonical path).

### Existing eval `test_webhook_no_unauthorized_dispatch_guard.rs`

No changes — it tests the INTENT_GUARD's post-hoc behavior, which is unchanged. The new eval tests the tool-boundary block. Both are valid layers of defense.

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| False positive: legitimate non-`[GitHub]` webhook channels accidentally caught | The predicate matches `[GitHub]` exactly. `[claude-pilot]`, `[Telegram]`, `[Slack]` etc. are unaffected. |
| False positive: synthetic `[GitHub]`-prefixed test messages | The prefix is the gateway's load-bearing format; tests outside the eval harness construct their own `LongRunningContext` with `originating_message = None`. |
| Field rename breaks existing tests | The new field is `Option<String>` defaulting to `None`; all existing test contexts compile unchanged with the default. |
| Predicate drift between agent.rs and executor.rs | Single source of truth in `webhook_dispatch.rs`; both consumers go through it. |
| Performance: per-dispatch string-prefix check overhead | Two `starts_with` calls; negligible relative to even the cheapest existing check. |
| Engine backstop allows the prompt-side rule to atrophy | The prompt-side `run_gh issue view` step makes the agent's reasoning **observable in tool-call logs** even when the engine doesn't reject — operators can audit and tighten. |

## Acceptance criteria

- [ ] `crates/mika-agent/src/webhook_dispatch.rs` exists with `READY_LABEL_DISPATCH_MARKER`, `is_unauthorized_webhook_dispatch` (allowlist-shape predicate per Phase 0 § Gateway event-prefix surface), `is_ready_label_dispatch_marker`, and the full predicate-matrix unit test covering rows A–H plus non-domain cases.
- [ ] `agent.rs::READY_LABEL_DISPATCH_MARKER` is replaced by the import from `webhook_dispatch.rs`. `agent.rs::ready_label_dispatch_trigger` delegates to `webhook_dispatch::is_ready_label_dispatch_marker`. `agent.rs::webhook_no_unauthorized_dispatch_trigger` keeps its current over-broad shape unchanged (see Predicate sharing § Important).
- [ ] `LongRunningContext` gains `originating_message: Option<String>`. All seven construction sites updated per Phase 0 § Construction sites — site 1 populates from latest user-role message, sites 2–7 pass `None`.
- [ ] `validate_dispatch_readiness` accepts `originating_message: Option<&str>` as a new parameter and checks it first. New error `unauthorized_webhook_dispatch` returned with structured JSON per the Design section.
- [ ] Three new unit tests in executor.rs pass (`rejects_unauthorized_webhook`, `allows_ready_label_webhook`, `allows_no_originating_message`).
- [ ] New eval `test_unauthorized_webhook_dispatch_tool_boundary.rs` passes; asserts no callback task was created and `tool_calls` row has `success=0` with the error in `output`.
- [ ] `skills/bundled/self-dev/system_prompt.md` Webhook Fallthrough section has the SCOPE RULE replaced with the pre-dispatch label gate (fallthrough-scoped, NOT qa/ci skill turns); Rule 9 has the engine-backstop sentence and the mika#932 incident line.
- [ ] No change to `skills/bundled/self-dev-webhook-qa/` or `skills/bundled/self-dev-webhook-ci/` system prompts — the engine gate's allowlist preserves their dispatch flows.
- [ ] `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green.
- [ ] CHANGELOG.md entry under `## [Unreleased]` → `### Fixed`: "Webhook fallthrough enforced at the dispatch tool boundary; `run_claude_pilot` on `[GitHub]` fallthrough-domain turns (issue events, comments, unknown event types) is rejected with `unauthorized_webhook_dispatch` before the subprocess spawns. Ready-label dispatches and qa/ci handler-skill dispatches are explicitly allowlisted (mika#933)."

## Compound

After implementation, file a `docs/solutions/logic-errors/` entry documenting the **post-hoc vs tool-boundary** distinction in guard design. Key claim, **scoped to stateful side-effect tools** (mika-arch F10 caveat): when a guard's failure mode is "the side effect has already shipped by the time EndTurn evaluates," EndTurn intent-precondition guards are insufficient — the gate must move to the tool-call boundary inside the executor. mika#910 vs mika#933 is the citation pair. Pure-read tools (`search_memory`, `gh_read`, `list_tasks`) have no irreversible side effects and EndTurn guards remain sufficient for those — the rule does not generalize blindly. The transferable design rule is: **identify the tool's reversibility before choosing the guard layer**.

## Out of scope (explicit non-changes)

- `self-dev-webhook-qa`, `self-dev-webhook-ci` — these activate via keyword on PR-event / check-suite prefixes that are explicitly **allowlisted** by `is_unauthorized_webhook_dispatch` (Phase 0 surface rows E, F, G). Their dispatch flows pass through the new gate unaffected. No prompt changes for either skill.
- mika#841's contract surface, mika#847's positive case, mika#910's post-hoc safety net — all preserved. mika#910's `webhook_no_unauthorized_dispatch_trigger` retains its current over-broad shape; tightening it is a follow-up ticket (see Predicate sharing § Important).
- The mika-platform#74 closing-comment template fix — companion ticket, separate PR.
- Any change to the `mika ask --agent mika-dev "implement <ref>"` direct path — that arrives without a `[GitHub]` prefix and is not affected.
- mika#919's grooming-marker check — orthogonal engine guard, separate PR. The two are merge-order-independent; see Phase 0 § Cross-ticket composition for the combined ordering.
