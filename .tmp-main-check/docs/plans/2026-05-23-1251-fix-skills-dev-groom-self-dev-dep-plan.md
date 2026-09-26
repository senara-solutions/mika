# Plan: fix(skills) — dev-groom unreachable on webhook turns (self-dev dependency edge)

- **Issue:** mika#1251
- **Type:** fix
- **Priority:** p0-critical
- **Branch:** `fix/1251/skills-dev-groom-unreachable-on-webhook`
- **Class:** mika#1173 follow-up (tool-restoration shipped incomplete)

## Problem

`run_claude_pilot_groom` returns `"Unknown tool"` on GitHub-webhook-driven
mika-dev turns (`channel_type='github'`), wedging the autonomous auto-groom
path on every `ready`-labelled ungroomed ticket. Confirmed on mika#1249
(2026-05-23T10:21:06Z, session `c51f7a4b-…`) and mika#1243 (2026-05-22,
`github` session `c274b38f-…`). The same tool **succeeds** on `cli`-channel
turns the same minute (mika#1243 session `2c693eac-…`) — channel→outcome
correlation is 100% across the 5-row evidence sample.

### Root cause (verified against HEAD `b7c663a0`)

`dev-groom` is `always_on = false` (`skills/bundled/dev-groom/skill.toml:5`)
and is **absent** from `self-dev`'s `dependencies` array
(`skills/bundled/self-dev/skill.toml:8-14` — verified: lists `build-mika`,
`deploy-mika`, `dev-pilot`, `browser-control`, `resolve-pr-conflicts`; no
`dev-groom`). On a webhook turn the user message is `[GitHub] Issue labeled
ready on senara-solutions/mika#…`, which contains none of dev-groom's keywords
(`groom`, `groom ticket`, `/mika-groom-ticket`, `groom issue`). So:

1. The matcher's first pass (`crates/mika-agent/src/skills/matcher.rs:42-54`)
   never marks dev-groom matched (no keyword hit, not always_on).
2. The BFS dependency pass (`matcher.rs:56-73`, verified) walks
   `self-dev.dependencies` and pulls in `dev-pilot` as a `Dependency` match —
   but never reaches `dev-groom` because no matched skill declares it.
3. `dev-groom`'s tool therefore never enters the turn's `skill_tools` map
   (`agent.rs:4351` `build_skill_tool_map` iterates only matched entries), and
   the dispatcher's 3-arm chain falls through to the `"Unknown tool"` branch.

`dev-pilot` works on the identical turn solely because `self-dev` (always_on)
lists it as a dependency. After mika#1173 (commit `72021b78`, 2026-05-17)
restored dev-groom as a tool-owning skill — adding the handler, identity
allowlist entry, and dispatch-lib case arm — **the loader-side dependency edge
was never added.** dev-pilot and dev-groom became tool-symmetric but
loader-asymmetric; that asymmetry is the failure surface.

## Fix

One-line edit to `skills/bundled/self-dev/skill.toml`: add `"dev-groom"` to the
`dependencies` array, mirroring the existing `dev-pilot` edge.

```toml
dependencies = [
    "build-mika",
    "deploy-mika",
    # dev-pilot and dev-groom must BOTH be listed — the two dispatch siblings
    # are loader-symmetric. self-dev (always_on) is the only edge that makes a
    # non-always-on dispatch tool reachable on keyword-less webhook turns. See
    # mika#1251 (the gap mika#1173 left) and the parity test in bundled_skills.rs.
    "dev-pilot",
    "dev-groom",
    "browser-control",
    "resolve-pr-conflicts",
]
```

The inline comment is per architect N2 — it makes the loader-symmetry invariant
discoverable at the declaration site, not only in the test file.

No engine code change. `self-dev` is `always_on = true`, so every turn that
runs through it will now resolve dev-groom via the BFS and register
`run_claude_pilot_groom` in the dispatch map.

## Why this fix shape is correct (blast-radius analysis)

Three engine behaviors flip when dev-groom becomes a `Dependency`-reason match
on every always-on turn. All three were verified against HEAD and are either
inert or the documented intent:

1. **`required_tools` is NOT enforced as a per-turn precondition.**
   `collect_required_tools` (`agent.rs:4517-4523`, verified) filters to
   `m.reason == MatchReason::Keyword` only. dev-groom arrives via `Dependency`,
   so its `required_tools = ["run_claude_pilot_groom"]`
   (`dev-groom/skill.toml:18`) is **not** demanded on every turn. This is the
   load-bearing correctness check: the fix does **not** wedge every mika-dev
   turn into "must call run_claude_pilot_groom." (The matcher's own doc comment
   at `matcher.rs:5-8` documents this #463 scoping: required_tools enforce only
   for keyword matches.)

2. **The dev-groom fabrication guard begins firing on conversation turns.**
   `agent.rs:1465-1469` gates the Verdict-fabrication guard on
   `enabled_tool_names.contains("run_claude_pilot_groom") && mode.is_conversation()
   && EndTurn`. Once the tool is loaded this predicate flips true for mika-dev
   conversation turns — which is exactly what the guard's own comment says it
   was written for. Callback turns bypass via `mode.is_conversation()`
   (verified), so inner-session Verdict lines still pass through unaffected. No
   regression; this closes the silent-bypass noted as the bug's secondary
   effect.

3. **Prompt-budget impact is negligible.** `dev-groom/system_prompt.md` is 2496
   bytes. dev-pilot (1623 bytes) is *already* a dependency-injected prompt on
   every always-on turn and works today — the precedent proves the mechanism.
   self-dev's own prompt is 51469 bytes against a declared `max_prompt_size =
   65536`; the existing dependency prompts sum ~9KB and adding dev-groom keeps
   comfortable headroom.

4. **Selector-path analysis — the fix lands on the broken path and does NOT
   leak into autonomous turns.** Three skill selectors exist; all verified at
   HEAD `b7c663a0`:

   | Selector | Trigger path | Resolves deps (BFS)? | dev-groom pulled in? |
   |----------|--------------|----------------------|----------------------|
   | `match_skills` / `match_message` (`matcher.rs:38`, `mod.rs:355`) | **conversation** — incl. the webhook `[GitHub] Issue labeled ready` turn (`agent.rs:2419`) | yes | **yes** — this is the path that broke; the fix targets it |
   | `callback_safe_skills` (`mod.rs:664`) | `SilentTrigger::Callback`/`PostCallbackAdvance`/`DeferredDispatch` (`agent.rs:3584`) | yes (+ includes exec/http) | yes — correct: callbacks continue conversation-authorized work |
   | `safe_always_on_skills` (`mod.rs:624`) | `SilentTrigger::Heartbeat`/`Reflection`/`Reminder`/`SkillRun` (`agent.rs:3588`) | **no** (and strips exec/http) | **no leak** — autonomous turns never pull dev-groom |

   The failing webhook turn is **conversation-mode** (confirmed by the
   fabrication guard's `mode.is_conversation()` gating that the bug relies on),
   so it routes through `match_message` → `match_skills`, whose BFS
   (`matcher.rs:56-73`) resolves the new `self-dev → dev-groom` edge. This
   corrects the first-pass architect brief's claim that the webhook path goes
   through `callback_safe_skills()`: that function is **silent-mode-only**.
   The correction matters for Test B's target (below). Crucially,
   `safe_always_on_skills` does **not** run the BFS, so adding the dependency
   edge cannot cause dev-groom's tool to surface on unsupervised
   heartbeat/reflection turns — resolving architect Surface 1 in-code rather
   than deferring it to implementation.

### Alternatives considered and rejected

- **Set `dev-groom.always_on = true`** — applies globally to every agent
  without a denying allowlist; broader blast radius than the targeted
  self-dev-scoped edge. Rejected.
- **Add `dev-groom` to `self-dev-webhook-ready-label.dependencies`** — scopes to
  the ready-label path only and misses the milestone-cascade M4 auto-groom path
  (`self-dev/system_prompt.md`), which routes through `self-dev` and shares the
  same loader gap. Rejected: the edge belongs on `self-dev` so it covers every
  path that reaches the orchestrator.

## Companion regression tests

Carry **both** tests per the first-pass architect recommendation — they guard
different failure surfaces. Parity alone passes even if a future change makes
`self-dev` not-always-on or regresses the BFS for keyword-less messages; the
behavior test pins the actual matched-set outcome on the path that broke.

**Test A — manifest parity (the config invariant).**
- **Location:** `crates/mika-agent/src/bundled_skills.rs` tests module (the
  bundled `self-dev/skill.toml` is embedded at compile time as a `BundledSkill`
  `files` entry — verified — so the test parses the real shipped manifest, not a
  synthetic fixture).
- **Assertion:** parse the embedded `self-dev` `skill.toml`, assert its
  `dependencies` contains **both** `dev-pilot` **and** `dev-groom`
  (case-insensitive). Frame as a loader-symmetry invariant between the two
  dispatch siblings, comment citing mika#1251 and mika#1173.

**Test B — matcher behavior (the path that broke).**
- **Location:** `crates/mika-agent/src/skills/matcher.rs` tests, alongside the
  existing `test_transitive_dependencies`.
- **Target — `match_skills`, NOT `callback_safe_skills`.** The first-pass
  architect prescribed `callback_safe_skills()` as "the code path for
  webhook/callback turns." That is a factual error corrected here: the webhook
  `[GitHub] Issue labeled ready` turn is **conversation-mode** and routes
  through `match_message` → `match_skills` (`agent.rs:2419`, `mod.rs:355`).
  `callback_safe_skills` is silent-mode-only (`agent.rs:3584`). Testing the
  callback selector would exercise a path the bug never touched. The correction
  is grounded in the selector table above, not preference.
- **Assertion:** construct a synthetic skill set — `self-dev`
  (`always_on = true`, `dependencies = ["dev-pilot", "dev-groom"]`),
  `dev-pilot`, `dev-groom` — call
  `match_skills(&skills, "[GitHub] Issue labeled ready on senara-solutions/mika#9999 — …")`
  (a keyword-less webhook-shaped message), and assert the matched set contains
  `dev-groom` with `MatchReason::Dependency`. This pins that a keyword-less
  message still reaches dev-groom through the always-on `self-dev` edge — the
  exact property that was missing.

## Implementation-time verification (architect Surfaces 2 & 3)

These are confirm-during-`/mika-work` checks, not plan-blocking. Surface 1
(heartbeat leak) is already resolved in the selector analysis above.

- **Surface 2 — tool-description token overhead.** `run_claude_pilot_groom`'s
  tool definition (its `description` in `dev-groom/tools.json`) will now be
  injected into the tools array on **every** mika-dev conversation turn, not
  only keyword-matched ones. Confirm the description is small (line-count check)
  and that prompt assembly respects `max_prompt_size`. Expected negligible —
  dev-pilot's tool already rides every turn — but verify rather than assume.
- **Surface 3 — dispatch arm has no channel/session precondition.** The fix
  makes `run_claude_pilot_groom` *loadable*; confirm the dispatch arm (added by
  mika#1173) makes it *callable* on webhook turns — i.e. it carries no
  `channel_type`/session guard that would convert "Unknown tool" into a
  different silent failure. The 5-row evidence sample only ever observed the
  tool *succeed* on cli channels, so the arm's webhook-channel behavior is
  unexercised. Verify by reading the dispatch arm; if any channel guard exists,
  that is in-scope for this fix (the ticket's goal is webhook-turn grooming).

## Acceptance criteria

> AC3 from the issue body ("mika#1249 auto-grooms end-to-end") is **superseded**:
> mika#1249 merged via #1250 before this fix landed, so it can no longer serve as
> the live reproduction. Validation moves to the regression tests (AC2) plus the
> next autonomous-loop ready-label dispatch after deploy (AC4). The first-pass
> architect endorsed this supersession.

- **AC1.** `dev-groom` appears in `skills/bundled/self-dev/skill.toml`
  `dependencies` array, with the loader-symmetry comment (architect N2).
- **AC2.** Both regression tests pass under `cargo test -p mika-agent`:
  Test A (manifest parity — `self-dev`'s embedded manifest declares both
  `dev-pilot` and `dev-groom`) and Test B (matcher behavior — a keyword-less
  `[GitHub] Issue labeled ready` message resolves `dev-groom` via `match_skills`
  with `MatchReason::Dependency`).
- **AC3.** `cargo build`, `cargo clippy`, and the existing skill/matcher test
  suite pass with no regression in `cli`-channel keyword-triggered grooming.
- **AC4.** Post-deploy validation: after `make deploy` re-seeds the bundled
  skill library, the next `ready`-labelled ungroomed mika ticket auto-grooms on
  its webhook turn without `"Unknown tool: run_claude_pilot_groom"`. (Per
  `feedback_test_with_dev_run` — the change is exercised end-to-end by the next
  autonomous loop run rather than a fabricated reproduction.)

## Deploy note

This is a bundled-skill manifest change. It is embedded at compile time
(`build.rs` recomputes the `self-dev` content hash) and re-seeded into the agent
skill library by `make deploy` (copy-based install). The running mika-dev agent
will not pick up the new dependency edge until redeployed.

## Out of scope (filed/flagged separately)

The two open follow-ups in the issue body are **not** part of this fix and
should be tracked as separate tickets:

1. **dispatch-lib.sh "add a dispatch sibling" checklist** omits the
   `self-dev/skill.toml dependencies` step (this is the omission that let
   mika#1173 ship incomplete). Add the missing step + consider a static parity
   test walking the engine's hardcoded tool-name set against the always-on
   dependency closure.
2. **Loader-vs-engine name-registration asymmetry** as a general failure class
   (the engine hardcodes `"run_claude_pilot_groom"` in ~27 lines but only the
   loader binds a handler).

Keeping these out preserves the one-line-fix severity match; the in-scope tests
(AC2) already guard the specific gap that bit us.

**Disregarded — architect N1 (cross-contamination).** The first-pass review's
non-blocking observation N1 referenced a `Dockerfile.agent` gws checksum pattern
and mika#1243's Dockerfile audit. That is unrelated to mika#1251 (skills loader,
not Docker) — it appears to be context bleed from the architect's earlier
session work on the #1243/#1249 Dockerfile tickets. No action; recorded here so
the bleed is visible rather than silently dropped.
