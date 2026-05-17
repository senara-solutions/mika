---
ticket: mika#1168
title: "bug(mika-dev): dispatch-loss on 2026-05-17 — sonnet classifier-refusal + qa-review run_gh tool-shadowing co-causes"
type: bug
priority: p1-important
labels: [bug, p1-important, agent-core]
created: 2026-05-17
plan_seq: 2026-05-17-003
base_sha: 72021b78
supersedes_plan: 2026-05-17-002-bug-1168-mika-dev-sonnet-prompt-injection-refusal-plan.md
---

# Plan — mika#1168 — Close dispatch-loss-on-2026-05-17 by fixing both diagnosed co-causes

## Phase 0 Pin

**Base SHA:** `72021b78` — `chore(dev-groom): revert prompt-only design
— restore deterministic tool+handler (mika#1173) (#1187)` (origin/main
at plan time, re-pinned 2026-05-17 after pre-commit verification
caught a base shift from earlier b4a6c4fe pin).

All line numbers and verbatim slices below are anchored against this SHA.
Implementer MUST verify scope at checkout time by re-greping each anchor's
first-line literal; if the count diverges from the tables below, either a
new site exists (add it) or one site was refactored away (remove it). Do
not proceed until the grep count matches the tables or the divergence is
reconciled here.

### Surface table — co-cause 1 (classifier-refusal): 16 post-response correction sites

All 16 are user-role `LlmContent::Text` injections (verified injection
mechanism at the INTENT_GUARDS injection site and at the inline
`format!()` class within `async fn run_loop`). Storage shape varies —
inline `format!()` calls inside `async fn run_loop` (sites #1–#9),
`&'static str` literals on `IntentPrecondition` struct fields (sites
#10–#14), and `&'static str` top-level constants (sites #15–#16) —
but the injection mechanism and the sonnet-classifier exposure are
identical across all three storage shapes.

| # | Line | Scope / guard | First-line literal (greppable) | Class |
|---|------|---------------|--------------------------------|-------|
| 1 | 1060 | `async fn run_loop` — text-shaped-tool-call guard | `[Your response contained tool calls as text (e.g., <function=...>)` | non-dispatch |
| 2 | 1096 | `async fn run_loop` — prose-style-tool-call guard | `[Your response contained a prose-style tool call for` | non-dispatch |
| 3 | 1182 | `async fn run_loop` — Gate #3 required-tools | `[Your response was rejected because you did not call the` | **DISPATCH** |
| 4 | 1268 | `async fn run_loop` — completion-claim guard | `[Your response was rejected because you claimed completion` | non-dispatch |
| 5 | 1308 | `async fn run_loop` — milestone-close-claim guard (mika#797, new since b4a6c4fe) | `[Your response was rejected because you claimed a GitHub` (milestone was closed) | non-dispatch |
| 6 | 1375 | `async fn run_loop` — fabricated-action guard | `[Your response was rejected because you claimed to have` | non-dispatch |
| 7 | 1505 | `async fn run_loop` — asserted-unavailability guard | `[Your response was rejected because you claimed` | non-dispatch |
| 8 | 1616 | `async fn run_loop` — required-suffix-line guard | `[Your response must end with one of these literal lines` | non-dispatch |
| 9 | 1681 | `async fn run_loop` — required-finding-list guard | `[Your response was rejected because it does not contain` | non-dispatch |
| 10 | 4943 | `INTENT_GUARDS` — `webhook_ready_label_dispatch` precondition (mika#1173 text updated for `run_claude_pilot_groom`) | `[Your response was rejected. The` (`ready` label has been...) | **DISPATCH** |
| 11 | 4977 | `INTENT_GUARDS` — `webhook_no_unauthorized_dispatch` precondition | `[Your response was rejected. You called` | non-dispatch |
| 12 | 4989 | `INTENT_GUARDS` — `webhook_zero_tools` precondition | `[Your response was rejected because you received a GitHub` (webhook event...) | **webhook-broad** (covers dispatch + non-dispatch webhooks) |
| 13 | 5002 | `INTENT_GUARDS` — resume/continue trigger precondition | `[Your response was rejected because you received a resume/continue` | non-dispatch |
| 14 | 5030 | `INTENT_GUARDS` — `deferred_dispatch_action` precondition | `[Your response was rejected. This is a deferred-dispatch` | **DISPATCH** |
| 15 | 5042 | `CALLBACK_TERMINAL_ACTION_CORRECTION` top-level `&str` const | `[Your response was rejected because this callback turn ended` | non-dispatch |
| 16 | 5277 | `CALLBACK_MILESTONE_ADVANCE_CORRECTION` top-level `&str` const (#991) | `[Your response was rejected. This is a callback turn for a milestone/project` | non-dispatch |

**Greppability check at base SHA:**
```bash
grep -cF '[Your response' crates/mika-agent/src/agent.rs    # → 16
```

**Trigger-predicate classification (verified at base SHA):**

- **Strictly dispatch-critical (3 sites):** fire only on dispatch
  paths.
  - #3 (1182): required-tools gate; trigger = `MatchReason::Keyword`
    on a skill with `required_tools` (mika-dev's self-dev / dev-pilot
    declares `run_claude_pilot` as required → fires on every direct
    dispatch turn).
  - #10 (4943): `webhook_ready_label_dispatch`; trigger =
    `is_ready_label_dispatch_marker(msg)` — **fires only on the
    ready-label webhook event** that lands "[GitHub] Issue labeled
    ready on ...".
  - #14 (5030): `deferred_dispatch_action`; trigger = `SilentTrigger::
    DeferredDispatch` (mika#1011 retry after global_dispatch_active
    rejection).
- **Webhook-broad (1 site):** fires on any `[GitHub]` webhook event,
  including but not limited to dispatch.
  - #12 (4989): `webhook_zero_tools`; trigger = `|msg|
    msg.starts_with("[GitHub]")` — covers comment events, label
    events (including ready-label as one of many), PR review events,
    check-suite events, etc. Reshape MUST happen (the mandate
    phrasing fires the classifier on these turns too) but it is not
    exclusively a dispatch-path site.

**Commit-A1 scope (4 sites: #3, #10, #12, #14).** All four reshapes
ship together because they share the same Option A text shape and the
A2 partition is already conflict-prone (per Risks section). #12 lands
in A1 even though it's webhook-broad-not-strictly-dispatch — the
ready-label path is one of the paths it covers, and the reshape is
identical to the other three. The classification distinction matters
for AC #2 (see below), not for partitioning.

**Commit-A2 scope (12 sites: #1, #2, #4, #5, #6, #7, #8, #9, #11, #13,
#15, #16).** Plus the 2 sibling silent-trigger sites at 3273/3297 IF
the implementer's verification confirms user-role injection.

### Sibling surface — silent-trigger initial prompts (decision required, not deferred)

`agent.rs` has two additional user-role candidates that use mandate
phrasing but do NOT start with `[Your response` — they're the
*initial* prompts of silent-triggered turns (not post-response
corrections):

| Line | Trigger | First-line literal |
|------|---------|---------------------|
| 3273 | `SilentTrigger::PostCallbackAdvance` (mika#991 ENGINE-DRIVEN ADVANCE) | `You MUST either:` (preceded by ENGINE-DRIVEN ADVANCE prose) |
| 3297 | `SilentTrigger::DeferredDispatch` (mika#1011 RETRY prompt, mika#1173 text updated to reference "matching dispatch tool") | `You MUST call the matching dispatch tool. Do not call update_task_status,` |

**Scope disposition: SCOPE-IN to commit A2.** Rationale: both sites
contain literal "You MUST" mandate phrasing identical in structure to
the 15 post-response correction sites; the sonnet classifier-refusal
mechanism is phrasing-driven, not message-class-driven, so any
user-role injection containing the mandate signature is exposed.
Treating them as "verify-at-implementation" defers a structural
decision into implementation-time when the operator (and architect)
have less context. The pre-merge verification step is not whether to
include them but to confirm the injection role:

```bash
# Implementer-time verification — confirm injection role:
git -C mika grep -B 30 -n "ENGINE-DRIVEN ADVANCE\|DEFERRED-DISPATCH RETRY\|matching dispatch tool" \
  crates/mika-agent/src/agent.rs | grep -E "role:|LlmRole::"
```

Expected: both sites resolve to `LlmRole::User`. If `-B 30` returns
no `LlmRole::` match, widen to `-B 100` before concluding the role
assignment is at a different call site — the silent-trigger
turn-construction may route through a helper that resolves role
further away than the marker text. Only if `-B 100` still returns no
hit AND tracing the consuming callsite shows `LlmRole::System` or
`LlmRole::Assistant` does the implementer DOWNgrade these sites to
out-of-scope and note the divergence in the PR body. Default
assumption (in-scope) reflects the silent-trigger turn-construction
pattern, which constructs initial prompts as user-role messages to
drive the turn.

**Reshape target wording for sites 3205, 3226:** drop the "You MUST"
phrasing; reframe as state-machine description ("The engine requires
either ... or ..." / "The engine expects run_claude_pilot to be
re-invoked with the original arguments"). Same reshape principle as
the `[mika-engine]` prefix on the 15 post-response sites; the prefix
itself isn't load-bearing here because the silent-trigger prompts are
the FIRST message of the turn (no prior context to disambiguate
trusted vs. user injection) — so drop the mandate phrasing and rely
on imperative-without-mandate-verb framing.

Greppability check post-reshape:
```bash
grep -cE "You MUST (either|call (run_claude_pilot|the matching dispatch tool))" crates/mika-agent/src/agent.rs    # → 0 (was 2 at base SHA)
```

### Surface table — co-cause 2 (qa-review tool-shadowing): 3 primary surfaces

| Surface | Path | Role |
|---------|------|------|
| `tools.json` registration | `skills/bundled/qa-review/tools.json` (the `run_gh` entry, lines 22–42 at base SHA) | Per-skill exec handler registration that shadows the global builtin `run_gh` |
| Handler script | `skills/bundled/qa-review/handlers/run_gh.sh` (lines 34–56 at base SHA) | Bash allowlist enforcing `pr review` / `pr diff` / `pr list` / `issue view` only; emits the verbatim "not in qa-review allowlist" error at line 55 |
| Global builtin | `crates/mika-agent/src/skills/builtin_handlers.rs:1819` (`async fn run_gh`) and the `GH_ALLOWED_SUBCOMMANDS` constant at line 1619 (which DOES include `issue`) | What `run_gh` resolves to absent per-skill shadowing |

The skill executor's handler-resolution order (which determines that
qa-review's per-skill handler shadows the global builtin even when
qa-review is merely always-on rather than the active intent) is a fourth
surface; only edited if fix variant (b3) wins.

### Always-on shadower census (Step 5b pre-resolved at plan time)

At plan time, the only always-on bundled skill that registers an
exec handler shadowing a global builtin is **qa-review** (registers
`run_gh`). Scan command and verbatim output:

```bash
cd skills/bundled && for d in */; do
  s=${d%skill.toml}; toml="${d}skill.toml"; tj="${d}tools.json"
  if [ -f "$toml" ] && grep -qE 'always_on\s*=\s*true' "$toml" \
     && [ -f "$tj" ]; then
    grep -oE '"name":\s*"(run_gh|run_shell|gh_read|build_mika|create_task|update_task_status|send_message|run_claude_pilot|check_task|list_tasks|cancel_task|toggle_skill|web_fetch)"' "$tj" \
      | sort -u | sed "s|^|  ${s%/}: |"
  fi
done
```

→ output: `qa-review: "name": "run_gh"` (single row).

This refutes the "≥2 other always-on skills shadow accidentally" trigger
for variant (b3) at plan time. The implementer MUST re-run this scan at
checkout time and halt if the result diverges (a new always-on shadower
would invalidate the variant-choice decision rule below). With zero other
shadowers, (b3) becomes a discretionary structural cleanup, not a
gate-triggered escalation.

### Hypothesis carry — mika#1166 (dev-groom 3ms exit)

mika#1166 ("dev-groom skill exits in 3ms without invoking `/ce:plan`")
opened 2026-05-17T07:43Z. The ticket reports `[init] Session , model
unknown, task <id>` (empty session, no model logged) and a `3ms / 2
turns / $0.00 / 0s` run summary — dispatch never reached the LLM.
That's facially inconsistent with co-cause 1 (classifier-refusal
produces a non-empty response with model + tokens billed) and with
co-cause 2 (qa-review allowlist failure produces a `run_gh` error in
mika-dev's reply, not a 3ms-no-model exit). Different failure shape,
probably different root cause.

But: co-cause 2's diagnosis (per-skill exec handler shadowing) is the
same *class* of bug — wrong skill's tool context wins. If dev-groom's
3ms exit traces to an always-on skill shadowing a handler that
claude-pilot startup depends on, this plan's Phase B fix could close
mika#1166 as a side-effect. **Phase D Step 11 explicitly smokes
dev-groom dispatch alongside the two dispatch smokes** to test this.
Either outcome (recovers → fold mika#1166 closure into this PR; doesn't
recover → evidence mika#1166 is a separate mechanism) compounds.

## Problem statement — symptom

Two failure shapes observed on `mika ask --agent mika-dev "implement
mika issue#NNN"` on 2026-05-17:

- **Shape A (silent loss):** mika#1162 dispatch produced refusal-shaped
  reply, no `claude_pilot_tasks` row.
- **Shape B (wedged queue):** mika#1163 dispatch produced refusal-shaped
  reply, tracking task `aa718483` created in `pending`, follow-up turn
  fabricated a non-existent dependency on #1162.

Same symptom surfaced via the `ready` label path on mika#797 / #894 /
#850 (2026-05-17T09:04–09:16Z), each producing `run_gh` errors quoted
verbatim in operator correction comments on this ticket.

## Root cause — two diagnosed co-causes

The two co-causes are **independent mechanisms producing the same
symptom**. Neither is a sub-cause of the other; neither is a footnote to
the other. They surface via overlapping invocation paths:

| Path | Co-cause 1 (classifier-refusal) | Co-cause 2 (qa-review shadowing) |
|------|---------------------------------|----------------------------------|
| `mika ask` direct dispatch | Fires when engine correction is injected (site #3) | Does NOT fire (no ready-label removal step) |
| `ready` label-triggered | Fires on multiple sites along the dispatch chain (sites #10, #12, #14) | Fires on the dispatch-ack handler's first step (ready-label removal) |

The strict-scope plan that ESCALATEd (`2026-05-17-002-...-plan.md` on
commit `c8efd9ec`) addressed only co-cause 1 and miscounted its sites
as 9 (actual at b4a6c4fe: 15; actual at current base 72021b78: 16)
and dispatch-critical sites as 2 (actual: 4). Its AC #1 smoked the
`mika ask` path — which is the operator workaround that bypasses BOTH
co-causes — and would have closed #1168 while ready-label dispatch
remained silently broken. This plan supersedes it.

### Co-cause 1 — Sonnet's input classifier refuses engine correction messages

After the 2026-05-07 model swap (`project_mika_dev_model_switch`), mika-dev
base is `anthropic/claude-sonnet-4-6`. Agent-loop post-condition guards
inject user-role correction messages at the 16 sites in the co-cause-1
surface table. The Gate #3 text at `agent.rs:1182–...` is representative
of the inline `format!()` storage shape:

```rust
"[Your response was rejected because you did not call the required tool(s):
 {}. You MUST call these tools with real data before producing your
 response. Do not fabricate or assume results — call the tools now. ..."
```

The `IntentPrecondition`-stored shape at site #10 (4943,
`webhook_ready_label_dispatch`) is representative of the `&'static str`
struct-field storage:

```rust
correction_message: "[Your response was rejected. The `ready` label has been \
     removed but you did not call run_claude_pilot. The Ready-Label \
     Dispatch handler requires you to: \
     (1) run_gh ... If the marker is PRESENT: call create_task then \
     run_claude_pilot with skill=dev-pilot, prompt=\"<repo>#<n>\", and \
     task_id=<UUID>. ...",
```

All 16 sites use the same `LlmContent::Text` + `LlmRole::User` injection
mechanism (INTENT_GUARDS injection within agent.rs and inline run_loop
format!() calls). Sonnet's input classifier treats
user-role messages containing mandate-shaped phrasing ("You MUST call
tool X" / "you MUST: (1) ...") as adversarial — the canonical
prompt-injection signature — and replies with the literal text
`"Prompt injection. Rejected."`. The engine sees a no-tool-call
response and either gives up (Shape A) or wedges (Shape B). Verified:
the literal string appears nowhere in the mika codebase (`grep -rn`)
and is absent from historical DB rows; 22 assistant rows containing
it exist, all on 2026-05-17 from mika-dev. Kimi-k2.5 historically
silently complied with the same correction shape (see
`feedback_sonnet_over_kimi_for_grounding`); sonnet defends.

This co-cause is a pragmatic-mitigation target within the prompt-level
enforcement mechanism — per
`docs/solutions/best-practices/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
("handler-level tool allowlisting beats prompt-level prohibition"), the
structural direction is to move gate corrections to a tool-result
channel. That direction is deliberately out of scope here; this plan
reshapes the existing mechanism rather than replacing it.

### Co-cause 2 — qa-review's per-skill `run_gh` exec handler shadows the global builtin

`skills/bundled/qa-review/tools.json` registers a per-skill `run_gh` exec
handler (`handlers/run_gh.sh`) that takes precedence over the global
builtin `run_gh` at `crates/mika-agent/src/skills/builtin_handlers.rs:1819`.
The per-skill handler's allowlist (`run_gh.sh:34–56`) permits only:

```
pr review | pr diff | pr list | issue view
```

The dispatch-ack handler at
`skills/bundled/self-dev-webhook-ready-label/system_prompt.md:9` calls
`run_gh("issue edit <n> --remove-label ready")` as its first step on
every ready-label event. Because qa-review is `always_on = true`
(`skill.toml:5`), its per-skill handler is registered into the agent's
tool registry even when qa-review is not the active intent. The
ready-label removal call therefore hits the qa-review allowlist and
fails with the verbatim error at `run_gh.sh:55`:

```
ERROR: Command 'issue edit' not in qa-review allowlist.
run_gh is restricted to: pr review, pr diff, pr list, issue view
```

The atomic dispatch-ack transaction rolls back, no `claude_pilot_tasks`
row is created, dispatch is silently lost. Operator's correction comment
at 2026-05-17T09:16Z documents this as the second independent cause.

Note: the audit's "Ticket B" pointer at `skill.toml:13 (mis-scoped
required_tools)` is off-by-one-file. `skill.toml:13` declares
`required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]`,
which only governs skill-registry matching. The actual narrow allowlist
lives in `handlers/run_gh.sh:34–56`; the shadowing mechanism lives in
`tools.json`. The plan's fix surface follows the code, not the audit's
pointer.

## Steps

### Phase A — Co-cause 1 (classifier-refusal reshape)

1. **Live-LLM discovery harness (`#[ignore]`, manual gate).** Land as
   `crates/mika-agent/tests/sonnet_injection_classifier_repro.rs`. Calls
   `claude-sonnet-4-6` with three representative correction-text inputs
   (one per storage shape):
   - Site #3 (inline `format!()` in run_loop, line 1182): the Gate #3
     required-tools text verbatim.
   - Site #10 (`&'static str` in `IntentPrecondition`, line 4943):
     the `webhook_ready_label_dispatch` correction text verbatim
     (includes the mika#1173 `run_claude_pilot_groom` reference).
   - Site #15 (top-level `&'static str` const, line 5042):
     `CALLBACK_TERMINAL_ACTION_CORRECTION` text verbatim.

   For each input, run two passes: (a) original mandate-shaped text →
   assert response contains `"Prompt injection"` substring
   (case-insensitive); (b) proposed Option A reshape text
   (`[mika-engine]` prefix, mandate phrasing dropped) → assert response
   does NOT contain `"Prompt injection"` AND emits a tool-call attempt
   against a stub tool registered by the harness. Vary one sub-string
   per run on the original text ((i) drop `"You MUST"`, (ii) drop
   `"rejected"`, (iii) drop the leading `"["`) to isolate the trigger
   phrase.

   On Option A reshape failure for ANY of the three representative
   inputs, harness prints `HALT: option_a_insufficient: site=<N>` and
   exits non-zero. The implementer halts per the Decision rule below.

2. **Pick reshape strategy.** If Step 1 validates Option A across all
   three representative inputs, proceed. If not, HALT and escalate to
   Vincent — do NOT attempt Option B (synthetic tool-result channel)
   in-flight. Option B's three concrete unknowns that an investigation
   would have to close:
   - **Per-provider tool-result schema compatibility.** Anthropic,
     OpenAI/openrouter, deepseek, gemini each have different
     tool-result message shapes (Anthropic's `tool_use_id` /
     `is_error`; openrouter normalizes loosely; gemini uses
     `functionResponse`). A synthetic tool-result with no matching
     prior `tool_use` would need to be accepted by every active
     transport. Survey + per-provider conformance test would be the
     investigation gate.
   - **Orphan tool-result handling in the agent loop.** The agent loop
     at `crates/mika-agent/src/agent.rs` currently maps tool-results
     to outstanding `tool_use` blocks for state tracking
     (`pending_tool_calls`, callback correlation, etc.). Injecting an
     orphan tool-result (no matching tool_use_id) could corrupt that
     state. Investigation would enumerate the read sites and define
     the synthetic-id namespace that doesn't collide with real
     tool_use_ids.
   - **Assistant-role processing of unsolicited tool-results in
     subsequent turns.** When the LLM sees a tool-result it didn't
     request, behavior varies: some models silently incorporate, some
     emit confusion ("I don't recall calling that tool"), some
     classify as injection (the same failure mode this plan is
     fixing). Per-provider behavior survey would be a second
     investigation gate.

   Option B is sketched at root-cause-framing depth, not executable
   depth, because closing these three unknowns is itself the scope of a
   separate research ticket (file as follow-up if Step 1 disproves
   Option A). The structural follow-up direction is correct (per
   `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`);
   the urgency mismatch with a p1 dispatch wedge is what scopes it out
   of THIS plan.

   **Operator-facing budget note (this paragraph is addressed to
   Vincent, not to the implementer):** Step 1's live-LLM iteration is
   expected to cost on the order of ~$2–10 in API spend and ~2 hours
   of operator time. If iteration blows that envelope (still no
   trigger isolation after 2h), HALT-and-escalate is the path; do not
   silently spend further. The pipeline cannot enforce this — it's
   operator discipline.

3. **Reshape the FOUR dispatch-critical sites (commit A1).** Apply the
   chosen Option A text to:
   - Site #3 (line 1182, Gate #3 required-tools, inline `format!()`)
   - Site #10 (line 4943, `webhook_ready_label_dispatch`, `&'static str`)
   - Site #12 (line 4989, `webhook_zero_tools`, `&'static str`)
   - Site #14 (line 5030, `deferred_dispatch_action`, `&'static str`)

   Commit message: `fix(agent): reshape dispatch-critical correction
   messages to bypass sonnet injection classifier (mika#1168)`. These
   four sites unblock both dispatch paths' immediate failure modes.

4. **Reshape the remaining 12 non-dispatch sites (commit A2).** Apply
   the same Option A shape to sites #1, #2, #4, #5, #6, #7, #8, #9,
   #11, #13, #15, #16. Plus the 2 sibling silent-trigger sites at
   3273/3297 IF the implementer's verification (per Phase 0
   sibling-surface note) confirms they are user-role injections. Commit message:
   `fix(agent): reshape remaining gate correction messages for
   classifier consistency (mika#1168)`. No behavioral changes intended
   beyond the text shape.

### Phase B — Co-cause 2 (qa-review tool-shadowing scope)

5. **Step 5 investigation (run BEFORE picking variant).** Two sub-tasks:
   - **5a. Enumerate dispatch-ack and other always-on `run_gh` needs.**
     Run a deduplicating subcommand-pair extraction across all bundled
     skill prompts:
     ```bash
     grep -hoE 'run_gh[^\n]{0,80}' skills/bundled/*/system_prompt.md \
       | grep -oE 'run_gh[[:space:](]*[`"]?[a-z]+[[:space:]_-]+[a-z]+' \
       | sed -E 's/^run_gh[[:space:](]*[`"]?//' \
       | awk '{print $1 " " $2}' \
       | sort -u \
       | grep -vE '^(pr review|pr diff|pr list|issue view)$'
     ```
     The four already-allowed pairs are excluded; the remaining
     deduplicated lines are the **subcommand pairs the widening must
     cover**. The Step 6 decision rule's `≤2 / ≥3` threshold runs
     against this deduplicated count, NOT against raw occurrence count.
     The dispatch-ack handler at
     `skills/bundled/self-dev-webhook-ready-label/system_prompt.md:9`
     contributes `issue edit` — confirmed at plan time.
   - **5b. Re-run the always-on shadower census from Phase 0.** Plan
     time scan showed only qa-review shadows a builtin among always-on
     skills (verbatim command + output in Phase 0). If the census at
     checkout time diverges (new always-on skill registers a builtin),
     halt and reconcile — the variant-choice decision rule depends on
     the census staying at exactly-one shadower.

6. **Pick the fix variant** based on Step 5's findings. Three
   candidates listed in increasing structural reach:
   - **(b1) Widen qa-review's allowlist.** Edit `run_gh.sh:34–56` case
     statement to allow the subcommand pairs enumerated by 5a.
     Minimal change; leaves the shadowing mechanism intact.
   - **(b2) Remove qa-review's `run_gh` registration.** Delete the
     `run_gh` entry from `qa-review/tools.json`. Global builtin
     `run_gh` then handles all qa-review tool calls via the broader
     `GH_ALLOWED_SUBCOMMANDS` list. Re-evaluate whether the qa-review
     system prompt still needs the tight "restricted to: pr review,
     pr diff, pr list, issue view" framing — at prompt-level, not
     handler enforcement. Useful if 5a surfaces ≥3 subcommand pairs
     beyond the dispatch-ack needs (widening becomes its own
     mini-allowlist).
   - **(b3) Change executor handler-resolution.** Modify the skill
     executor so per-skill exec handlers apply only when the skill is
     the active intent (`MatchReason::Keyword` or equivalent), not
     when it's merely `always_on`. Structural cleanup; relevant if
     5b's census changes (a future always-on shadower regresses
     dispatch in the same way). At plan time, with only qa-review
     shadowing, (b3) is **discretionary**, not gate-triggered — pick
     it if Vincent wants to close the shadowing class structurally
     rather than instance-by-instance.

   Decision rule: default to (b1) if 5a yields ≤2 subcommand pairs;
   move to (b2) if 5a yields ≥3; consider (b3) only on explicit Vincent
   directive (no auto-escalation). Document chosen rationale inline in
   commit message.
7. **Apply chosen variant (commit B1).** Implement the picked variant
   from Step 5/6. Single commit.

### Phase C — Tests and telemetry (covers BOTH co-causes)

8. **Hermetic CI regression guard for co-cause 1.** Land as
   `crates/mika-agent/tests/correction_message_classifier_guard.rs`.
   The test exercises the gate-3 retry loop with a mock LLM transport
   that returns a tool-call response when called, and **captures the
   user-role correction message the agent loop injects on retry**. The
   assertion is on the captured text directly, NOT on a mock that
   pattern-matches request shape:

   ```text
   // Assertion shape (load-bearing — the test must fail if the agent
   // emits old mandate-shaped text):
   let injected = harness.captured_injected_corrections();
   assert!(injected.len() >= 1, "agent did not inject correction on no-tool-call response");
   for msg in &injected {
       assert!(msg.starts_with("[mika-engine]"),
               "correction missing trusted-marker prefix: {msg}");
       assert!(!msg.contains("You MUST call"),
               "correction still contains mandate phrasing: {msg}");
       assert!(!msg.contains("rejected because"),
               "correction still uses rejection framing: {msg}");
   }
   ```

   Driving the agent loop: configure the mock to return a no-tool-call
   text response on the first request, then any response on subsequent
   requests. Run a turn that should fire the required-tools gate (a
   keyword-matched skill turn). Assert the captured correction count
   ≥1 and each captured correction passes the substring properties
   above. This tests the AGENT's emitted text, not the mock's
   pattern-matching. Regression where someone partially reverts the
   reshape (e.g., reshapes 14 of 15 sites and forgets one) fails the
   test because at least one captured correction violates the
   substring properties.

   Runs on every CI build; no network, no API key.

9. **Hermetic CI regression guard for co-cause 2.** Land as
   `crates/mika-agent/tests/qa_review_run_gh_shadowing_guard.rs` (or
   extend an existing executor-test file). With qa-review's tools.json
   loaded and qa-review as `always_on`, invoke `run_gh issue edit ...
   --remove-label ready`. Assert the call succeeds (after the Phase B
   fix) and that the qa-review system-prompt restrictions still apply
   when qa-review IS the active intent (i.e., the fix narrows blast
   radius, doesn't blow open the qa-review surface entirely).
10. **Refusal-detection telemetry + retry-suppression.** In
    `crates/mika-agent/src/agent.rs` near the Gate #3 site: when the
    LLM response is non-empty text, contains zero tool calls, AND a
    correction was injected in the prior turn — log `warn!` with gate
    id and 200-char excerpt. Additionally: detect the substring
    `"Prompt injection"` heuristically; on detection, do NOT retry the
    same correction on the same turn (bounded-infinite-loop class) and
    emit `EngineError::ClassifierRefusal { gate, excerpt }` for
    operator surfacing.

### Phase D — End-to-end validation

11. **Smoke THREE dispatch paths before merging.**
    - **Direct-dispatch (co-cause 1 path):** `mika ask --agent mika-dev
      "implement mika issue#<throwaway-N>"` → assert a
      `claude_pilot_tasks` row reaches `running`.
    - **Ready-label-dispatch (co-cause 2 path):** file a throwaway
      ticket, add the `ready` label via `gh issue edit <N> --add-label
      ready`, observe the webhook trigger → assert a
      `claude_pilot_tasks` row reaches `running` AND the ready label is
      successfully removed from the issue.

      **Webhook-infra fallback (named, not invented at impl-time):**
      if dev environment cannot reach the production webhook receiver
      to fire the gateway → mika-dev path, substitute a server-fixture
      test: drive `crates/mika-agent/src/server/webhooks.rs`'s
      `issues.labeled` handler with a synthetic payload (`action:
      labeled`, `label.name: ready`, valid HMAC) and assert the same
      two post-conditions (task row reaches `running`, label removed).
      The fixture lives next to existing webhook handler tests; do not
      weaken the AC by skipping it.
    - **dev-groom hypothesis-test smoke:** `mika ask --agent mika-dev
      "groom mika issue#<another-throwaway-N>"` → observe behavior. If
      the dispatch succeeds and `/ce:plan` runs, that's evidence Phase
      B's fix closes mika#1166 as a side-effect → fold mika#1166's
      closure into this PR's commit message. If the dispatch still
      exits in 3ms with no model logged, that's evidence mika#1166 has
      a separate mechanism → leave mika#1166 open, do not retroactively
      claim closure.

    All three smokes (the third as evidence-collection, not as a
    blocker) must report results in the PR description. Direct-dispatch
    and ready-label smokes are the actual merge gates.

## Out of scope (deliberately)

- **Option B for co-cause 1 (synthetic tool-result channel).** Research
  problem; file as follow-up if Step 1 disproves Option A.
- **Rolling mika-dev back to kimi-k2.5.** Rejected: kimi fabricates
  dispatch references (`feedback_sonnet_over_kimi_for_grounding`).
- **Skill-level model override for dispatch turns.** Per
  `project_skill_override_scope_gap`, skill overrides do NOT fire on
  autonomous-loop turns by default; mika#1011 carve-out is the
  workaround, not a model-swap path.
- **General trusted-system-message plumbing redesign.** Follow-up if the
  audit (`mika-audit-mika-dev-investigate-e33bda22`) recommends it.
- **Auditing other always-on skills for accidental shadowing.** Phase 0
  census pre-resolves this; if 5b shows a new shadower at checkout,
  fold the audit in; otherwise file as a follow-up ticket.
- **Fixing mika#1166 (dev-groom 3ms exit) as a primary deliverable.**
  Phase D Step 11 smokes dev-groom to test the side-effect hypothesis
  but mika#1166 is NOT a blocker for this PR. If the smoke recovers
  dev-groom, fold the closure into commit message + PR body; if not,
  leave mika#1166 open.

## Acceptance criteria — symptom closure

Both dispatch paths must be green. AC #1 and AC #2 are **PEERS**, not
primary + footnote.

1. **Direct-dispatch smoke green.** `mika ask --agent mika-dev "implement
   mika issue#<N>"` produces a `claude_pilot_tasks` row in `pending`
   that transitions to `running` within normal dispatch latency. No
   `"Prompt injection. Rejected."` substring in the mika-dev reply.
2. **Ready-label-dispatch smoke green — BOTH co-cause fixes confirmed
   firing.** Adding the `ready` label to a throwaway issue via `gh
   issue edit <N> --add-label ready` triggers a webhook event whose
   handler:
   - (2a) successfully removes the `ready` label — **proves co-cause 2
     fix:** the dispatch-ack's `run_gh issue edit ... --remove-label
     ready` no longer hits the qa-review allowlist; no verbatim
     `"not in qa-review allowlist"` error in mika-dev's reply.
   - (2b) produces a `claude_pilot_tasks` row that transitions to
     `running` AND mika-dev's session log shows engine-injected
     correction messages (if any fired) using the `[mika-engine]`
     prefix — **proves co-cause 1 fix:** sites #10/#12/#14 reshape held;
     no `"Prompt injection. Rejected."` substring in the reply.

   Both sub-assertions must pass. The reason: ready-label dispatch
   threads through BOTH mechanisms (co-cause 2's `run_gh` allowlist
   check happens first at the dispatch-ack's label-removal step;
   co-cause 1's classifier fires later if the LLM ends a turn without
   the appropriate dispatch tool and the intent-guard correction at
   site #10 or #12 is injected). A smoke that only asserts "task row reaches
   `running`" passes if EITHER fix landed (the qa-review widening
   alone would let the label removal succeed, then if mika-dev's
   subsequent turn calls `run_claude_pilot` directly without
   re-prompting the gate, the row reaches `running` without ever
   triggering the reshape). Without the (2a)+(2b) split, the PR could
   ship with one co-cause silently still broken. (Webhook-infra
   fallback per Step 11 if dev environment lacks production webhook
   reach.)
3. Step 1's discovery harness (`sonnet_injection_classifier_repro.rs`)
   exits zero when run with `--ignored` against live sonnet (manual
   gate, not CI).
4. Step 8's classifier-refusal hermetic guard passes on CI, asserting
   on the AGENT's captured injected-correction text (not on mock
   pattern-matching).
5. Step 9's qa-review shadowing hermetic guard passes on CI.
6. Step 10's telemetry + retry-suppression is exercised by an explicit
   hermetic test injecting a refusal-shaped response.
7. `cargo test -p mika-agent` and `cargo clippy -p mika-agent` clean.
8. **Greppability hygiene check (PR-body-tracked, not hard gate):**
   `grep -cF '[mika-engine]' crates/mika-agent/src/agent.rs` returns
   N where N equals the count of reshaped sites listed in the PR body.
   If commit A1 ships alone (A2 deferred per the merge-race
   mitigation), N is 4 and the PR body MUST list the 12 deferred
   sites + filed follow-up ticket. If A2 also ships, N is 16 (or 18
   if the sibling silent-trigger sites are folded in). The hard gate
   is "every reshaped site uses the new shape"; the count is hygiene
   documentation, not a symptom-closure criterion.

## Risks

- **Classifier moves under us.** Step 10's heuristic detection +
  retry-suppression is the safety net; Step 1's harness is the
  early-warning instrument for model upgrades. Compound to
  `docs/solutions/best-practices/` post-merge.
- **Option A reshape still trips the classifier on one storage shape
  but not another.** Step 1's harness tests three representative inputs
  (one per storage shape); if any fail, HALT per Decision rule rather
  than silently shipping a partial reshape.
- **qa-review fix variant (b3) blast radius.** Changing executor
  handler-resolution affects every always-on skill that registers a
  per-skill handler. Phase 0 census shows only qa-review currently
  qualifies, so blast radius is minimal — but Step 5b re-runs the
  census at checkout time to catch any post-plan-time additions.
  Default lean stays (b1); (b3) is discretionary per Vincent.
- **Variant (b1) too narrow.** If Step 5a enumerates ≥3 subcommand
  pairs that always-on skill prompts expect from `run_gh`, switch to
  (b2) per the Step 6 decision rule. The threshold is set so a
  one-line widening doesn't grow into an ad-hoc allowlist.
- **Phase A and Phase B race on agent.rs / executor merges.** Both
  modify hot files (`agent.rs` is most-churned, executor is
  second-most). Rebase between Phase A commits and again before Phase B
  commit; if conflict in Phase A's commit A2 is non-trivial, ship A1
  + B1 (dispatch-critical only) and file A2 as follow-up. AC #8's
  soft greppability count accommodates this without being silently
  violated.
- **AC #2 smoke depends on webhook infrastructure.** If the production
  webhook path is unreachable in dev, Step 11's named server-fixture
  fallback (synthetic `issues.labeled` payload + HMAC, driven against
  the actual webhook handler) is the substitute. Implementer MUST NOT
  invent a weaker test; the fallback is pre-specified.
- **Sibling silent-trigger sites (3273, 3297) miscounted in scope.**
  Phase 0 flags them for implementer-time injection-role verification.
  If they're user-role, fold into commit A2; if they're system-role or
  assistant-role, they're out of scope and the plan stays at 16-site
  surface. Misjudging this would leave silent-trigger turns vulnerable
  to the same classifier refusal (low-frequency but real failure mode).

## Grounding evidence

- `crates/mika-agent/src/agent.rs` @ base SHA `72021b78` — sites #1–#16
  per co-cause-1 surface table; sibling sites 3273, 3297
- INTENT_GUARDS injection shape (`LlmContent::Text` + `LlmRole::User`)
  verified at the IntentPrecondition correction-message read site
- inline-format!() injection shape (same role + content type) at
  the run_loop post-condition guard sites
- `skills/bundled/qa-review/tools.json` — `run_gh` per-skill exec
  registration (the shadowing mechanism)
- `skills/bundled/qa-review/handlers/run_gh.sh:34–56` — narrow
  allowlist; line 55 emits the verbatim error mika-dev quoted
- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md:9` —
  the dispatch-ack handler's first call hitting the qa-review handler
- `crates/mika-agent/src/skills/builtin_handlers.rs:1619` —
  `GH_ALLOWED_SUBCOMMANDS` (global allowlist; DOES include `issue`)
- `crates/mika-agent/src/skills/builtin_handlers.rs:1819` — global
  builtin `run_gh` (what qa-review's per-skill handler shadows)
- `docs/solutions/best-practices/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
  — prompt-level vs structural enforcement principle (frames why
  Option A is a pragmatic mitigation, not a structural fix)
- DB query: `SELECT date(created_at), agent_id, role, COUNT(*) FROM
  messages WHERE content LIKE 'Prompt injection%' GROUP BY 1,2,3` →
  only 2026-05-17, only mika-dev assistant + mika-arch quoting
- Operator correction comment on this ticket at 2026-05-17T09:16Z —
  full verbatim of the qa-review error string + workaround

## Grooming history

- 2026-05-17 — Original strict-scope plan
  (`2026-05-17-002-...-plan.md`) drafted and committed at `860c1718`.
  Surface count claimed 9 sites; actual at base SHA is 15. Dispatch
  classification claimed 2 dispatch-critical; actual is 4. These count
  errors carried forward into the first iteration of this expanded
  plan and are corrected at the 2026-05-17 pass-1 friend-review entry
  below.
- 2026-05-17 — Pass 1 (mika-arch session `291a9d32`): Disposition
  ITERATE. Three BLOCKING (F1: Phase 0 Pin missing; F2: live/hermetic
  test conflated; F3: Option B unsketched). All addressed in commit
  `c8efd9ec`.
- 2026-05-17 — Pass 2 same session: Verdict ESCALATE. BLOCKING F1:
  qa-review allowlist co-cause documented in operator correction
  comment was absent from the plan's root-cause section. AC #1 in the
  strict-scope plan smoked only the workaround path.
- 2026-05-17 — Per the friend's peer-review citing the mika#874
  precedent ("ESCALATE is terminal, no third pass; the path forward is
  either ship-with-audit-trail or rewrite to a new artifact identity"),
  the strict-scope plan is treated as superseded. This expanded plan
  covers both co-causes as equal-weight peers, with peer AC structure
  and joint symptom-closure. Fresh `/mika-groom-ticket mika#1168` will
  run against THIS plan, not the superseded one.
- 2026-05-17 — Friend's pass-1 review of the initial expanded plan
  (verdict: STRONG, lean GROOMED with 3 must-fix + 2 flags +
  cosmetic). Applied as follows:
  - **M1 (site #7 scope identifier suspect)** — verified at base SHA.
    Site #7 IS in `INTENT_GUARDS` (correct). BUT the deeper
    verification revealed the surface table missed 6 sites (actual
    count: 15, not 9) and the dispatch-critical classification missed
    2 sites (#9 webhook_ready_label_dispatch and #11 webhook_zero_tools
    both fire on ready-label dispatch paths). Surface table fully
    rewritten; commit A1 scope expanded from 2 to 4 sites; co-cause-1
    root cause now acknowledges three distinct storage shapes (inline
    format!() / IntentPrecondition &'static str / top-level &str const)
    all sharing the same user-role injection mechanism. Sibling
    silent-trigger sites at 3205/3226 flagged for implementer-time
    injection-role verification.
  - **M2 (Step 5a grep under-specified)** — tightened to a
    subcommand-pair extraction with explicit dedup and exclusion of
    the four already-allowed pairs.
  - **M3 (Step 8 was testing the mock not the agent)** — restructured
    Step 8 to capture the AGENT's injected correction text and assert
    substring properties directly. Mock pattern-matching no longer
    load-bearing; regression on partial reshape (e.g., 14 of 15 sites)
    now fails the test.
  - **F1 (Step 1's 2-hour timebox pipeline-binding)** — moved to an
    operator-facing budget note inside Step 2's HALT-and-escalate
    rule, explicitly labelled "NOT a pipeline-enforced gate."
  - **F2 (AC #8 greppability gate conflicts with A2-deferred mitigation)** —
    softened AC #8 to hygiene-with-PR-body-tracked-count;
    symptom-closure ACs are #1 and #2 only.
  - **Cosmetic (rename body candidate labels)** — applied (a)/(b)/(c)
    → (b1)/(b2)/(b3) in the rename body to match the plan.
- 2026-05-17 — Pre-commit base-SHA rebase. After the second friend
  pass-1 returned GROOMED (base SHA `b4a6c4fe`), pre-commit
  verification caught that origin/main had advanced to `72021b78`
  via three intermediate commits (mika#797 milestone-close fix,
  mika#1175 task-engine class-aware backstop, mika#1173 dev-groom
  prompt-only revert). Re-grep at `72021b78` returned 16 sites (was
  15 at b4a6c4fe). The new 16th site at line 1308 is a
  milestone-close-claim guard added by mika#797, non-dispatch class,
  scope-in to commit A2. All other line numbers shifted; site #10
  (formerly #9) text now references `run_claude_pilot_groom`
  (mika#1173 added a new tool sibling to `run_claude_pilot` for
  grooming dispatch). Sibling silent-trigger sites moved from
  3205/3226 to 3273/3297; site 3297 text updated from "You MUST
  call run_claude_pilot" to "You MUST call the matching dispatch
  tool" (mika#1173 dispatch-tool abstraction). Plan's diagnoses
  unchanged — co-cause 1 (classifier-refusal on mandate-shaped
  user-role injections) and co-cause 2 (qa-review tool-shadowing
  of `run_gh issue edit`) both apply identically at the new SHA.
  base_sha frontmatter + Phase 0 base SHA line + all 16 surface
  table rows + trigger-predicate classification + commit A1/A2
  partitioning + sibling-surface line numbers + Step 1
  representative inputs + Step 3 A1 site list + Step 4 A2 site
  list + AC #8 greppability count + Risks sibling-site reference
  all updated to 72021b78 anchors. Pre-commit rebase is itself
  evidence that `feedback_grep_count_surface_verification.md`
  discipline catches base shifts before they corrupt the plan.
- 2026-05-17 — Concurrent verifications run while preparing the second
  friend pass-1 paste (anticipating his named checks):
  - **Trigger-predicate verification for sites #9 + #11.** Confirmed
    site #9 (`ready_label_dispatch_trigger`) fires only on
    ready-label webhook events. Confirmed site #11
    (`webhook_zero_tools`) fires on any `[GitHub]` webhook event
    (broader than dispatch). Classification table now distinguishes
    "strictly dispatch-critical" (3 sites: #3, #9, #13) from
    "webhook-broad" (1 site: #11). Commit-A1 scope unchanged at 4
    sites — partitioning isn't affected by the classification
    refinement.
  - **AC #2 disambiguation.** Split AC #2 into (2a) co-cause-2-fix-
    confirming sub-assertion (label removal succeeds, no
    `"not in qa-review allowlist"` error) and (2b)
    co-cause-1-fix-confirming sub-assertion (no
    `"Prompt injection. Rejected."` in reply, correction messages
    use `[mika-engine]` prefix). Both must pass; previous
    "task row reaches `running`" alone would have passed if
    only one co-cause fix landed.
  - **Sibling silent-trigger sites (3205, 3226) scope disposition.**
    Promoted from inline parenthetical to a proper sub-note. Default
    disposition: SCOPE-IN to commit A2, with implementer-time
    injection-role verification (specific grep command provided) to
    confirm user-role assumption. Reshape target wording specified.
- Process learning captured in
  `~/.claude/projects/-data-workspace-mika-platform/memory/feedback_read_issue_comments_during_grooming.md`
  — Phase 1 of `/mika-groom-ticket` must fetch issue comments, not
  just body; co-cause corrections live there.
