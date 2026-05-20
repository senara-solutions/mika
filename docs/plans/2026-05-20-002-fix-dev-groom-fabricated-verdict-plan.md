---
issue: mika#1133
type: fix
component: mika-dev / dev-groom skill
created: 2026-05-20
branch: bug/1133/mika-dev-mika-ask-returns-verdict
---

# Fix: dev-groom returns "Verdict: GROOMED" without dispatching architect (mika#1133)

## Problem

When operator calls `mika ask --agent mika-dev "groom mika issue#N"` against a
ticket that has a plan committed but no architect-review record, mika-dev
responds with:

```
mika#N — groomed ✅, plan on `<branch>` @ `<sha>`. Awaiting `ready` label to dispatch.
Verdict: GROOMED
```

The `Verdict: GROOMED` line is fabricated — no `mika-arch` session was invoked,
no `run_claude_pilot_groom` tool was called, no body callout was written.

Observed 2026-05-15 on mika#920. Operator-visible failure mode: orchestrators
read "Verdict: GROOMED" as load-bearing, dispatch downstream, then hit the
engine-level `dispatch_no_grooming_marker` gate (#919) because the issue body
has no canonical callout. Wasted cycle.

## Root cause

`dev-groom` is a **dispatcher** skill — it forwards work to `claude-pilot`,
which produces the verdict via callback. But its `skill.toml` declares the
**producer** manifest shape:

```toml
# skills/bundled/dev-groom/skill.toml (current, broken)
[output]
required_suffix_lines = [
    "Verdict: GROOMED",
    "Verdict: ESCALATE",
]
# (no [constraints] required_tools)
```

This is the inverse of the canonical dispatcher pattern. Compare:

| Skill | Role | `required_tools` | `required_suffix_lines` |
|---|---|---|---|
| `self-dev` (always_on) | dispatcher | `["run_claude_pilot"]` | (none) |
| `dev-pilot` (keyword) | dispatcher | (none — relies on `self-dev` parent) | (none) |
| `mika-arch-groom-ticket` | **producer** | `["gh_read"]` | `["Disposition: READY/ITERATE/ESCALATE"]` |
| `mika-arch-second-review` | **producer** | `["gh_read"]` | `["Verdict: GROOMED/ESCALATE"]` |
| **`dev-groom` (current)** | dispatcher | **(none)** | **`["Verdict: GROOMED/ESCALATE"]`** ❌ |
| **`dev-groom` (fixed)** | dispatcher | **`["run_claude_pilot_groom"]`** | **(none)** ✅ |

Producer skills produce the Verdict — `required_suffix_lines` is correct there
because the LLM emits the verdict text on its own EndTurn. Dispatcher skills
return immediately with a task ID; the Verdict arrives later via callback.
Putting `required_suffix_lines` on a dispatcher creates fabrication pressure:
the LLM dispatches (or doesn't), emits a text response, the suffix-line guard
(`agent.rs:1631`, post-condition #8, mika#864) rejects EndTurn for missing
Verdict, the corrective re-prompt names the accept-set verbatim, and the LLM
rationalizes by appending `Verdict: GROOMED`.

The required-tools gate (#3, `agent.rs:~1100`) would have caught the missing
dispatch, but `dev-groom` doesn't declare it — so the LLM is free to respond
conversationally without calling `run_claude_pilot_groom`, leaving the suffix-line
guard as the only enforcement, which the LLM satisfies by fabrication.

## Architectural framing

The bug is single-skill: only `dev-groom` carries the broken manifest shape.
No parallel risk on `mika-qa` — its `qa-review` skill embeds the verdict trailer
inside the `gh pr review --body` argument via `required_tool_arg_suffixes`
(pre-spawn validation, mika#899), and its `required_tools = ["qa_pr_view", "run_gh", ...]`
forces a real GitHub API call. The verdict is produced server-side (by
GitHub on accepting the review), not by the mika-qa LLM. Different mechanism,
no fabrication surface.

The skill manifest pattern delineation — dispatcher vs producer — is
implicit in the codebase but undocumented. The post-fix `CLAUDE.md`
update names it.

## Phase 0 — Verbatim pins

**Base SHA:** `9c2d5635` (`main` HEAD at plan time; immediate parent of the
plan-init commit on this branch).

The fix touches four load-bearing sites in the engine + bundled-skill tree.
Implementer must verify these slices match the current source before applying
F1–F4; any drift since the base SHA requires re-pinning before editing.

### Pin 1 — `skills/bundled/dev-groom/skill.toml` (full file at base)

```toml
[skill]
name = "dev-groom"
description = "Two-pass grooming flow (operator or autonomous) — takes a ticket from open to GROOMED plan-on-branch via /ce:plan and mika-arch architect review"
version = "0.1.0"
always_on = false
timeout_secs = 600

[triggers]
keywords = [
    "groom",
    "groom ticket",
    "/mika-groom-ticket",
    "groom issue",
]

[output]
required_suffix_lines = [
    "Verdict: GROOMED",
    "Verdict: ESCALATE",
]
```

The bug surface is concentrated in lines 16-20: `[output] required_suffix_lines`
without a sibling `[constraints] required_tools`. F1 removes lines 16-20 and
adds the `[constraints]` block.

### Pin 2 — `skills/bundled/self-dev/skill.toml` lines 1-25 (mirror pattern)

```toml
[skill]
name = "self-dev"
description = "Orchestrator: delegates implementation work to Claude Code via claude-pilot"
version = "0.2.0"
always_on = true
timeout_secs = 30
max_prompt_size = 65536
dependencies = [
    "build-mika",
    "deploy-mika",
    "dev-pilot",
    "browser-control",
    "resolve-pr-conflicts",
]

[constraints]
required_tools = ["run_claude_pilot"]

[triggers]
keywords = [
    "add feature",
    "implement",
    "develop yourself",
    "build",
    ...
]
```

This is the canonical dispatcher manifest shape that F1 mirrors. Note: `self-dev`
declares `[constraints] required_tools` AND has no `[output]` block. F1 brings
`dev-groom` to the same shape (with `run_claude_pilot_groom` as the required tool
instead of `run_claude_pilot`).

### Pin 3 — `crates/mika-agent/src/agent.rs` lines 1402-1441 (#5 fabricated-action guard)

This is the **insertion landmark for F3** (position 5b, immediately after this
guard). Verbatim slice at base SHA:

```rust
                    // Fabricated action-claim guard: if the agent claims to have
                    // performed an action (posted, commented, etc.) with a GitHub URL
                    // but made zero tool calls in this turn, reject and re-prompt.
                    // This catches hallucinated tool results where the agent fabricates
                    // resource URLs without executing any tool. See #308.
                    if !skip_remaining_guards
                        && matches!(response.stop_reason, LlmStopReason::EndTurn)
                        && !fabricated_action_retry_done
                        && tools_called.is_empty()
                        && let Some((verb, url)) = detect_fabricated_action_claim(&text)
                    {
                        fabricated_action_retry_done = true;
                        warn!(
                            step,
                            verb,
                            url,
                            label = mode.label(),
                            "Fabricated action claim detected with zero tool calls — re-prompting"
                        );
                        request.messages.push(LlmMessage {
                            role: LlmRole::Assistant,
                            content: LlmContent::Blocks(
                                mika_common::llm::response_content_to_blocks(&response.content),
                            ),
                        });
                        request.messages.push(LlmMessage {
                            role: LlmRole::User,
                            content: LlmContent::Text(format!(
                                "[mika-engine] The previous response claimed to have \
                                 {verb} a resource ({url}) without calling any tool \
                                 in this turn. The engine expects actions to be \
                                 performed via tools (e.g., run_gh); URLs and action \
                                 results come from actual calls, not synthesis. \
                                 Calling the appropriate tool now performs the action, \
                                 or the response should state that the action cannot be \
                                 performed.",
                            )),
                        });
                        continue;
                    }

                    // Intent-precondition registry (#702): iterate INTENT_GUARDS
```

F3 inserts a new guard block AFTER line 1441 and BEFORE line 1443's `Intent-precondition
registry` comment. Following the existing shape pattern (gate condition, retry flag flip,
warn log, push assistant content, push corrective user message, `continue`).

### Pin 4 — `crates/mika-agent/src/agent.rs` lines 1631-1689 (#864 suffix-line guard, for chain-position context)

This is the existing #864 suffix-line guard. F3 does **not** go here (per
architect F2 finding — fabrication guards belong in the fabrication cluster
at positions 4b/5, not the manifest-driven output-validation cluster at
positions 8/9). Pinned to confirm chain ordering and to anchor the rationale
for moving F3 upstream:

```rust
                    // #864 — Required-suffix-line guard. Skills can declare an exhaustive
                    // accept-set for their final line; missing match rejects EndTurn once.
                    // Position: END of the chain — other guards' rejections take precedence
                    // so a turn rejected for a more fundamental reason doesn't waste a
                    // suffix-line check.
                    if !skip_remaining_guards
                        && matches!(response.stop_reason, LlmStopReason::EndTurn)
                        && !required_suffix_line_retry_done
                        && !required_suffix_lines.is_empty()
                    {
                        let last_3_non_empty: Vec<&str> = text
                            .lines()
                            ...
```

Once F1 removes `dev-groom`'s `[output]` block, `required_suffix_lines` is empty
for dev-groom keyword-match turns, and this guard becomes inert for the dev-groom
case. Other producer skills (`mika-arch-groom-ticket`, `mika-arch-second-review`,
`mika-arch-groom-milestone`) still drive this guard correctly — F1's manifest
change is single-skill scoped.

### Pin 5 — `skills/bundled/dev-groom/system_prompt.md` lines 15-21 (F2 insertion target)

```markdown
### Important
- **Always pass `skill: "dev-groom"`** — required by the schema for engine dispatch-class derivation
- **Always pass the task UUID as `task_id`** (36-char format) when a task exists. Do NOT pass issue references — pass the UUID returned by `create_task`. This ensures logs land at `/var/log/claude-pilot/{uuid}.log`
- **Do NOT do the work inline** — never read source files, analyze the ticket, or write a plan yourself. The grooming workflow runs in the inner session via `/mika-groom-ticket`. Always use `run_claude_pilot_groom`
- Do NOT call `run_claude_pilot_groom` again for the same task while one is already running
- On `Verdict: GROOMED`, the issue body now carries `Branch:` + `Plan:` + `Grooming history:` callouts — the ticket is ready for dispatch via `run_claude_pilot` (dev-pilot)
- On `Verdict: ESCALATE`, surface the architect's reasoning to the operator and halt — do not retry without operator instruction
```

F2 inserts ONE new bullet between bullets 3 and 4 (after the "Do NOT do the work
inline" bullet), and qualifies bullets 5-6 with "On `Verdict: GROOMED` **callback**"
and "On `Verdict: ESCALATE` **callback**". The exact insertion text is in F2 below.

### Pin 6 — `tests/eval/grounding_regressions/` cluster reference

Confirmed from `crates/mika-agent/CLAUDE.md` § "Evaluation — Grounding Regressions
(#741, #862, #863, #864, #890, #894, #901, #1059)":

> 31 fabrication-detection scenarios. ... scenarios 22-29 from the required-finding-list
> conditional-disclosure-evasion guard (#901).

F4's two new scenarios (`dev_groom_fabricated_verdict_caught` and
`dev_groom_dispatched_no_verdict`) belong in this cluster at positions 30-31.
The dispatcher-manifest parity test (per architect F3 finding) does NOT belong
in this cluster — it goes in `crates/mika-agent/src/skills/manifest.rs` next to
`test_builtin_tool_names_parity` (the existing manifest-invariant test).

## Fix

Five files; smallest-surface fix that mirrors the canonical dispatcher pattern.

### F1 — Restore dispatcher manifest shape (the root-cause fix)

**File:** `skills/bundled/dev-groom/skill.toml`

Remove the `[output]` block and add `[constraints]` to force dispatch.

```toml
[skill]
name = "dev-groom"
description = "Two-pass grooming flow (operator or autonomous) — takes a ticket from open to GROOMED plan-on-branch via /ce:plan and mika-arch architect review"
version = "0.2.0"   # bump from 0.1.0 — manifest contract change
always_on = false
timeout_secs = 600

[triggers]
keywords = [
    "groom",
    "groom ticket",
    "/mika-groom-ticket",
    "groom issue",
]

[constraints]
required_tools = ["run_claude_pilot_groom"]

# [output] block intentionally removed — dev-groom is a dispatcher.
# The verdict is produced by the inner Claude Code session and arrives
# via callback. mika-dev's conversation-mode response should not emit
# Verdict: GROOMED — the structural guard in F3 enforces this.
```

Why this works:
- `required_tools = ["run_claude_pilot_groom"]` makes gate #3
  (`agent.rs:~1100`, mika#463/#516) reject any EndTurn that didn't attempt
  the dispatch. Mirrors `self-dev/skill.toml:14-15` exactly.
- Removing `required_suffix_lines` eliminates the fabrication pressure from
  gate #8. The LLM's EndTurn after a successful dispatch is `"Dispatched
  grooming for mika#N. Awaiting architect verdict via callback."` — clean,
  no verdict, no rejection.
- `MatchReason::Keyword`-only collection (mika#463) means the constraint
  only fires when the operator's message actually contains a `groom` keyword;
  it's inert otherwise.

### F2 — Update the system prompt to forbid Verdict emission

**File:** `skills/bundled/dev-groom/system_prompt.md`

Add an explicit prohibition at the top of the `### Important` block:

```markdown
### Important
- **Always pass `skill: "dev-groom"`** — required by the schema for engine dispatch-class derivation
- **Always pass the task UUID as `task_id`** (36-char format) when a task exists. Do NOT pass issue references — pass the UUID returned by `create_task`. This ensures logs land at `/var/log/claude-pilot/{uuid}.log`
- **Do NOT do the work inline** — never read source files, analyze the ticket, or write a plan yourself. The grooming workflow runs in the inner session via `/mika-groom-ticket`. Always use `run_claude_pilot_groom`
- **Do NOT emit `Verdict: GROOMED` or `Verdict: ESCALATE` in your dispatch response.** The verdict is produced by the inner Claude Code session and arrives via callback from claude-pilot — not from your turn. Your dispatch response should be: `"Dispatched grooming for <ref>. Awaiting architect verdict via callback (task: <task_id>)."` The engine rejects fabricated Verdict lines via the dev-groom fabrication guard (mika#1133).
- Do NOT call `run_claude_pilot_groom` again for the same task while one is already running
- On `Verdict: GROOMED` callback, the issue body now carries `Branch:` + `Plan:` + `Grooming history:` callouts — the ticket is ready for dispatch via `run_claude_pilot` (dev-pilot)
- On `Verdict: ESCALATE` callback, surface the architect's reasoning to the operator and halt — do not retry without operator instruction
```

The change is one prohibition bullet plus the qualifier "on … callback" on the
two trailing bullets. Per `feedback_prompt_enforcement_fragile.md`, prompt-level
"do not" instructions are weak — F3's structural guard is the binding
enforcement. The prompt change is for self-explanation (LLM knows *why*) and
operator-readable documentation, not for primary enforcement.

### F3 — Structural fabrication guard (defense-in-depth, AC2)

**File:** `crates/mika-agent/src/agent.rs` (new post-condition guard at position 5b — after #5 fabricated-action guard, before #6 intent-precondition registry; insertion point at ~line 1442)

After F1, the LLM has no `required_suffix_lines` pressure to emit Verdict. But
it might still emit one by training pattern. AC2 asks the engine to verify
groundedness. F3 is a **claim-legitimacy guard** (semantic sibling of #4b
milestone-close-claim and #5 fabricated-action-claim), not a manifest-driven
output-shape validator (#864 / #901). It therefore belongs in the
fabrication-guard cluster at the top of the chain, NOT in the
output-validation tail. Insert immediately after #5 (`agent.rs:1441`,
`continue` of fabricated-action guard) and before #6 (`agent.rs:1443`,
Intent-precondition registry comment) — position 5b.

This placement is the architect's directed correction (first-pass finding F2):
positioning F3 with the output-shape validators would misclassify it for
future chain readers. The guard is mechanically equivalent at either position
because F1 makes #864 inert for dev-groom turns, but semantic placement matters
for cluster integrity.

```rust
// #1133 — dev-groom fabrication guard. Detects "Verdict: GROOMED" /
// "Verdict: ESCALATE" in conversation-mode mika-dev text without a
// satisfying run_claude_pilot_groom tool call in the turn. dev-groom
// is a dispatcher — verdicts arrive via callback, not from this turn.
//
// Gating:
//   - Conversation mode only (`!mode.is_silent()`). Callback turns
//     legitimately carry Verdict lines from the inner session and
//     must pass through unaffected.
//   - EndTurn only (don't fire mid-tool).
//   - Single-retry (mirror of #864 retry pattern).
if !skip_remaining_guards
    && !mode.is_silent()
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !dev_groom_fabrication_retry_done
{
    let claims_verdict = text.lines().any(|line| {
        let t = line.trim();
        t == "Verdict: GROOMED" || t == "Verdict: ESCALATE"
    });
    let dispatched = all_tool_summaries
        .iter()
        .any(|s| s.tool_name == "run_claude_pilot_groom" && s.success);
    if claims_verdict && !dispatched {
        dev_groom_fabrication_retry_done = true;
        warn!(
            step,
            label = mode.label(),
            "dev-groom fabrication guard: response claims Verdict \
             without a successful run_claude_pilot_groom call — \
             re-prompting (#1133)"
        );
        request.messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Blocks(
                mika_common::llm::response_content_to_blocks(&response.content),
            ),
        });
        request.messages.push(LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(
                "[mika-engine] Your response contains `Verdict: GROOMED` \
                 or `Verdict: ESCALATE` but you did not call \
                 `run_claude_pilot_groom` in this turn. The dev-groom \
                 skill is a dispatcher — verdicts arrive via callback \
                 from claude-pilot, never from your turn.\n\n\
                 If grooming dispatch is genuinely needed: call \
                 `run_claude_pilot_groom` now and re-emit a dispatch \
                 acknowledgement (no Verdict line).\n\
                 If grooming dispatch is not needed (e.g., ticket is \
                 already groomed and you're just answering a status \
                 question): re-emit your response with the Verdict \
                 line removed."
                .to_string(),
            ),
        });
        continue;
    }
}
```

Add the retry flag declaration where other guard flags are declared (search
for `required_suffix_line_retry_done` and add `dev_groom_fabrication_retry_done`
adjacent):

```rust
let mut dev_groom_fabrication_retry_done = false;
```

Position rationale: position 5b (after #5 fabricated-action, before #6
intent-precondition registry). F3 is a claim-legitimacy guard, same family
as #4b and #5 — it checks whether the response's grooming claim is grounded
in a tool call this turn, not whether the response satisfies a manifest
output-shape contract. The fabrication cluster (positions 4b–5) is where
"the response claims X without doing X" guards live; F3 fits exactly that
shape (response claims `Verdict: GROOMED` without calling
`run_claude_pilot_groom`).

Mode gating (`!mode.is_silent()`) is critical: callback turns deliver Verdict
text from the inner session as their primary payload. Firing the guard there
would block legitimate callback delivery.

Composition with downstream guards: with F1 applied, `dev-groom` contributes
no entries to `required_suffix_lines`, so #864 is inert for dev-groom keyword
turns. Producer skills (`mika-arch-*`) still drive #864 correctly — F3 and
#864 do not conflict because the trigger sets are disjoint (F3 fires on
"claims Verdict without tool call"; #864 fires on "missing required suffix
when tool was called and a producer skill matched"). Both can in principle
fire on the same turn, but in practice the manifest separation prevents it.

### F4 — Regression tests (AC3)

**Files:** `crates/mika-agent/tests/eval/grounding_regressions/`

Two new scenarios mirroring the #864 pattern:

1. **`dev_groom_fabricated_verdict_caught.rs`** — Mock LLM keyword-matches
   dev-groom, returns `"Plan exists on branch. Verdict: GROOMED"` without
   calling `run_claude_pilot_groom`. Assertion: F3 guard rejects on first
   EndTurn, retry produces non-Verdict response, final assertion via
   `assert_response_forbids(&["Verdict: GROOMED", "Verdict: ESCALATE"])`.
2. **`dev_groom_dispatched_no_verdict.rs`** — Mock LLM keyword-matches
   dev-groom, calls `run_claude_pilot_groom` successfully (mock returns
   `{"task_id": "...", "deferred": false}`), then emits `"Dispatched
   grooming for mika#1133, task <id>"`. Assertion: no guard fires,
   EndTurn accepted, `assert_response_contains(&["Dispatched", "task"])`.

Also add a **dispatcher-manifest parity test** as a static manifest invariant
in the skills unit-test layer, **not** in `grounding_regressions/`. Per the
architect's first-pass F3 finding: `grounding_regressions/` tests runtime LLM
behavior under specific inputs; the parity test is a build-time/static
assertion on manifest files. Mixing the two would violate the cluster's
semantic boundary and reduce discoverability for future contributors looking
for manifest invariants.

**Placement:** colocate with `test_builtin_tool_names_parity` (mika#1217),
which currently lives in `crates/mika-agent/src/tools/mod.rs` per the
mika#1217 commit. If `test_builtin_tool_names_parity` has been moved by the
time of implementation, follow it — the rule is "next to the existing
manifest-invariant parity test," wherever that ends up. Implementer should
grep for `test_builtin_tool_names_parity` at the base SHA to confirm the
location.

Test shape:

```rust
#[test]
fn test_dispatcher_skills_dont_declare_required_suffix_lines() {
    let dispatchers = ["self-dev", "dev-pilot", "dev-groom"];
    for name in dispatchers {
        let manifest = BUNDLED_SKILL_MANIFESTS
            .iter()
            .find(|(n, _, _)| *n == name)
            .expect(&format!("dispatcher skill {} not found", name));
        let parsed: SkillManifest = toml::from_str(manifest.1).unwrap();
        assert!(
            parsed.output.as_ref()
                .map(|o| o.required_suffix_lines.is_empty())
                .unwrap_or(true),
            "Dispatcher skill {} must not declare required_suffix_lines. \
             Dispatchers forward to claude-pilot; verdicts arrive via \
             callback, not from the dispatcher LLM's turn. See mika#1133.",
            name
        );
    }
}
```

This guards against the same shape regressing on a future dispatcher addition.

### F5 — Compound doc

**File:** `docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md`

Sections:
1. **Attribution.** Observed 2026-05-15 on mika#920; dispatched
   "groom mika issue#920" against a ticket with plan committed but no
   architect review record; response carried `"groomed ✅, ... Verdict:
   GROOMED"`, body had no callout.
2. **Root cause.** Manifest pattern mismatch — `dev-groom` carries
   producer-shape `required_suffix_lines` without dispatcher-shape
   `required_tools`. Suffix-line guard (mika#864) re-prompts on missing
   verdict, LLM rationalizes by fabrication.
3. **Five-phase fix.** F1 manifest, F2 prompt, F3 guard, F4 tests, F5 doc.
4. **Dispatcher-vs-producer pattern delineation.** Tabular reference for
   future skill additions. Anchor doc for the new parity test.
5. **Verification.** End-to-end: dispatch `mika ask --agent mika-dev "groom
   mika issue#<unreviewed>"` against a known-unreviewed ticket, observe
   response contains `"Dispatched..."` and no `"Verdict: GROOMED"`.
6. **Lessons.** (a) Manifest shape inversion across skill categories is a
   silent bug-class — parity tests catch the next one. (b)
   `required_suffix_lines` is for LLM-produced verdicts; never for
   dispatch-and-wait flows. (c) Engine-level structural guards beat
   prompt-level "do not" — F3 is the binding enforcement, F2 is for
   self-explanation.

## Acceptance criteria mapping

| AC | Covered by |
|----|-----------|
| AC1 — mika-dev does NOT emit Verdict when no architect review needed | F1 (removes pressure) + F2 (prompt clarifies) |
| AC2 — verify Verdict groundedness when emitted | F3 (structural guard, mode-gated to conversation) |
| AC3 — regression test: known-unreviewed ticket → no Verdict in response | F4 (`dev_groom_fabricated_verdict_caught`) |

## Out of scope

- **mika#920 body callout writeback** — separate concern (mika#1123).
  The fix here prevents the fabrication; it does not retroactively groom
  mika#920. Operator should re-dispatch grooming on mika#920 after this
  fix ships.
- **Engine-side "already groomed" short-circuit on conversation-mode
  `mika ask`** — currently only the autonomous loop (mika#996) checks
  for existing Plan callouts before dispatching dev-groom. Adding the
  same check to the `mika ask` path would deduplicate, but is a separate
  capability ticket. Under the F1+F2 fix, calling "groom" on an
  already-groomed ticket will run a fresh grooming pass — the inner
  `/mika-groom-ticket` spec's idempotency clause ("If the worktree
  exists when this command runs, the existing plan is reused as the
  starting point") absorbs the redundancy.
- **mika-qa parallel risk audit** — confirmed no parallel: qa-review's
  verdict is server-side (GitHub API), not LLM-side. Documented in F5
  compound doc for the audit trail.

## Risks

| Risk | Mitigation |
|------|-----------|
| F2 (`required_tools`) creates a tight retry loop if `run_claude_pilot_groom` itself fails | Existing required-tools gate has terminal-failure bypass (#516 + #890); failed dispatches are recognized and EndTurn accepted |
| F3 false-positive on legitimate callback Verdict delivery | Mode gate (`!mode.is_silent()`) — callback turns are `SilentTrigger::Callback`, bypass F3 entirely |
| F4 parity test breaks on future dispatcher skill that genuinely needs `required_suffix_lines` | Acceptable: if a future skill IS a verdict producer, it shouldn't be in the dispatcher list. The list is small and stable (self-dev, dev-pilot, dev-groom) — additions are deliberate manifest decisions, not accidents |
| Skill version bump (0.1.0 → 0.2.0) on dev-groom — propagates to mika-skills hot-reload? | dev-groom is engine-coupled (bundled), not marketplace-distributed. No hot-reload propagation — change ships with mika binary |

## Rollback

- F1 (skill.toml): `git revert` the manifest line edits; binary rebuild restores
  original behavior. Inert change from the engine's perspective on revert.
- F2 (system_prompt.md): pure prompt edit; revert via `git revert`.
- F3 (agent.rs guard): single guard function with one retry flag; clean
  `git revert`. No data migration. No schema change.
- F4 (tests): `git rm` the two new scenario files and the parity test.
- F5 (doc): `git rm` the compound doc.

No DB schema change. No data migration. No env-var dependency. Pure code change.

## Verification

Pre-merge:
1. `cargo build --release` — clean compile.
2. `cargo test -p mika-agent --test eval grounding_regressions::dev_groom_fabricated_verdict_caught` — F3 catches fabrication.
3. `cargo test -p mika-agent --test eval grounding_regressions::dev_groom_dispatched_no_verdict` — F3 doesn't block legitimate dispatch.
4. `cargo test -p mika-agent test_dispatcher_skills_dont_declare_required_suffix_lines` — parity test passes.
5. `cargo clippy --all-targets --all-features -- -D warnings` — clean.

Post-deploy:
1. Dispatch `mika ask --agent mika-dev "groom mika issue#<known-unreviewed-N>"` against a freshly-filed unreviewed ticket.
2. Observe response: contains `"Dispatched"`, contains a task ID, does NOT contain `"Verdict: GROOMED"` or `"Verdict: ESCALATE"`.
3. Wait for callback (~minutes for a fresh /mika-groom-ticket run).
4. Observe callback message: legitimate Verdict line from the architect, body callout written, ticket dispatch-ready.
5. `grep dev_groom_fabrication_guard $MIKA_SERVER_LOG_FILE` — should be empty on healthy ticks; non-empty entries indicate the LLM is still attempting fabrication and the guard is catching it (file follow-up if persistent).

## File list

```
skills/bundled/dev-groom/skill.toml                              # F1: manifest fix
skills/bundled/dev-groom/system_prompt.md                        # F2: prompt update
crates/mika-agent/src/agent.rs                                   # F3: fabrication guard at position 5b + retry flag
crates/mika-agent/tests/eval/grounding_regressions/dev_groom_fabricated_verdict_caught.rs  # F4: runtime scenario
crates/mika-agent/tests/eval/grounding_regressions/dev_groom_dispatched_no_verdict.rs      # F4: runtime scenario
crates/mika-agent/tests/eval/grounding_regressions/mod.rs        # F4: register two new modules
crates/mika-agent/tests/eval/grounding_regressions/README.md     # F4: vocabulary + scenario-count update (30-31)
crates/mika-agent/src/tools/mod.rs                               # F4: dispatcher-manifest parity test next to test_builtin_tool_names_parity (or wherever that test currently lives at base SHA)
crates/mika-agent/CLAUDE.md                                      # document new post-condition #8b
docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md  # F5: compound doc
```

## Implementation order

Sequence matters because F3's tests (F4) depend on F1+F2 being in place:

1. F1 — `skill.toml` edit (single-file change; rebuilds skill manifest)
2. F2 — `system_prompt.md` edit (cosmetic; doesn't affect test runs)
3. F3 — agent.rs guard + retry flag (compile-blocking work)
4. F4 — tests (depend on F3's guard existing)
5. F5 — compound doc (last; references all four above)

Single PR, single commit per F per `git_commit_skill` conventions, or one
combined commit if the operator prefers.
