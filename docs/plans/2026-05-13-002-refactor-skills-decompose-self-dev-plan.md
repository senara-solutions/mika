---
issue: 1106
type: refactor
title: decompose self-dev along trigger-event axis
branch: refactor/1106/skills-decompose-self-dev-along-trigger
companion: senara-solutions/mika#1105 (ceiling raise 64KB → 80KB — must merge first)
target_ceiling: 49152
audit_date: 2026-05-13
---

# Plan — decompose `self-dev` along trigger-event axis

## Problem (WHY)

`skills/bundled/self-dev/system_prompt.md` on `main` is **65,248 bytes** against a 64KB hard ceiling enforced by `crates/mika-agent/tests/bundled_skills_load.rs::bundled_skills_load_without_oversized_prompts`. Headroom: 288 bytes (0.4%). The file has been touched in 26 commits since 2026-04-23 — sustained growth, not a one-time spike. Two open PRs (mika#1101, mika#1103) are currently blocked, and any future change to `self-dev` will hit the same wall.

Companion ticket mika#1105 raises the ceiling to 80KB as a tactical unblock. This plan delivers the **structural fix** so the ceiling can be tightened again afterward.

## Decision (WHAT)

Decompose `self-dev/system_prompt.md` along the **trigger-event axis** by extracting three trigger-specific subtrees into new sibling skills, following the established `self-dev-<event>` convention used by `self-dev-iterate`, `self-dev-webhook-qa`, `self-dev-webhook-ci`.

| New sibling | Trigger event | Moved bytes | Source range (in current `self-dev/system_prompt.md`) |
|---|---|---:|---|
| `self-dev-callback` | `[claude-pilot]` / `[deploy_mika]` callback messages | 14,142 | L101–238: `### Callback Entry Point (post background task)` |
| `self-dev-webhook-ready-label` | `[GitHub] Issue labeled ready` webhook | 6,397 | L239–296: `### Ready-Label Dispatch` |
| `self-dev-heartbeat` | `[heartbeat trigger]` engine-fired message | 1,197 | L297–308: `### Heartbeat Trigger (mika#991)` |
| **Total moved** | | **21,736** | |

Post-decomposition `self-dev/system_prompt.md`: **43,512 bytes** (5,640B / 8.6% headroom under the 48KB target).

## Section-by-section audit

Sections measured by accumulated line-byte count including heading lines. Total of rows = 65,248B (source file size).

| # | Section (line range) | Bytes | Trigger event | Disposition | Rationale |
|---|---|---:|---|---|---|
| 0 | File preamble (L1–4) | 271 | (frame) | `stay` | Skill identity statement. |
| 1 | `### ROUTING — READ FIRST` (L5–23) | 2,080 | any user message / dispatch tool call | `stay` | Top-level routing table for the always-on host; source-prefix check hands off to siblings. Must stay in the host. |
| 2 | `### Triggering this skill` (L24–26) | 135 | "add feature" / "implement" | `stay` | Keyword surface. |
| 3 | `### User Notifications` (L27–41) | 905 | (cross-cutting) | `stay` | Used by every workflow including milestone/project paths that stay. |
| 4 | `### Workflow` Step 1–3 + Metadata extraction (L42–100) | 4,073 | user free-text / `implement repo issue#N` | `stay` | Generic single-issue dispatch. Direct-user trigger, no sibling owns it. The Metadata-extraction sub-block (L87–100) is the canonical citation for all 3 existing siblings via `> Metadata extraction: see self-dev skill.` — anchor stays. |
| 5 | `### Callback Entry Point` (L101–238) | **14,142** | `[claude-pilot]` / `[deploy_mika]` callback | **`new-sibling: self-dev-callback`** | Largest single block. Trigger is a distinct, engine-classified message shape; engine already enforces it via `callback_milestone_advance` intent-precondition guard. The `recover_unpushed_work` sub-block (L139–179) is already cross-referenced from `self-dev-webhook-qa` (currently points at `self-dev`; will point at `self-dev-callback` post-move). |
| 6 | `### Ready-Label Dispatch` (L239–296) | 6,397 | `[GitHub] Issue labeled ready` webhook | **`new-sibling: self-dev-webhook-ready-label`** | One webhook event type (`issues.labeled` name=`ready`). Engine has dedicated `webhook_ready_label_dispatch` guard. Matches `self-dev-webhook-<event>` naming. Self-contained. |
| 7 | `### Heartbeat Trigger (mika#991)` (L297–308) | 1,197 | `[heartbeat trigger]` engine event | **`new-sibling: self-dev-heartbeat`** | Distinct trigger marker, distinct handler. Smallest of the three but trigger-axis pure. Robustness note: if the architect rejects this carve as too small, dropping it still hits the target (44,709B / 4.4KB headroom). |
| 8 | `### Webhook Fallthrough` (L309–324) | 1,502 | unmatched `[GitHub]` webhook | `stay` | Negative path — fires when no other webhook sibling activates. Belongs in the always-on host that owns the routing table. |
| 9 | `### Block Resumption Commands` (L325–335) | 836 | user "continue" / "skip" / "merge anyway" / "retry" | `stay` | Direct-user trigger, no webhook surface. |
| 10 | `### Completion Signals` + `Step 6 — Close out` (L336–394) | 5,514 | user "task complete" + universal close-out | `stay` | Step 6 is the universal close-out called by every workflow path including the moved callback path. Moving it would force every sibling to duplicate or re-reference. |
| 11 | `## Grooming Dispatch` (L395–445) | 2,458 | user "groom repo#N" | `stay` | Direct-user trigger, no webhook surface. |
| 12 | `## Calibration Rules` (Rules 4, 6, 7, 8, 9, 10, 11) (L446–520) | 7,191 | (cross-cutting discipline) | `stay` | Rule 9 is explicitly about always-on host routing discipline. Other rules are cross-referenced by sibling prompts via "see Rule X". Moving these would require content edits (deduplication) or breaking cross-refs — both forbidden by the ticket's "no content edits" constraint. |
| 13 | `## Milestone Workflow` (L521–725) | **13,719** | user "implement repo milestone#N" + callback M4 advance | `stay` | Direct-user trigger, top-of-table route. Cannot move to a webhook sibling without changing the dispatch axis (ticket forbids). Step M4 is the cascade target for callback advancement — pinned to the host. |
| 14 | `## Project Workflow` (L726–797) | 2,375 | user "implement repo project#N" | `stay` | Direct-user trigger. Same axis as milestone — stays. |
| 15 | `## Resume Semantics` (L798–833) | 1,999 | user "resume" / "continue" / "stop repo milestone#N" | `stay` | Direct-user trigger. Engine guard #702 fires on the always-on host. |

## New sibling specs

### 1. `skills/bundled/self-dev-callback/`

- **Trigger keywords (`skill.toml`):** `["claude-pilot callback", "callback", "long_running:run_claude_pilot", "long_running:deploy_mika", "error_max_turns", "PIPELINE FAILURE"]`
- **`always_on`:** `false`
- **`dependencies`:** `["self-dev"]` (host owns Metadata extraction and Step 6 — Close out)
- **Seeded content:** L101–238 of source `self-dev/system_prompt.md`, intact (~14,142B).
- **Engine guard:** `callback_milestone_advance` intent-precondition continues to fire on the same turn class. No engine code change.

### 2. `skills/bundled/self-dev-webhook-ready-label/`

- **Trigger keywords (`skill.toml`):** `["issue labeled ready", "ready label", "issues.labeled.ready"]`
- **`always_on`:** `false`
- **`dependencies`:** `["self-dev"]`
- **Seeded content:** L239–296 of source, intact (~6,397B).
- **Engine guard:** `webhook_ready_label_dispatch` continues to fire. No engine code change.

### 3. `skills/bundled/self-dev-heartbeat/`

- **Trigger keywords (`skill.toml`):** `["heartbeat trigger", "heartbeat"]`
- **`always_on`:** `false`
- **`dependencies`:** `["self-dev"]`
- **Seeded content:** L297–308 of source, intact (~1,197B).
- **Engine guard:** none (heartbeat is engine-fired, not engine-guarded). Tick scheduler delivers the message; skill matching picks up the keyword.

## Execution sequencing

**Strict prerequisite:** mika#1105 (ceiling 64KB → 80KB) MUST merge first. This ticket cannot ship while the 64KB hard ceiling is active because the move-set is structural — during the move, intermediate states (new siblings created but content not yet pulled from `self-dev`) would still trip the 64KB gate.

Within this ticket:

1. **Create `self-dev-callback/`** first (largest carve, highest leverage). Steps:
   - Create `skills/bundled/self-dev-callback/skill.toml` (template from `self-dev-webhook-ci/skill.toml`).
   - Create `skills/bundled/self-dev-callback/system_prompt.md` from L101–238 of `self-dev/system_prompt.md`, intact.
   - Delete L101–238 from `self-dev/system_prompt.md`.
   - Update `self-dev-webhook-qa/system_prompt.md:138–149` cross-reference from `self-dev` → `self-dev-callback` for the `recover_unpushed_work` verdict class. **This is the only allowed content edit during this ticket** — see Risk 1.
   - Run `cargo test -p mika-agent --test bundled_skills_load` — expect pass.
2. **Create `self-dev-webhook-ready-label/`** — same pattern, L239–296.
3. **Create `self-dev-heartbeat/`** — same pattern, L297–308.
4. **Verify final size** of `skills/bundled/self-dev/system_prompt.md` with `wc -c`. Expect 43,512B (±50B for line-ending drift).
5. **Run the full bundled-skills test:** `cargo test -p mika-agent --test bundled_skills_load -- --nocapture`. All four `self-dev*` skills must scan successfully.
6. **Run autonomous-loop integration tests** (whatever the implementer identifies as covering the keyword-matching dispatch path; at minimum `cargo test -p mika-agent` — full agent crate).
7. **Manual smoke** of the keyword matcher: confirm that a `[claude-pilot] callback` message body routes to `self-dev-callback`, a `[GitHub] Issue labeled ready` body routes to `self-dev-webhook-ready-label`, and a `[heartbeat trigger]` body routes to `self-dev-heartbeat` — with `self-dev` itself no longer claiming those keywords.

## Acceptance criteria

- [ ] `skills/bundled/self-dev/system_prompt.md` byte count is **under 49,152 (48KB)** post-decomposition. Target: 43,512B.
- [ ] `cargo test -p mika-agent --test bundled_skills_load` passes on the resulting branch (with the 80KB ceiling from #1105 still active — the test should pass comfortably under the lower 48KB target inherent to this work, but the runtime ceiling stays at 80KB pending a separate tightening follow-up).
- [ ] Three new sibling directories exist with valid `skill.toml` + `system_prompt.md`.
- [ ] `self-dev-webhook-qa/system_prompt.md:138–149` cross-reference updated to point at `self-dev-callback` (the only content edit; structurally required for the move to be correct).
- [ ] No regression in autonomous-loop integration tests.
- [ ] PR description enumerates: each new sibling, the L<x>–<y> source range it seeded from, and the final `self-dev/system_prompt.md` byte count.

## Risks and flags

1. **Cross-reference update on `self-dev-webhook-qa` is structurally required.** The `recover_unpushed_work` verdict class at `self-dev-webhook-qa/system_prompt.md:138–149` currently says "handled in `self-dev/system_prompt.md`". After moving the Callback Entry Point to `self-dev-callback`, this pointer must be updated. **This is structurally necessary for correctness — the moved content is no longer in `self-dev`.** The ticket's "no content edits" constraint refers to edits *to the moved content itself*; updating a stale pointer in a non-moved sibling is structural, not content. Architect should confirm this interpretation.

2. **Tightening the runtime ceiling is a separate follow-up.** This ticket only brings `self-dev/system_prompt.md` under 48KB. The 80KB runtime constant from #1105 stays in place until a third ticket explicitly tightens it. Out of scope here; flagged so we don't lose track.

3. **Keyword-matching dispatch precedence.** `self-dev` is `always_on=true`. The new siblings will be `always_on=false`. The skill matcher needs to pick the more-specific siblings before falling through to the always-on `self-dev`. If the matcher's precedence rule is purely keyword-match-first (regardless of `always_on`), this works; if `always_on` skills are evaluated first regardless of keyword specificity, the new siblings will never fire. **Implementer must verify** `crates/mika-agent/src/skills/matcher.rs` precedence before declaring the move complete.

4. **Dead content flagged, NOT included in this ticket.** L233–237 of source is a 341-byte tombstone (`### Step 5 — (removed: QA is now webhook-driven)`). Real dead content but ticket forbids content trimming. Stays for a follow-up grooming pass.

5. **Engine guard naming stays stable.** `callback_milestone_advance`, `webhook_ready_label_dispatch`, and the heartbeat tick scheduler all key off message-shape classifiers in the engine, not skill names. No engine code change required; this is purely a prompt-routing refactor.

## Out of scope (echoing ticket)

- Renaming existing siblings.
- Introducing a shared-base + variant-overlay mechanism.
- Trimming load-bearing content. Dead content (Risk 4) deferred to a follow-up.
- Tightening the runtime 80KB ceiling back down to a stricter value.

## Sequencing constraint

**Do NOT add the `ready` label to mika#1106 until mika#1105 has merged.** Echoed from the ticket body; restated here so dispatch automation respects it.
