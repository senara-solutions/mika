---
issue: 1583
type: feat
date: 2026-07-01
---

# Plan — feat(agent-core,skills): nudge-driven skill creation (mika#1583)

## Problem

Sub-issue 1 (mika#1582, **landed on this base**) shipped the `skill_manage` authoring
tool + the `lifecycle_state` activation column + the `allow_authoring` identity gate.
But the tool is dormant — nothing prompts an agent to invoke it when a task pattern is
worth extracting into a skill. This sub-issue adds the **nudge**: a soft, advisory
prompt injection at turn-end that suggests the agent author or refine a skill, gated by
an iteration counter and an identity flag (default off).

The nudge is *advisory* — the agent decides whether to call `skill_manage`. Authored
skills still land `staged` and require operator promotion (sub-issue 1's
`lifecycle_state`), so the authoring path is never load-bearing.

## Grounding — current state of the base (verified 2026-07-01)

Sub-issue 1 primitives are all present on this branch's base:

- `crates/mika-agent/src/prompt.rs:134` — `SkillsIdentityConfig { allowlist: Option<Vec<String>>, allow_authoring: Option<bool> }`, `#[derive(Debug, Deserialize, Clone, Default)]`, `#[serde(default)]` on both fields.
- `crates/mika-agent/src/tools/skill_manage.rs` — `SkillManageTool`, registered unconditionally in `default_tools()` (`tools/mod.rs:818`, name at `tools/mod.rs:753`).
- `crates/mika-agent/src/well_known_agents.rs` — all four well-known identities carry `[skills] allow_authoring = false` (mika-dev `:134`, mika-qa `:219`, mika-arch via `build_mika_arch_identity()` `:388`, mika-test `:1332`).
- `AgentState` lives at `crates/mika-agent/src/server/state.rs:26` — **server-only**; existing atomic precedent `skills_dirty: Arc<AtomicBool>` (`:29`).
- `run_loop` at `crates/mika-agent/src/agent_loop/mod.rs:654` — the single turn loop, shared by conversation / silent / team modes. Does **not** receive `AgentState`.
- `run_agent` / `run_agent_with_deadline` at `agent_loop/mod.rs:2577` / `:2724`; `AgentParams` struct at `:2521`. The system prompt is assembled at `:2758` via `prompt::build_system_prompt(&prompt_ctx)`, then the conversation-summary block is appended right after (`:2763`-`:2768`) — this is the canonical "append a `<...>` block to the assembled prompt" pattern.
- `prompt::build_system_prompt(ctx: &PromptContext)` at `prompt.rs:518`; `PromptContext` is a pure prompt-shaping struct (no per-agent mutable state).
- Identity parse at `prompt.rs:338` `parse_identity_or_fail_closed()` — `toml::from_str` at `:360`, fail-closed sentinel for well-known agents at `:377`-`:390`. **No post-parse field validation today.**

### Two facts that shape the design

1. **`AgentState` is not threaded into `run_loop`.** Cross-turn per-agent state reaches
   the loop by being passed as a borrowed field on `AgentParams`. There is direct
   precedent: `AgentParams.skills_dirty: &'a AtomicBool` (`:2548`) and
   `AgentParams.pr_reviews_posted: Option<&'a Arc<...>>` (`:2571`) are both `AgentState`
   fields threaded by reference, `None`/default in CLI/test. The nudge counter follows
   the identical pattern.

2. **`allow_authoring` gates `skill_manage` at *execution* time only, not visibility.**
   `apply_agent_tool_visibility()` (`agent_loop/mod.rs:4972`) filters only by the
   identity `[tools].disabled` denylist. `skill_manage` is registered for every agent
   and is *always* present in the LLM tool array; `skill_manage.rs` rejects the call at
   execution when `allow_authoring != Some(true)`. **Consequence:** AC8 as written
   ("skill_manage is not in the resolved tool registry when allow_authoring=false") does
   not hold against sub-issue 1's actual implementation. See § AC8 resolution below.

## Design

### New type: `SkillNudgeState`

A small Arc-shareable holder for the two cross-turn atomics, stored on `AgentState`
(the same lifetime/home as `skills_dirty`). New module
`crates/mika-agent/src/agent_loop/skill_nudge.rs` (co-located with the loop that reads
it):

```rust
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Cross-turn nudge counter for a single agent (mika#1583). Lives on `AgentState`,
/// threaded into the agent loop by reference (same pattern as `skills_dirty`).
#[derive(Debug, Default)]
pub struct SkillNudgeState {
    /// Tool-invoking turns since the last nudge fired. Reset to 0 when a nudge fires.
    pub iters_since_skill_nudge: AtomicU32,
    /// Set at turn-end when the threshold is crossed; consumed (cleared) at the next
    /// turn's prompt assembly.
    pub pending_skill_nudge: AtomicBool,
}
```

`AgentState` (`server/state.rs`) gains `pub skill_nudge: Arc<SkillNudgeState>`,
initialised `Arc::new(SkillNudgeState::default())` in the `init_agent` factory (same
site that constructs `skills_dirty`).

### Config: extend `SkillsIdentityConfig`

`prompt.rs:134` — add two `Option` fields mirroring `allow_authoring`:

```rust
/// Whether turn-end skill-authoring nudges are injected for this agent (mika#1583).
/// `None` or `Some(false)` = disabled (default). `Some(true)` = enabled.
#[serde(default)]
pub nudge_enabled: Option<bool>,
/// Nudge cadence in tool-invoking turns (mika#1583). `None` = default 10.
/// `Some(0)` is rejected at identity load (see load-time validation). `Some(N>0)` =
/// nudge every N tool-invoking turns.
#[serde(default)]
pub nudge_interval: Option<u32>,
```

Add accessor helpers on `SkillsIdentityConfig` for the resolved values (keeps the
`unwrap_or` defaults in one place, testable):

```rust
impl SkillsIdentityConfig {
    pub fn nudge_is_enabled(&self) -> bool { self.nudge_enabled.unwrap_or(false) }
    pub fn resolved_nudge_interval(&self) -> u32 { self.nudge_interval.unwrap_or(10) }
    pub fn authoring_enabled(&self) -> bool { self.allow_authoring.unwrap_or(false) }
}
```

### Load-time validation — reject `nudge_interval = Some(0)` (architect F1)

`toml::from_str` accepts `0` as a valid `u32`, so a post-parse check is required. Add a
free validator and call it in `parse_identity_or_fail_closed()` immediately after the
successful `toml::from_str` (`prompt.rs:360`):

```rust
fn validate_skills_config(skills: &SkillsIdentityConfig) -> Result<(), String> {
    if skills.nudge_interval == Some(0) {
        return Err("skills.nudge_interval must be > 0 (use nudge_enabled = false to disable)".into());
    }
    Ok(())
}
```

On `Err`, route into the **existing** malformed-identity path: for well-known agents,
the fail-closed sentinel (`prompt.rs:377`-`:390`); for user-defined agents,
`Identity::default()`. This reuses the established parse-failure handling — no new
error surface. A unit test pins the rejection.

This makes `nudge_enabled` the sole on/off gate and `nudge_interval` always `> 0` when
present — so the runtime check needs **no** zero-guard (architect F1's pseudocode
comment).

### Turn-end check — set `pending_skill_nudge` (in `run_loop`)

The increment + threshold check live in `run_loop` because that is where
`tool_use_occurred` and `enabled_tool_names` already exist (`agent_loop/mod.rs:698`,
`:669`), and it matches the ticket's "end of run_loop's outer iteration, after
post-condition guards have accepted the EndTurn."

Thread a bundled optional config into `run_loop` (avoids five loose params):

```rust
/// Threaded into `run_loop` from `run_agent` when nudges may apply (conversation mode
/// with a server-provided `SkillNudgeState`). `None` in CLI / silent / team modes.
pub(crate) struct SkillNudgeContext<'a> {
    pub state: &'a SkillNudgeState,
    pub enabled: bool,        // identity.skills.nudge_is_enabled()
    pub interval: u32,        // identity.skills.resolved_nudge_interval()
    pub authoring_enabled: bool, // identity.skills.authoring_enabled()
}
```

At the terminal EndTurn-accept point in `run_loop` (right before returning
`Ok(LoopResult::Done(..))`, after all post-condition guards passed):

```rust
if let Some(nudge) = skill_nudge {
    // Count only useful (tool-invoking) turns — mirrors Hermes's iteration semantics.
    if tool_use_occurred {
        nudge.state.iters_since_skill_nudge.fetch_add(1, Ordering::Relaxed);
    }
    // Nudge presupposes a usable authoring path: authoring enabled AND skill_manage
    // actually presented to the LLM this turn. (interval validated > 0 at load.)
    let authoring_usable =
        nudge.authoring_enabled && enabled_tool_names.contains("skill_manage");
    if nudge.enabled
        && authoring_usable
        && nudge.state.iters_since_skill_nudge.load(Ordering::Relaxed) >= nudge.interval
    {
        nudge.state.pending_skill_nudge.store(true, Ordering::Relaxed);
        nudge.state.iters_since_skill_nudge.store(0, Ordering::Relaxed);
    }
}
```

**Counting granularity (interpretation call, flagged for architect).** The issue body
says both "every tool-invoking turn" (Mechanism) and "tool-invoking-step boundary"
(AC2). A `run_loop` invocation *is* one turn (one user message → ≤20 tool steps → one
EndTurn). We increment **once per turn** when any tool was invoked (`tool_use_occurred`),
matching the provenance ("counting *useful* iterations", Hermes) and the nudge block's
"roughly {interval} tool-invoking turns" language. Per-step counting would make the
default interval of 10 fire mid-turn, contradicting "fires on the *next* turn's prompt
assembly, never mid-turn." Per-turn is the coherent reading; called out so the architect
can confirm rather than infer.

### Prompt injection — consume `pending_skill_nudge` (in `run_agent`)

Injection happens at prompt-assembly time, immediately after `build_system_prompt` and
alongside the existing summary-append (`agent_loop/mod.rs:2758`-`:2768`) — **not** inside
`build_system_prompt` (which is a pure, widely-called/​tested prompt builder that should
not gain per-agent mutable state):

```rust
// After: let mut system = ... build_system_prompt(...) ...
if let Some(nudge) = params.skill_nudge {
    if ctx.identity.skills.nudge_is_enabled()
        && nudge.pending_skill_nudge.swap(false, Ordering::Relaxed)
    {
        system.push_str("\n");
        system.push_str(&skill_nudge::render_nudge_block(
            ctx.identity.skills.resolved_nudge_interval(),
        ));
        system.push_str("\n");
    }
}
```

`swap(false, ..)` reads-and-clears atomically, satisfying AC4's "after injection,
`pending_skill_nudge` is cleared." The `nudge_is_enabled()` re-check is
belt-and-suspenders (pending is only ever set while enabled, but an operator could flip
the flag off between turns).

`render_nudge_block(interval: u32) -> String` (in `skill_nudge.rs`) returns the advisory
block verbatim from the issue body:

```
<skill-nudge priority="advisory">
You have completed roughly {interval} tool-invoking turns since the last
skills review. If a recent task pattern is worth extracting into a reusable
skill, consider calling `skill_manage(action="create" | "update" | "inspect")`
this turn. The skill will land `staged` and require operator promotion before
it activates — your authoring is advisory, not load-bearing. If no pattern
stands out, ignore this nudge and proceed normally.
</skill-nudge>
```

### Threading summary

- `AgentParams` (`:2521`) gains `pub skill_nudge: Option<&'a SkillNudgeState>` (mirrors
  `pr_reviews_posted`'s optional-borrowed shape).
- Server callsite(s) that build `AgentParams` from `AgentState` pass
  `Some(&agent_state.skill_nudge)`. CLI / test callsites pass `None`.
- `run_agent_with_deadline` builds `Option<SkillNudgeContext>` from `params.skill_nudge`
  + `ctx.identity.skills.*` and passes it to `run_loop` (new trailing param
  `skill_nudge: Option<&SkillNudgeContext>`).
- `run_silent_agent` / `run_team_agent` call `run_loop` with `None` for the new param
  (silent/team turns don't nudge — see § Scope).

### Well-known agent identities (AC5)

Add `nudge_enabled = false` (a sibling line to the existing `allow_authoring = false`)
to all four identity templates in `well_known_agents.rs`:
mika-dev (`:134` block), mika-qa (`:219`), mika-arch (`build_mika_arch_identity()` format
string, `:388`), mika-test (`:1332`). No `nudge_interval` line needed (default 10 is
correct and `None` is valid).

### AC8 resolution — authoring presupposition

AC8's literal premise (skill_manage absent from the registry when
`allow_authoring=false`) is false: sub-issue 1 gates execution, not visibility, so
skill_manage is always in the array. We satisfy **AC8's intent** — "Nudge presupposes
the authoring path" — by gating the nudge on `authoring_usable = authoring_enabled &&
enabled_tool_names.contains("skill_manage")`. When `allow_authoring=false` the agent
*cannot* successfully author (the tool rejects the call), so nudging would be pure noise;
gating on `allow_authoring` is the semantically correct presupposition. The
`skill_manage`-in-enabled-set conjunct additionally covers the case where an operator
disables skill_manage via `[tools].disabled` while leaving `allow_authoring=true`. This
divergence from AC8's wording (and its rationale) is documented in the PR body.

## Scope

**In scope:** conversation-mode nudging (`run_agent` path) — the operator-facing surface
named as the Phase 1 target. The atomics live on `AgentState` (shared across all of an
agent's turn types), so a later ticket can extend increment/injection to silent/team
modes with no state migration.

**Out of scope** (per issue body + architect F2): authoring itself (sub-issue 1),
curator (sub-issue 3), expansion to autonomous-loop agents (blocked-by the guard-check
finding-spike), hard-gate enforcement, and silent/team-mode injection. Phase 1
enablement of `nudge_enabled = true` on operator-facing agents (Mika Prime,
orchestrator-CC) is an **operator config change on `mika-platform` identity templates**,
tracked as an operator action note — not a code change in this PR (architect F2).

## Implementation units

### U1. `SkillNudgeState` + `render_nudge_block` (new module)

**Files:** `crates/mika-agent/src/agent_loop/skill_nudge.rs` (new), register `mod skill_nudge;` in `agent_loop/mod.rs`.

**Goal:** the state holder, the `render_nudge_block(interval)` renderer, and a pure
decision helper `should_fire_nudge(enabled, authoring_usable, interval, iters) -> bool`
that the turn-end check and the unit tests both call (keeps AC6/7/8 testing off the full
loop).

### U2. Extend `SkillsIdentityConfig` + accessors + load-time validation

**Files:** `crates/mika-agent/src/prompt.rs`.

- Add `nudge_enabled: Option<bool>`, `nudge_interval: Option<u32>` (`#[serde(default)]`).
- Add `nudge_is_enabled()`, `resolved_nudge_interval()`, `authoring_enabled()`.
- Add `validate_skills_config()` and call it in `parse_identity_or_fail_closed()` after
  parse; route `Err` into the existing fail-closed (well-known) / default (user) path.

### U3. `AgentState` field + init

**Files:** `crates/mika-agent/src/server/state.rs` + the `init_agent` factory.

- `pub skill_nudge: Arc<SkillNudgeState>`, initialised alongside `skills_dirty`.

### U4. Thread nudge into the loop + turn-end check + injection

**Files:** `crates/mika-agent/src/agent_loop/mod.rs` + server callsite(s) building `AgentParams`.

- `AgentParams.skill_nudge: Option<&'a SkillNudgeState>`.
- `SkillNudgeContext<'a>` built in `run_agent_with_deadline`; new `run_loop` trailing
  param `skill_nudge: Option<&SkillNudgeContext>`.
- Turn-end increment + `should_fire_nudge` check in `run_loop` at the EndTurn-accept
  terminal point.
- Injection (`swap`-and-append) after `build_system_prompt` in `run_agent_with_deadline`.
- `run_silent_agent` / `run_team_agent` `run_loop` calls pass `None`.
- Server `AgentParams` builders pass `Some(&agent_state.skill_nudge)`; CLI/test pass `None`.

### U5. Well-known identity defaults (AC5)

**Files:** `crates/mika-agent/src/well_known_agents.rs`.

- Add `nudge_enabled = false` to mika-dev / mika-qa / mika-arch / mika-test `[skills]` blocks.

### U6. Tests

**Files:** unit tests in `skill_nudge.rs` and `prompt.rs`; one integration test under
`crates/mika-agent/tests/eval/` if the EvalHarness cheaply exercises injection.

- **AC6** (`skill_nudge.rs`): `should_fire_nudge` returns `false` when `enabled=false`
  for every counter value (incl. `iters >= interval`). Synthetic config values — no
  well-known identity in the fixture.
- **AC7** (`skill_nudge.rs`): with `enabled=true`, `authoring_usable=true`,
  `iters >= interval` → fires; and the turn-end mutation on `SkillNudgeState` sets
  `pending=true` and resets `iters` to 0.
- **AC8** (`skill_nudge.rs`): `authoring_usable=false` (either `allow_authoring=false` or
  `skill_manage` absent from the enabled set) → does not fire, even with `enabled=true`
  and `iters >= interval`.
- **AC4** (`skill_nudge.rs`): `render_nudge_block(10)` contains `priority="advisory"`,
  `skill_manage`, and the `staged`/promotion framing; and an injection-helper test
  asserting the block is appended when pending is set and `pending` is cleared after.
- **F1** (`prompt.rs`): an identity with `nudge_interval = 0` fails validation (routes to
  default / fail-closed); `nudge_interval = 5` and unset both pass.
- **Off-by-default guard** (Failure-disposition detector-class): synthetic identity with
  `nudge_enabled` unset → `nudge_is_enabled() == false` → no injection regardless of
  counter. Well-known identities are **not** in the fixture set.

## Verification contract

- `cargo build -p mika-agent` clean.
- `cargo test -p mika-agent` green, including the six new test groups (AC4/6/7/8, F1,
  off-by-default).
- `cargo clippy -p mika-agent` clean (watch the new `too_many_arguments` on `run_loop` —
  the bundled `SkillNudgeContext` keeps the added surface to one param; existing
  `#[allow(clippy::too_many_arguments)]` on the loop functions already covers it).
- `make verify-bundled-skills` unaffected (no new bundled skill).
- Manual reasoning trace in PR body: (a) fresh agent, `nudge_enabled` unset → never
  nudges; (b) `nudge_enabled=true`, `allow_authoring=false` → never nudges (AC8 intent);
  (c) `nudge_enabled=true`, `allow_authoring=true`, 10 tool-invoking turns → nudge block
  present on turn 11's system prompt, counter reset, block absent on turn 12.

## Definition of Done

- [ ] `SkillsIdentityConfig` has `nudge_enabled: Option<bool>` (default `false`) and `nudge_interval: Option<u32>` (default `10`) with accessors (AC1).
- [ ] `SkillNudgeState { iters_since_skill_nudge: AtomicU32, pending_skill_nudge: AtomicBool }` on `AgentState`; counter increments once per tool-invoking turn (AC2).
- [ ] Turn-end check sets `pending_skill_nudge=true` + resets counter when `nudge_enabled` AND authoring usable AND `iters >= interval` (AC3).
- [ ] Prompt assembly appends the advisory `<skill-nudge>` block (with staged-promote framing) when pending, and clears the flag after (AC4).
- [ ] All four well-known agents ship `nudge_enabled = false`; Prime/orchestrator-CC untouched (AC5).
- [ ] Unit tests AC6, AC7, AC8 pass on synthetic fixtures (well-known identities excluded).
- [ ] `nudge_interval = Some(0)` rejected at identity load (architect F1); Phase-1 enablement recorded as an operator action note, not code (architect F2).
- [ ] `cargo build` / `cargo test` / `cargo clippy` for `mika-agent` all green.

## Acceptance criteria

- AC1. `SkillsIdentityConfig` extended with `nudge_enabled` and `nudge_interval` fields. Defaults: `nudge_enabled = false`, `nudge_interval = 10`.
- AC2. `AgentState` extended with `iters_since_skill_nudge` and `pending_skill_nudge` atomics. Counter increments on the tool-invoking-turn boundary.
- AC3. Turn-end check: when counter >= interval AND `nudge_enabled` AND `skill_manage` is a usable authoring path, sets `pending_skill_nudge = true` and resets counter to 0.
- AC4. Prompt assembly: when `pending_skill_nudge = true`, the `<skill-nudge>` block is appended to the system prompt (named as advisory, includes staged-promote framing); after injection, `pending_skill_nudge` is cleared.
- AC5. All four well-known agents (mika-dev, mika-qa, mika-arch, mika-relay/mika-test) ship with `nudge_enabled = false` explicit in their identity templates. Mika Prime and orchestrator-CC are operator-provisioned; this sub-issue does not modify their identity.toml.
- AC6. Unit test: nudge does NOT inject when `nudge_enabled = false` regardless of counter state (synthetic identity fixture; well-known identities not in the fixture set).
- AC7. Unit test: nudge DOES inject on the synthetic fixture when `nudge_enabled = true` and counter >= interval, after a tool-invoking turn EndTurns. Counter resets to 0 after the check fires.
- AC8. Unit test: when `skill_manage` is not a usable authoring path (gated off by sub-issue 1's `allow_authoring = false`), nudge does NOT inject even if `nudge_enabled = true` and counter >= interval. Nudge presupposes the authoring path. (Implementation note: sub-issue 1 gates `skill_manage` at execution, not visibility, so the gate keys on `allow_authoring` plus `skill_manage` presence rather than presence alone — see § AC8 resolution.)

## Risks

- **Bootstrap hazard, contained.** Nudge fires inside the same agent loop mika-dev/qa/arch run in. Three containments hold: `nudge_enabled = false` default on all four well-known agents; `allow_authoring = false` gates the authoring path (and the nudge, per AC8 resolution); authored skills land `staged` (sub-issue 1).
- **Nudge fatigue.** The block invites "ignore if no pattern." Follow-on tuning ticket if nudge-without-action exceeds 80% over 200 turns — not blocking.
- **Counter-reset on no-action is intentional** — soft-suggestion semantics; the alternative (nag until acted on) is rejected.
- **Counting-granularity interpretation** (per-turn, not per-step) is called out in § Design for architect confirmation — the one place the issue text is ambiguous.

## Provenance

Inspired by Hermes Agent's `_iters_since_skill` counter (default 10). Mika's adaptation:
advisory inline block in the next turn's system prompt (not a background daemon),
identity-gated default-off (multi-tenant safety), staged-then-promote authoring
(lifecycle_state is the load-bearing safety differentiator).
