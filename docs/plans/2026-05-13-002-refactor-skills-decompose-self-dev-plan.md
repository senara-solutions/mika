---
issue: 1106
type: refactor
title: decompose self-dev along trigger-event axis
branch: refactor/1106/skills-decompose-self-dev-along-trigger
companion: senara-solutions/mika#1105 (ceiling raise 64KB → 80KB — must merge first)
target_ceiling: 49152
audit_date: 2026-05-13
revision: 2 (addresses mika-arch first-pass ITERATE findings F1/F2/F3 and NF1/NF2/NF3)
---

# Plan — decompose `self-dev` along trigger-event axis

## Problem (WHY)

`skills/bundled/self-dev/system_prompt.md` on `main` is **65,248 bytes** against a 64KB hard ceiling enforced by `crates/mika-agent/tests/bundled_skills_load.rs::bundled_skills_load_without_oversized_prompts`. Headroom: 288 bytes (0.4%). The file has been touched in 26 commits since 2026-04-23 — sustained growth, not a one-time spike. Two open PRs (mika#1101, mika#1103) are currently blocked, and any future change to `self-dev` will hit the same wall.

Companion ticket mika#1105 raises the ceiling to 80KB as a tactical unblock. This plan delivers the **structural fix** so the ceiling can be tightened again afterward.

## Decision (WHAT)

Decompose `self-dev/system_prompt.md` along the **trigger-event axis** by extracting two trigger-specific subtrees into new sibling skills, following the established `self-dev-<event>` convention used by `self-dev-iterate`, `self-dev-webhook-qa`, `self-dev-webhook-ci`.

| New sibling | Trigger event | Moved bytes | Source range (in current `self-dev/system_prompt.md`) |
|---|---|---:|---|
| `self-dev-callback` | `[claude-pilot]` / `[deploy_mika]` callback messages | 14,142 | L101–238: `### Callback Entry Point (post background task)` |
| `self-dev-webhook-ready-label` | `[GitHub] Issue labeled ready` webhook | 6,397 | L239–296: `### Ready-Label Dispatch` |
| **Total moved** | | **20,539** | |

Post-decomposition `self-dev/system_prompt.md`: **44,709 bytes** (4,443B / 9.0% headroom under the 48KB target).

**`self-dev-heartbeat` was rejected** (was proposed at revision 1; dropped on KISS grounds per architect NF1). The 1,197B Heartbeat Trigger section stays in `self-dev`. A standalone sibling for a 1.2KB content block multiplies skill-directory overhead without commensurate trigger-axis clarity benefit — the 48KB target is hit comfortably without it. If the heartbeat handler grows materially in the future, a dedicated ticket can carve it out then.

## Phase 0 — Pin (matcher precedence and sibling pattern)

**This section resolves the first-pass F1 finding by pinning the load-bearing matcher behavior with citations, instead of deferring verification to the implementer.**

### Matcher precedence rule (citation: `crates/mika-agent/src/skills/matcher.rs:38-54`)

`match_skills()` returns a `Vec<MatchedSkill>` — the **union** of:
1. All enabled `always_on=true` skills (loaded on every turn regardless of message content), AND
2. Any enabled skill where at least one of its `keywords_lower` entries is a substring of `message_lower`.

Excerpt of the production matcher (lines 41-54):

```rust
// First pass: direct matches (always_on or keyword hit), tracking reason
let mut matched_reasons: HashMap<usize, MatchReason> = HashMap::new();
for (i, entry) in skills.iter().enumerate() {
    let keyword_hit = entry
        .keywords_lower
        .iter()
        .any(|kw| message_lower.contains(kw));
    if keyword_hit {
        // Keyword match takes precedence even if also always_on
        matched_reasons.insert(i, MatchReason::Keyword);
    } else if entry.manifest.skill.always_on {
        matched_reasons.insert(i, MatchReason::AlwaysOn);
    }
}
```

The `MatchReason` enum (lines 9-18) distinguishes `AlwaysOn` / `Keyword` / `Dependency` — used downstream for constraint-enforcement scoping (per `crates/mika-agent/CLAUDE.md` Skills System → Match-reason conditioning, #463). For the purposes of this plan, the load-bearing fact is: **both always-on and keyword-matched skills are loaded into the same turn**. A new sibling with `always_on=false` and a keyword that hits a specific message will co-fire with `self-dev` (`always_on=true`) on that message — no precedence shadow, no race.

### Existing-sibling pattern is already proven

`always_on` values from the four `self-dev*` `skill.toml` files on `main` (`grep -E "^always_on" skills/bundled/self-dev*/skill.toml`):

```
skills/bundled/self-dev/skill.toml:always_on = true
skills/bundled/self-dev-iterate/skill.toml:always_on = false
skills/bundled/self-dev-webhook-qa/skill.toml:always_on = false
skills/bundled/self-dev-webhook-ci/skill.toml:always_on = false
```

The three existing `always_on=false` siblings have been operating in production for several months and are referenced by the engine guards `webhook_ready_label_dispatch` and similar (per `mika-agent/CLAUDE.md` Post-Conditions section). The pattern this plan extends — `always_on=true` host + `always_on=false` trigger-specific siblings — is the existing pattern, not a new mechanism.

**Conclusion:** the matcher will load both `self-dev` and the new sibling on the keyword-matched turn. No code change required. No matcher integration test required. The new siblings will fire on their declared keywords; `self-dev` will continue to fire on every turn for routing / generic workflow / shared close-out.

## Section-by-section audit

Sections measured by accumulated line-byte count including heading lines. Total of rows = 65,248B (source file size).

| # | Section (line range) | Bytes | Trigger event | Disposition | Rationale |
|---|---|---:|---|---|---|
| 0 | File preamble (L1–4) | 271 | (frame) | `stay` | Skill identity statement. |
| 1 | `### ROUTING — READ FIRST` (L5–23) | 2,080 | any user message / dispatch tool call | `stay` | Top-level routing table for the always-on host; source-prefix check hands off to siblings. Must stay in the host. |
| 2 | `### Triggering this skill` (L24–26) | 135 | "add feature" / "implement" | `stay` | Keyword surface. |
| 3 | `### User Notifications` (L27–41) | 905 | (cross-cutting) | `stay` | Used by every workflow including milestone/project paths that stay. |
| 4 | `### Workflow` Step 1–3 + Metadata extraction (L42–100) | 4,073 | user free-text / `implement repo issue#N` | `stay` | Generic single-issue dispatch. Direct-user trigger, no sibling owns it. Metadata-extraction sub-block (L87–100) is the canonical citation for all 3 existing siblings via `> Metadata extraction: see self-dev skill.` — anchor stays. |
| 5 | `### Callback Entry Point` (L101–238) | **14,142** | `[claude-pilot]` / `[deploy_mika]` callback | **`new-sibling: self-dev-callback`** | Largest single block. Engine already enforces this trigger class via `callback_milestone_advance` intent-precondition guard (mika-agent/CLAUDE.md §6b). The `recover_unpushed_work` sub-block (L139–179) is already cross-referenced from `self-dev-webhook-qa` (currently points at `self-dev`; will point at `self-dev-callback` post-move — see "Cross-reference correction" below). |
| 6 | `### Ready-Label Dispatch` (L239–296) | 6,397 | `[GitHub] Issue labeled ready` webhook | **`new-sibling: self-dev-webhook-ready-label`** | One webhook event type (`issues.labeled` name=`ready`). Engine has dedicated `webhook_ready_label_dispatch` guard (mika-agent/CLAUDE.md §6 entry a). Matches `self-dev-webhook-<event>` naming. Self-contained. |
| 7 | `### Heartbeat Trigger (mika#991)` (L297–308) | 1,197 | `[heartbeat trigger]` engine event | `stay` | Trigger-axis pure but only 1,197B; dropping into a standalone sibling on KISS-over-axis-purity grounds (architect NF1). The 48KB target is met without this carve. Revisit if the heartbeat content grows. |
| 8 | `### Webhook Fallthrough` (L309–324) | 1,502 | unmatched `[GitHub]` webhook | `stay` | Negative path — fires when no other webhook sibling activates. Belongs in the always-on host that owns the routing table. |
| 9 | `### Block Resumption Commands` (L325–335) | 836 | user "continue" / "skip" / "merge anyway" / "retry" | `stay` | Direct-user trigger, no webhook surface. |
| 10 | `### Completion Signals` + `Step 6 — Close out` (L336–394) | 5,514 | user "task complete" + universal close-out | `stay` | Step 6 is the universal close-out called by every workflow path including the moved callback path. Moving it would force every sibling to duplicate or re-reference. |
| 11 | `## Grooming Dispatch` (L395–445) | 2,458 | user "groom repo#N" | `stay` | Direct-user trigger, no webhook surface. |
| 12 | `## Calibration Rules` (Rules 4, 6, 7, 8, 9, 10, 11) (L446–520) | 7,191 | (cross-cutting discipline) | `stay` | **Decision** (not uncertainty per NF2): Rules 4/6/7/8/9/10/11 are cross-cutting prose discipline. They are referenced by sibling prompts via "see Rule X" cross-refs. Moving them would force either content edits (deduplication of overlapping rule text), breaking cross-refs, or introducing a `_shared/`-style prose mechanism. All three are forbidden by the ticket's scope (no content edits; no shared-base+overlay mechanism; the existing `_shared/dispatch-lib.sh` pattern is for executable plumbing, not prose). Calibration Rules stay in `self-dev`. |
| 13 | `## Milestone Workflow` (L521–725) | **13,719** | user "implement repo milestone#N" + callback M4 advance | `stay` | Direct-user trigger, top-of-table route. Cannot move to a webhook sibling without changing the dispatch axis (ticket forbids). Step M4 is the cascade target for callback advancement — pinned to the host. |
| 14 | `## Project Workflow` (L726–797) | 2,375 | user "implement repo project#N" | `stay` | Direct-user trigger. Same axis as milestone — stays. |
| 15 | `## Resume Semantics` (L798–833) | 1,999 | user "resume" / "continue" / "stop repo milestone#N" | `stay` | Direct-user trigger. Engine guard `resume_reconcile` fires on the always-on host (mika-agent/CLAUDE.md §6 entry d). |

## New sibling specs

### 1. `skills/bundled/self-dev-callback/`

- **Trigger keywords (`skill.toml`):** `["claude-pilot callback", "callback", "long_running:run_claude_pilot", "long_running:deploy_mika", "error_max_turns", "PIPELINE FAILURE"]`
- **`always_on`:** `false`
- **`dependencies`:** `["self-dev"]` (host owns Metadata extraction and Step 6 — Close out)
- **`skill.toml` estimated size:** ~300B (template from `self-dev-webhook-qa/skill.toml` which is 349B)
- **Seeded content:** L101–238 of source `self-dev/system_prompt.md`, intact (~14,142B).
- **Engine guard:** `callback_milestone_advance` intent-precondition continues to fire on the same turn class. No engine code change.

### 2. `skills/bundled/self-dev-webhook-ready-label/`

- **Trigger keywords (`skill.toml`):** `["issue labeled ready", "ready label", "issues.labeled.ready"]`
- **`always_on`:** `false`
- **`dependencies`:** `["self-dev"]`
- **`skill.toml` estimated size:** ~250B (template from `self-dev-webhook-ci/skill.toml` which is 244B)
- **Seeded content:** L239–296 of source, intact (~6,397B).
- **Engine guard:** `webhook_ready_label_dispatch` continues to fire. No engine code change.

### `skill.toml` overhead (per NF3)

Two new `skill.toml` files at ~250-350B each = ~550-700B total overhead. The 64KB / 80KB ceiling test (`bundled_skills_load_without_oversized_prompts`) only enforces against each `system_prompt.md` independently, not against total directory size. The new `skill.toml` files do not affect any ceiling test. Mentioned here for completeness so the plan's byte math is fully enumerated.

## Cross-reference correction (decision committed — F2 resolved)

`self-dev-webhook-qa/system_prompt.md:138–149` (Verdict Class: `recover_unpushed_work`) currently states the recovery logic is "handled in `self-dev/system_prompt.md`." After moving the Callback Entry Point to `self-dev-callback`, that pointer is **stale and incorrect** — the recovery content lives in `self-dev-callback`, not `self-dev`.

**Decision: update the pointer to reference `self-dev-callback`.** This is a structural defect correction, not a content edit:

- **Definition:** A cross-reference that names a file in which the referenced content no longer resides is a broken link — a correctness defect introduced by the structural move.
- **Why "no content edits" doesn't forbid this:** The ticket's "no content edits" constraint scopes to "the moved content itself" (don't rewrite the moved prose) and "load-bearing content" (don't trim sections to make the math work). A stale pointer in a non-moved sibling is neither moved nor trimmed — it is collateral to the move and must be corrected for the new structure to be coherent. Leaving the pointer pointing at the wrong file would leave a known-broken reference in production, which is a worse outcome than fixing a pointer.
- **Scope discipline:** This is the **only content edit** introduced by this ticket. The plan explicitly forbids any other content edit (no trimming, no reformatting, no deduplication, no rule renaming). If during implementation the architect or implementer notices another change that would require touching prose, that change must be filed as a follow-up ticket.

Alternative considered and rejected: leave `recover_unpushed_work` (the 5,193B sub-block at L139–179 within the callback section) in `self-dev` as a free-standing section. This was rejected because (a) it defeats roughly 37% of the callback move's leverage (5,193 / 14,142 = 36.7%), (b) it severs the cohesive callback-handler content across two files for no structural benefit, and (c) the pointer-update interpretation is independently defensible.

## Execution sequencing

### Hard prerequisite: rebase on mika#1105 merge commit before testing (F3 operational note)

**Do not run `cargo test -p mika-agent --test bundled_skills_load` on intermediate states where mika#1105 has not yet merged.** Reasoning: during the move, the implementer will create new sibling directories and excise content from `self-dev/system_prompt.md`. At intermediate commit boundaries, `self-dev/system_prompt.md` may still be at 65,248B (before content is excised) which trips the 64KB gate.

**Operational instruction to the implementer:**

1. Wait for mika#1105 to merge to `main`.
2. `git -C mika fetch origin main`
3. `git -C mika rebase origin/main` from this branch (`refactor/1106/skills-decompose-self-dev-along-trigger`).
4. Confirm `crates/mika-agent/tests/bundled_skills_load.rs` reflects the new 80KB ceiling (per the AC of #1105) before proceeding.
5. Then begin the in-branch move work below.

If `git rebase` produces conflicts (likely, since #1105 touches the same test file), resolve in favor of the rebased `main` content — this branch has not yet modified that file.

### Within this ticket (post-rebase)

1. **Create `self-dev-callback/`** first (largest carve, highest leverage):
   - Create `skills/bundled/self-dev-callback/skill.toml` (template from `self-dev-webhook-qa/skill.toml`, swap `name`, `keywords`, `dependencies`).
   - Create `skills/bundled/self-dev-callback/system_prompt.md` from L101–238 of `self-dev/system_prompt.md`, intact.
   - Delete L101–238 from `self-dev/system_prompt.md`.
   - **Apply the F2 cross-reference correction:** edit `self-dev-webhook-qa/system_prompt.md:138–149` to point at `self-dev-callback` for the `recover_unpushed_work` verdict class.
   - Run `cargo test -p mika-agent --test bundled_skills_load -- --nocapture` — expect all four `self-dev*` skills (host + 3 existing siblings + 1 new sibling = 4 of 4) scan successfully.
2. **Create `self-dev-webhook-ready-label/`** — same pattern, L239–296. Delete from source.
3. **Verify final size** of `skills/bundled/self-dev/system_prompt.md` with `wc -c`. Expect **44,709B (±50B for line-ending drift)**.
4. **Run the full bundled-skills test:** `cargo test -p mika-agent --test bundled_skills_load -- --nocapture`. All five `self-dev*` skills must scan successfully.
5. **Run the broader agent test suite:** `cargo test -p mika-agent`. No regression in skill-matcher tests (`matcher.rs::tests::*`) or autonomous-loop integration tests.
6. **Manual smoke** of the keyword matcher: confirm that a `[claude-pilot] callback` user message body routes to `self-dev-callback` (keyword reason), a `[GitHub] Issue labeled ready` body routes to `self-dev-webhook-ready-label` (keyword reason), and a `[heartbeat trigger]` body routes to `self-dev` (always-on reason — heartbeat stayed in host per NF1).

## Acceptance criteria

- [ ] **Prerequisite gate:** mika#1105 merged to `main`; this branch rebased onto the merge commit before running any size-gate test.
- [ ] `skills/bundled/self-dev/system_prompt.md` byte count is **under 49,152 (48KB)** post-decomposition. Target: **44,709B**.
- [ ] `cargo test -p mika-agent --test bundled_skills_load` passes on the resulting branch (under the 80KB runtime ceiling from #1105 — comfortably so under the lower 48KB target inherent to this work).
- [ ] `cargo test -p mika-agent` passes — no regression in matcher tests or autonomous-loop integration tests.
- [ ] Two new sibling directories exist under `skills/bundled/`:
   - `self-dev-callback/` with valid `skill.toml` + `system_prompt.md` (~14,142B).
   - `self-dev-webhook-ready-label/` with valid `skill.toml` + `system_prompt.md` (~6,397B).
- [ ] `self-dev-webhook-qa/system_prompt.md:138–149` cross-reference updated to point at `self-dev-callback` for the `recover_unpushed_work` verdict class (the single permitted content edit; structurally required for the move to be correct — see "Cross-reference correction" above).
- [ ] PR description enumerates: each new sibling, the L\<x\>–\<y\> source range it seeded from, the final `self-dev/system_prompt.md` byte count, and the cross-reference correction.

## Risks and flags

1. **Cross-reference correction is the single permitted content edit.** Resolved as a structural defect correction in "Cross-reference correction" above. If the implementer encounters a second cross-reference that needs updating (one not anticipated by this plan), that is a signal — the implementer must surface it for grooming review, not silently apply it. The "no content edits" constraint is strict; one collateral fix is justified, two means the move-axis hasn't been fully audited.

2. **Tightening the runtime ceiling is a separate follow-up.** This ticket only brings `self-dev/system_prompt.md` under 48KB. The 80KB runtime constant from #1105 stays in place until a third ticket explicitly tightens it. Out of scope here; flagged so we don't lose track.

3. **Matcher precedence is no longer a risk.** Resolved in Phase 0 Pin above with `matcher.rs:38-54` citation and existing-sibling evidence. The new siblings will fire on their keywords; the host fires always-on. Union semantics, no precedence shadow.

4. **Sequencing dependency is operationalized.** Resolved in "Hard prerequisite" above with explicit rebase instruction. The implementer does not run the size-gate test on intermediate `main` states.

5. **Dead content flagged, NOT included in this ticket.** L233–237 of source is a 341-byte tombstone (`### Step 5 — (removed: QA is now webhook-driven)`). Real dead content but ticket forbids content trimming. Stays for a follow-up grooming pass.

6. **Engine guard naming stays stable.** `callback_milestone_advance`, `webhook_ready_label_dispatch`, and the heartbeat tick scheduler all key off message-shape classifiers in the engine, not skill names (citation: `mika-agent/CLAUDE.md` Post-Conditions section). No engine code change required; this is purely a prompt-routing refactor.

## Out of scope (echoing ticket)

- Renaming existing siblings.
- Introducing a shared-base + variant-overlay mechanism for prose discipline rules.
- Trimming load-bearing content. Dead content (Risk 5) deferred to a follow-up.
- Tightening the runtime 80KB ceiling back down to a stricter value.
- Carving out `self-dev-heartbeat` as a standalone sibling. Dropped on KISS grounds (NF1); revisit if the heartbeat content grows materially.

## Sequencing constraint (repeated for visibility)

**Do NOT add the `ready` label to mika#1106 until mika#1105 has merged.** Echoed from the ticket body; restated here so dispatch automation respects it. The hard prerequisite in "Execution sequencing" above operationalizes this for the implementer.
