---
title: Add dev-handsoff bundled skill (v0.1, artifact-only)
type: feat
status: active
date: 2026-05-06
---

# Add dev-handsoff bundled skill (v0.1, artifact-only)

## Overview

Create a new bundled skill at `mika/skills/bundled/dev-handsoff/` that mika-dev fires at end of an autonomous run to write a structured handsoff artifact to `<worktree>/.mika/handsoff/<TODAY>[-<slug>].md`. v0.1 is artifact-only — no git operations, no commit, no push, no cross-repo writes. The skill emits a terminal signal block pointing at the artifact; the v0.2 pipeline pickup (separate ticket on `mika-platform`) reads the artifact, transforms it into a HANDSOFF-CONTRACT-conformant published log at `mika-platform/docs/logs/<date>-mika-dev[-<slug>].md`, and commits + pushes it.

## Problem Frame

mika-dev currently has no structured way to wrap an autonomous run. End-of-run knowledge (tickets touched, decisions in flight, filed work, next-session checklist) is volatile — lost between runs unless serialized into a file. Operator-Claude has `/mika-handsoff` (mika-platform#81, groomed today, ready-labelled) for the conversational case. mika-dev's autonomous case has been the missing third artifact in the 67d85cfa thread plan; this ticket closes that.

The architecture intentionally splits artifact-writing from publishing into two phases:
- **v0.1 (this ticket):** mika-dev writes a structured artifact in its own worktree at `.mika/handsoff/`. No cross-repo writes. No git ops. mika-dev's writer-scope is its own worktree, period (per `feedback_work_in_terms_of_mika`).
- **v0.2 (separate ticket on mika-platform, deferred):** the `/mika` pipeline's wrap phase scans worktree-local handsoff artifacts, transforms them to HANDSOFF-CONTRACT format, lands them in `mika-platform/docs/logs/`, commits + pushes.

This split keeps actor-mapping clean: mika-dev writes content (it knows what happened), the pipeline wrap-phase publishes it (it already crosses the sub-repo / mika-platform boundary).

## Requirements Trace

- **R1.** Two new files: `mika/skills/bundled/dev-handsoff/skill.toml` and `mika/skills/bundled/dev-handsoff/system_prompt.md` (note: convention is `system_prompt.md`, NOT `PROMPT.md` — see Key Technical Decisions).
- **R2.** Skill manifest: prompt-only (no handler script), keyword-triggered (multi-word phrases per `project_keyword_substring_false_positives`), `always_on=false`, six trigger phrases per ticket.
- **R3.** Skill writes to `<worktree>/.mika/handsoff/<TODAY>[-<slug>].md` only. No git mutations. No cross-repo writes.
- **R4.** Slug resolution chain (ticket Artifact Contract §1): work-item → session-id-prefix → empty (rule 3 = first-of-day, no work-item).
- **R5.** YAML frontmatter schema (ticket Artifact Contract §2): `date`, `session_id`, `actor`, `branch`, `repo`, `summary`, `work_item`, `slug`, `runs[]` — all required, types pinned. v0.2-routing layer.
- **R6.** Body sections match HANDSOFF-CONTRACT exactly (see D2 below) — TL;DR, Story so far, Tickets & PRs touched, Blocked / carry-forward, Decisions in flight, Filed today, What to do next session. Sections drop-if-empty.
- **R7.** Per-section append discipline (ticket Artifact Contract §3): append for prose/lists, update-in-place-by-Ref for tables, wholesale-rewrite for next-session checklist.
- **R8.** Phase 5 emits exact signal block (Path, Summary, Pickup) — no restated content.
- **R9.** Zero `AskUserQuestion` calls — synthesis-only, slug fallback chain guarantees buildable path.
- **R10.** Voice: third-person throughout ("mika-dev did X").
- **R11.** Verification: all 14 steps from ticket pass (manifest parse, keyword activation, negative keyword test, required_tools sanity, keyword-conflict grep, end-to-end new-file rules 1/2/3, continuation append, frontmatter schema, voice check, signal-block check, zero-question check, no-git check).

## Scope Boundaries

- Two-file bundled skill only. No Rust changes. No `SkillManifest` field additions. No handler script. No `tools.json`.
- No git operations from the skill (`git add`, `git commit`, `git push` — all forbidden). Read-only `git rev-parse --show-toplevel` for worktree-root detection IS allowed (the only git invocation in the skill).
- No cross-repo writes. The skill writes only inside the sub-repo worktree where mika-dev is running.
- No automated tests beyond the verification list. Bundled skills are prompt+manifest contracts; integration is manual via mika skills validate + smoke runs.

### Deferred to Separate Tasks

- **Session-end lifecycle hook on mika-agent loop** (ticket Dependencies §1) → separate ticket on `mika`. Adds `[lifecycle].on_session_end` to `SkillManifest` so the skill fires automatically rather than via keyword trigger only.
- **Pipeline pickup step in mika-platform** (ticket Dependencies §2) → separate ticket on `mika-platform`. Reads v0.1 artifacts, transforms to HANDSOFF-CONTRACT format, lands in `mika-platform/docs/logs/`, commits, pushes.

## Context & Research

### Relevant Code and Patterns

- `mika/skills/bundled/self-dev/skill.toml` + `system_prompt.md` — canonical prompt-only bundled skill structure. Verified at grooming: directory contains exactly two files (no handlers/, no tools.json). Uses `system_prompt.md` filename (NOT `PROMPT.md` as ticket body suggests — see D1).
- `mika/skills/bundled/dev-pilot/` — handler-bearing bundled skill structure (handlers/, tools.json, skill.toml, system_prompt.md). NOT the right reference for v0.1; self-dev is.
- `mika/crates/mika-agent/src/skills/manifest.rs:13–52` — `SkillManifest` schema. v0.1 conforms; no changes needed. v0.2 dependency #1 adds `[lifecycle]` field here.
- `mika-skills/CLAUDE.md` lines 79–116 — manifest field reference (canonical doc).
- `mika-platform/docs/logs/HANDSOFF-CONTRACT.md` — the HANDSOFF-CONTRACT spec, shipped via PR #82 (merged 2026-05-05). The artifact body sections must match this contract — see D2.
- Deployed bundled skills' `required_tools` values (verified via grep): `gh_read`, `qa_pr_view`, `run_gh`, `run_shell`, `build_mika`, `review_skill`, `run_claude_pilot`. **None match the ticket's suggested `["bash", "edit", "write"]`** — those are Claude Code tool names, not mika-internal builtin tool names. See D3.

### Institutional Learnings

- **Memory `feedback_work_in_terms_of_mika`** — writer-scope discipline. mika-dev writes only inside its own worktree. v0.1's "no cross-repo writes" rule is the codification.
- **Memory `project_keyword_substring_false_positives`** — keyword shape. Multi-word phrases only; no single-word triggers like "session", "wrap", "build". The six trigger phrases all comply.
- **Memory `feedback_orthogonality_flag_semantics`** — actor clarity. mika-dev should not have two writer-scopes (its worktree AND mika-platform's logs dir). v0.1 preserves single-writer-scope.

### External References

- None. This is a workspace-internal bundled skill.

## Key Technical Decisions

- **D1 (correction): prompt filename is `system_prompt.md`, NOT `PROMPT.md`.** Ticket body says `PROMPT.md` but the deployed convention across all 20+ bundled skills is `system_prompt.md`. Verified at grooming: `self-dev/`, `dev-pilot/`, `dev-groom/`, etc. all use `system_prompt.md`. Plan uses `system_prompt.md`. Ticket body discrepancy noted; not blocking.

- **D2 (reconciliation against HANDSOFF-CONTRACT, mika-platform#80): artifact body sections match HANDSOFF-CONTRACT exactly.** The ticket's "Spec dependency" note explicitly requests this reconciliation during grooming. The ticket's Artifact Contract §3 lists 5 sections (Story so far, Tickets touched, Decisions in flight, Filed today, What to do next session). HANDSOFF-CONTRACT lists 8 (TL;DR, Story so far, Tickets & PRs touched, Blocked / carry-forward, Decisions in flight, Filed today, What to do next session, Sessions).
    - **Reconciled:** the artifact body uses HANDSOFF-CONTRACT's section names exactly. This means:
        - Add `## TL;DR` (drop if no prose summary; YAML `summary` frontmatter is the routing layer, the body TL;DR is the human-readable reproduction).
        - Rename `## Tickets touched` → `## Tickets & PRs touched` (matches HANDSOFF-CONTRACT verbatim).
        - Add `## Blocked / carry-forward` (drop if empty).
        - Omit `## Sessions` from the skill's output (drop-if-empty rule applies; YAML `runs[]` frontmatter is the canonical session-list surface; pickup may inject the body Sessions section if needed for the published log).
    - **Why:** v0.2 pickup is a transformation step. Identity transformation of body sections (path-rename only) is simpler and less error-prone than mapping. DRY at the contract layer means using HANDSOFF-CONTRACT's section names verbatim.
    - **Append discipline:** unchanged from ticket Artifact Contract §3 — append/update-in-place-by-Ref/wholesale-rewrite per section semantics. The added sections (TL;DR, Blocked / carry-forward) follow the same pattern (TL;DR rewrites wholesale on continuation, since it's the latest-summary; Blocked / carry-forward appends).

- **D3: `required_tools` is implementation-discovery, not pre-decided.** Ticket suggests `["bash", "edit", "write"]` but those are Claude Code tool names, not mika-internal builtin tool names. Deployed bundled skills use mika-internal names (`gh_read`, `run_shell`, `run_claude_pilot`, etc.). The implementer must:
    1. Read mika's tool registry to identify the canonical names for: file-read, file-write, mkdir-p, and read-only git operations.
    2. Set `required_tools` to the matching mika-internal names.
    3. Verify by `grep -h "^required_tools" mika/skills/bundled/*/skill.toml | sort -u` — the new value should align with the existing-skills surface or be a justified additive.
    Plan does NOT pre-commit specific tool names; defer to grooming-time tool-registry consultation. Ticket Verification step #4 codifies this check.

- **D4: keyword-conflict grep is mandatory before commit.** Ticket Verification step #5 + Pre-dispatch confirmations §2 specify the runnable grep. Plan inherits this gate. The grep must run from cwd `/data/workspace/mika-platform` and must report `CLEAN '<phrase>'` for all six configured phrases. The silent-zero failure mode (grep against non-existent path) is real — the ticket explicitly calls out `mika-platform/.claude/commands/` is wrong (should be `.claude/commands/` since cwd IS mika-platform).

- **D5: skill is keyword-triggered in v0.1; lifecycle-triggered in v0.2.** Activation path is mika-dev sees a configured phrase in the conversation context, fires the skill. v0.1 makes no attempt to auto-fire at session-end — that requires the lifecycle hook (deferred to v0.2 dependency #1). Operator/dev-pilot caller invokes by saying one of the six phrases.

- **D6: zero `AskUserQuestion` is load-bearing on the slug fallback chain.** The skill never asks the operator for input. The slug fallback chain (work-item → session-id-prefix → empty) guarantees a deterministic path under all conditions. If synthesis is wrong (e.g., wrong work_item, terse Story so far), the operator corrects in a follow-up turn. No interrogation.

- **D7: third-person voice throughout.** "mika-dev did X." Voice consistency is load-bearing because the v0.2-published log is dual-audience (engineering trace + Director narrative source). First-person leaks would force pickup to do voice transformation; identity transformation is cleaner.

## Open Questions

### Resolved During Planning

- **Q: Filename `PROMPT.md` vs `system_prompt.md`?** Resolved: `system_prompt.md` per deployed convention. Ticket body discrepancy.
- **Q: Should artifact sections match HANDSOFF-CONTRACT or stay minimal?** Resolved D2: match HANDSOFF-CONTRACT exactly. DRY at the contract layer; pickup is identity transformation.
- **Q: Should `required_tools` be `["bash", "edit", "write"]`?** Resolved D3: NO. Those are Claude Code tool names, not mika-internal builtin names. Defer to implementer to consult the tool registry; ticket Verification #4 codifies the check.
- **Q: What happens when both rule 1 (work-item) and rule 2 (session-id-prefix) could apply?** Resolved: rule 1 wins (priority order in slug fallback chain). The "first match" rule is unambiguous.
- **Q: Should the skill capture session metadata that's not in the YAML frontmatter (e.g., LLM tokens used, dispatch counts)?** Resolved: NO. v0.1 captures handsoff narrative + routing metadata. Operational metrics belong in observability/audit surfaces, not handsoff logs.

### Deferred to Implementation

- **`required_tools` exact value** (D3). Implementer reads mika's tool registry and selects canonical names.
- **Path of mika's tool registry source** for the D3 check. Likely `mika/crates/mika-agent/src/tools/` or similar; implementer locates and grep.
- **Whether `runs[]` entry timestamps use UTC or local time.** YAML frontmatter `runs[].started_at` and `ended_at` are ISO-8601 — implementer picks UTC (consistent with `<TODAY>` per ticket Artifact Contract §1: `date -u +%Y-%m-%d`).

## Implementation Units

- [ ] **Unit 1: Create `mika/skills/bundled/dev-handsoff/` directory with two files**

**Goal:** Land the bundled skill — manifest + prompt — that writes the handsoff artifact per the contract.

**Requirements:** R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11

**Dependencies:** mika-platform#80 (HANDSOFF-CONTRACT.md) — already shipped via PR #82. Section names in D2 reference this.

**Files:**
- Create: `skills/bundled/dev-handsoff/skill.toml`
- Create: `skills/bundled/dev-handsoff/system_prompt.md`

**Approach:**
- **`skill.toml`:**
    - `[skill]` section: `name = "dev-handsoff"`, `version = "0.1.0"`, `description` per ticket, `always_on = false`.
    - `[triggers]` section: `keywords = [...]` with the six multi-word phrases per ticket. Single-word phrases prohibited (`project_keyword_substring_false_positives`).
    - `[constraints]` section: `required_tools = [...]` — exact list from D3 implementation-discovery (canonical mika-internal tool names; verified against existing bundled skills' surface).
    - No `[lifecycle]` field (that's v0.2 dependency #1).
    - No `dependencies` field (skill is self-contained).
    - No `[output]` field beyond convention (the skill's terminal block is signal-only per D7 of ticket).
- **`system_prompt.md`:**
    - Opens with one-paragraph framing: "dev-handsoff skill v0.1 — write structured handsoff artifact to worktree-local `.mika/handsoff/`. No git ops. No cross-repo writes. v0.2 pipeline pickup transforms artifact → published log."
    - Cites `mika-platform/docs/logs/HANDSOFF-CONTRACT.md` as the canonical body-section spec. Body of the artifact matches HANDSOFF-CONTRACT exactly per D2.
    - Five phases per ticket Phase Outline:
        1. Resolve target file path (worktree root, `<TODAY>=$(date -u +%Y-%m-%d)`, slug fallback chain, mkdir).
        2. New-file create (frontmatter + body template, populated from session synthesis).
        3. Continuation append (read existing → apply per-section discipline → rewrite next-session wholesale → append `runs[]` entry → update top-level `summary`).
        4. Synthesis discipline (zero `AskUserQuestion`; synthesize from session context; operator corrects in follow-up turn).
        5. Emit signal block (Path, Summary, Pickup — exact format per ticket).
    - Body sections (per D2, matching HANDSOFF-CONTRACT verbatim): `## TL;DR`, `## Story so far`, `## Tickets & PRs touched`, `## Blocked / carry-forward`, `## Decisions in flight`, `## Filed today`, `## What to do next session`. Drop empty sections (no `(none)` placeholders).
    - Discipline section near end: writer-scope, artifact-contract-load-bearing, keyword-triggered-not-lifecycle, zero-AskUserQuestion-on-slug-chain, third-person, session-state-not-compounding, no-git, no-umbrella-prose.
    - Voice: third-person throughout ("mika-dev did X").

**Patterns to follow:**
- `mika/skills/bundled/self-dev/skill.toml` for manifest shape (prompt-only, no handlers/tools.json).
- `mika/skills/bundled/self-dev/system_prompt.md` for prompt-body conventions.
- `mika-platform/docs/logs/HANDSOFF-CONTRACT.md` for body-section names + per-section append rules.

**Test scenarios:**
- Test expectation: none for automated CI — bundled skills are prompt+manifest contracts. Verification is the 14-item list below (manual + grep-based).

**Verification (14 items per ticket):**
1. **Manifest parse** — `mika skills validate dev-handsoff` (or canonical equivalent) parses without error.
2. **Listing** — `mika skills list` shows `dev-handsoff`.
3. **Keyword activation positive** — turn with "wrap up the run" activates the skill.
4. **Required-tools sanity** — `grep -h "^required_tools" mika/skills/bundled/*/skill.toml | sort -u` shows the new tool list aligned with deployed surface (D3).
5. **Keyword-conflict grep** — run from cwd `/data/workspace/mika-platform`, the script in ticket Pre-dispatch confirmations §2, must report `CLEAN '<phrase>'` for all six phrases. Silent-zero is the failure mode this guards against.
6. **End-to-end rule 1 (work-item)** — `<worktree>/.mika/handsoff/<TODAY>-<work-item-slug>.md` created with full frontmatter + body sections.
7. **End-to-end rule 3 (first-of-day, no work-item)** — `<TODAY>.md` created (no slug suffix).
8. **End-to-end rule 2 (later-in-day, no work-item)** — `<TODAY>-<sid8>.md` created.
9. **Continuation append** — re-trigger; body sections follow per-section append discipline; frontmatter `runs[]` gains entry; `summary` updates; other top-level fields unchanged.
10. **Frontmatter schema** — every required field from D2 / R5 present and well-typed.
11. **Voice check** — third-person throughout; no first-person leaks.
12. **Phase 5 single-source-of-truth** — terminal block has only Path/Summary/Pickup lines.
13. **Zero-question check** — no `AskUserQuestion` invocation in any run.
14. **No-git check** — no `git add`/`git commit`/`git push`. `git rev-parse --show-toplevel` (read-only) IS expected and required.

## System-Wide Impact

- **Interaction graph:** Three surfaces:
  - Filesystem at `<worktree>/.mika/handsoff/` — writes/extends one file per day per worktree.
  - mika skill loader — parses `skill.toml`, validates against schema.
  - mika-dev's keyword-trigger surface — activates skill on configured phrases.
- **API surface parity:** Two parities to preserve:
  - **HANDSOFF-CONTRACT body-section parity** (D2): the v0.1 artifact body must use HANDSOFF-CONTRACT's section names verbatim. v0.2 pickup is identity transformation of body content.
  - **Operator-Claude `/mika-handsoff` parity** (mika-platform#81, ready-labelled): both consumers of HANDSOFF-CONTRACT must produce body-section-conformant output. Voice differs (operator uses first-person session voice; mika-dev uses third-person), but section names are identical.
- **Unchanged invariants:** This skill does NOT modify `SkillManifest` schema, does NOT add Rust code, does NOT change mika-dev's webhook surfaces, does NOT touch `mika-platform/docs/logs/`. v0.1 is fully additive within the `mika/skills/bundled/` directory.
- **State lifecycle risks:**
  - **Stale artifacts** — files written by v0.1 but never picked up by v0.2 accumulate in `<worktree>/.mika/handsoff/`. v0.2 picks them up in date order; no garbage collection in v0.1.
  - **Filename collisions across days** — impossible by construction (`<TODAY>` prefix on every filename).
  - **Continuation append corruption** — if synthesis truncates an existing section's prior content, the artifact is corrupted. Phase 3 must read entire file, parse sections, and append/update without truncation. The skill's discipline section codifies this.
- **Integration coverage:** Manual + grep + smoke. The 14 verification steps are the integration test set. v0.2 ticket will add automated coverage for the artifact contract at the pickup boundary.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Skill writes outside `<worktree>/.mika/handsoff/` (cross-repo write leak). | Phase 1 explicitly resolves worktree root via `git rev-parse --show-toplevel`. Discipline section codifies "writer-scope is the worktree, period." Verification step #14 grep checks for any cross-repo paths in the prompt. |
| Body section names diverge from HANDSOFF-CONTRACT (D2 reconciliation lost in implementation). | D2 explicitly reconciles section names. system_prompt.md must use HANDSOFF-CONTRACT's verbatim section names. Verification step #10 frontmatter schema check + manual smoke catch divergence. |
| `required_tools` value wrong (D3 unresolved at implementation, defaults to ticket's `["bash", "edit", "write"]`). | D3 explicitly defers to implementer with a tool-registry consultation gate. Verification step #4 grep against deployed bundled skills' surface. If new tool names aren't in the deployed surface, implementer must justify additively. |
| Keyword conflict with existing skill or command (silent activation drift). | Verification step #5 keyword-conflict grep is MANDATORY before commit. Run from `/data/workspace/mika-platform`. Pass criterion: `CLEAN '<phrase>'` for all six. Silent-zero failure mode (grep against non-existent path) explicitly named in ticket Pre-dispatch confirmations. |
| Skill silently performs git ops because LLM rationalizes "I should commit this." | system_prompt.md discipline section codifies "no git ops" as a hard rule. Verification step #14 reviews the prompt for any git-mutation references. The skill prompt should NOT mention `git add`/`commit`/`push` even in error/recovery paths. |
| `AskUserQuestion` budget violation (LLM asks operator for slug). | D6 codifies zero-AskUserQuestion as load-bearing on slug fallback chain. system_prompt.md's Phase 4 "synthesis discipline" section spells this out. Verification step #13 confirms no AskUserQuestion in any run. |
| Continuation append corrupts existing artifact (truncation, section reordering). | Phase 3 reads file end-to-end before mutating. Per-section append discipline is verbatim from D2. Verification step #9 explicitly tests continuation discipline against a populated artifact. |
| YAML frontmatter schema drift (e.g., adds field, renames `slug`). | R5 and D2 pin schema. Verification step #10 confirms all required fields present and well-typed. v0.2 pickup parses frontmatter; schema drift breaks pickup. |

## Documentation / Operational Notes

- This plan IS the implementation documentation for v0.1.
- A v0.1 → v0.2 migration note may be useful but is not required for AC. If shipped, file under `mika/docs/solutions/architecture-patterns/dev-handsoff-v01-artifact-contract-2026-05-06.md` and reference from both v0.2 dependency tickets.
- Optional CLAUDE.md update on `mika-skills/CLAUDE.md` to add bundled-skill-as-artifact-writer pattern (alongside the existing handler-bearing skill pattern). Defer to follow-up if pattern emerges with a second instance.
- Solution-doc compounding the artifact-contract-as-versioned-spec pattern would be useful when v0.2 ships (so v0.2 can cite v0.1's contract as historical record). Defer until v0.2.

## Sources & References

- **Origin ticket:** mika#967
- **Prerequisite (now satisfied):** mika-platform#80 (HANDSOFF-CONTRACT.md), shipped via PR #82
- **Companion ticket:** mika-platform#81 (`/mika-handsoff` operator slash command, groomed and ready-labelled today)
- **Design thread:** 67d85cfa (2026-05-04)
- **Memory:** `feedback_work_in_terms_of_mika.md`, `project_keyword_substring_false_positives.md`, `feedback_orthogonality_flag_semantics.md`
- **Pattern reference:** `mika/skills/bundled/self-dev/` (prompt-only bundled skill structure)
- **Schema reference:** `mika/crates/mika-agent/src/skills/manifest.rs:13–52`, `mika-skills/CLAUDE.md` lines 79–116
