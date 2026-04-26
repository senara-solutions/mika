---
title: "fix(mika-arch): drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS"
type: fix
status: groomed
date: 2026-04-26
origin: senara-solutions/mika#818
depth: shallow
reviewer: Claude Chat (Mika project) — external reviewer per the recursive-self-review carve-out (mika-arch reviews tickets that aren't about her own permission surface; tickets that ARE about her own surface route external)
verdict: GROOMED with 5 small additions applied (D3 rationale, test naming + local array, validation rows-unchanged check, doc-comment positive framing, new Unit 5 PR-description security summary)
---

## Pre-merge sanity check (operator)

Grep for hardcoded count assertions on `MIKA_ARCH_DISABLED_TOOLS.len()` (today's value would be 26 after the fix; prior value 29). Confirmed clean: `grep -rn '\b29\b' crates/mika-agent/src/well_known_agents.rs` returns no matches; `grep -rn 'MIKA_ARCH_DISABLED_TOOLS' crates/` shows three references (line 290 iterator, line 724 contains-check on `send_message`, line 1197 dynamic `.len()` equality), all name-agnostic. No hardcoded count to drift. Safe to land.

## External review summary

Claude Chat (Mika project) approved with five additions, all applied below:
1. D3 rationale extended — auto-migration would need a stale-vs-intentional-drift predicate (cf. `MIKA_DISABLE_AGENT_PROVISIONING`'s purpose); that's separate architecture, not 30 lines.
2. Test renamed `test_mika_arch_disabled_tools_excludes_agent_self_state` (encodes invariant, not current state); test body uses local `agent_self_state_tools` array for one-line append-on-grow.
3. Validation step 3 explicit "rows-unchanged" check with baseline lengths from the 2026-04-26 operator seed — verified-right not assumed-right on data preservation.
4. Doc-comment style: delete "Memory mutations" header (cleanest), fold positively into "Notably allowed" with citation to review-guide § Orthogonality. No inverted "NOT denied" marker (negative-fact comment antipattern — rots when read in isolation).
5. New Unit 5 — PR description with explicit security-envelope summary (4 sections: what changes / what doesn't / why / threat model). Treats the PR description as a load-bearing artifact, not implementation noise.



# fix(mika-arch): drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS

## Overview

Remove `update_core_memory`, `store_fact`, and `update_fact` from the `MIKA_ARCH_DISABLED_TOOLS` const at `crates/mika-agent/src/well_known_agents.rs:222-256` so mika-arch can persist self-state across sessions. ~3 lines removed from the const, 1 test assertion updated, ~5 lines updated in the const's doc comment, 1 CLAUDE.md section updated, 1 manual operator step documented for the existing-mika-arch deploy migration. No new code paths, no schema, no new abstraction.

## Problem Frame

mika-arch's role per spec is "Principal-Engineer-class advisory reviewer" — fundamentally a cross-session pattern-recognition role. Read-only on platform side-effects (no commits, merges, shell exec, code generation) is correct. Read-only on her own self-state is not — it starves her of the substrate the role assumes. Without `update_core_memory`/`store_fact`/`update_fact`, she resets to baseline every session and cannot accumulate operator-pattern recognition, prior-decision recall, or commitment tracking.

Surfaced empirically on 2026-04-26 session `83519e10-…` when she explicitly named the gap mid-FYI: *"I don't have `update_core_memory` or `store_fact` in my available tools for this session. The facts I'd want to persist are..."* and listed 6 facts. Operator (CC) wrote them to the DB by hand as a stopgap; this ticket removes the need for the stopgap.

The denylist's design intent is the **read-only architect contract** from spec §1.2/§2.2. Memory writes were bundled into the broader "mutational tools" category alongside PR merge, skill mutations, task mutations — the bundle was assembled by enumerating mutational tools without separating by *what gets mutated*. Memory writes mutate the agent's own self-state (5 core memory blocks + 4 facts categories, all scoped to `agent_id = 'mika-arch'`); PR merge mutates the platform. Different surface, different blast radius, different category. Bundle was wrong on granularity.

## Requirements Trace

- **R1.** `update_core_memory`, `store_fact`, `update_fact` removed from `MIKA_ARCH_DISABLED_TOOLS`.
- **R2.** All other denied tools preserved (skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations). Read-only-platform-side-effects invariant is unchanged.
- **R3.** Existing test `test_mika_arch_identity_has_tools_disabled_block` updated to assert the inverse for `update_core_memory`.
- **R4.** Doc comment categorizing "Memory mutations" as a denied category removed; "Notably allowed" section expanded to include memory writes with citation to `mika/docs/architecture/review-guide.md` § Orthogonality (the agent self-state vs platform side-effects principle landed in commit `2bba6223`).
- **R5.** `mika/crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)" updated to reflect the corrected scope and cite the review-guide section.
- **R6.** Operator deploy migration step documented: existing mika-arch's `identity.toml` was provisioned with the old denylist baked in; the provisioning path's `agent_exists` short-circuit (`well_known_agents.rs:367-369`) means the new `MIKA_ARCH_DISABLED_TOOLS` won't propagate to existing agents on restart. Operator must either delete the existing identity.toml (forcing re-provision) or manually edit it.

## Scope Boundaries

**In scope:**
- 3 const-array edits at `well_known_agents.rs:224-226`.
- 5-line doc-comment update at `well_known_agents.rs:207-221`.
- 1 test assertion flip at `well_known_agents.rs:716`.
- 1 CLAUDE.md section update at `crates/mika-agent/CLAUDE.md`.
- 1 paragraph documenting the deploy-migration manual step.

**NOT in scope:**
- `[tools].allowlist` migration (the future symmetric well-known-agent shape mentioned in `crates/mika-agent/CLAUDE.md`). Defer — minimal change is dropping three names. Allowlist migration is its own concern when applied symmetrically across all well-known agents.
- Any change to the other denied categories (skill mutations, config writes, etc.).
- Auto-migration logic in `provision_well_known_agents` to update existing identity.toml files. Manual operator step is sufficient for one-time fix; auto-migration is YAGNI until we have multiple in-flight denylist changes.
- Backfill of mika-arch's existing core_memory rows (already done by operator on 2026-04-26 — 5 blocks populated, 9 facts seeded; future writes append).

### Non-goals

- Cross-agent file access for mika-arch (`read_agent_file` with `agent` parameter — orchestrator-only). mika-arch stays advisory.
- Granting mika-arch any platform-mutation tool (run_shell, run_gh, run_claude_pilot, etc.). Those stay denied.
- Changing the per-skill model overrides or the identity allowlist. Those are separate concerns.

## Context & Research

### Relevant code

- **`crates/mika-agent/src/well_known_agents.rs:222-256`** — the `MIKA_ARCH_DISABLED_TOOLS` const. Comments at 199-221 categorize the denied tools by purpose ("Memory mutations", "Skill mutations", "Config / files", "Reminders", "Tasks", "PR mutations", "Cross-agent invocation", "Agent / team mutations"). The "Memory mutations" category contains exactly the three names this ticket removes.
- **`crates/mika-agent/src/well_known_agents.rs:289-293`** — `build_mika_arch_identity` writes the denylist into the rendered `identity.toml` at provision time. No change needed; the rendered toml automatically reflects the corrected const.
- **`crates/mika-agent/src/well_known_agents.rs:716`** — test `test_mika_arch_identity_has_tools_disabled_block` currently asserts `toml.contains("\"update_core_memory\"")`. After the fix, this should assert `!toml.contains("\"update_core_memory\"")`.
- **`crates/mika-agent/src/well_known_agents.rs:1197`** — assertion `assert_eq!(identity.tools.disabled.len(), MIKA_ARCH_DISABLED_TOOLS.len())`. This is computed dynamically against the const; **no change needed** — dropping three names from the const automatically updates both sides of the assertion.
- **`crates/mika-agent/src/agent.rs:3189`** — `apply_agent_tool_visibility()`. The filter that consumes `[tools].disabled` from identity.toml at LLM-tool-array assembly. No change needed — this code is denylist-name-agnostic.
- **`crates/mika-agent/src/prompt.rs:115-120, 252`** — identity loading. Reads `[tools].disabled` from identity.toml. No change needed.

### The deploy migration consideration (R6)

`provision_well_known_agents` at `well_known_agents.rs:354-451` has an `agent_exists` short-circuit (lines 365-369): if `~/.mika/agents/mika-arch/identity.toml` already exists, the function `continue`s and skips re-rendering. This means a fresh deploy of the new const won't propagate to an already-provisioned mika-arch.

**Operator manual step** (post-merge, post-`make deploy`):
1. Edit `~/.mika/agents/mika-arch/identity.toml` directly.
2. In the `[tools].disabled` array, remove the three lines: `"update_core_memory",`, `"store_fact",`, `"update_fact",`.
3. Restart mika-server (`sudo rc-service mika-server restart`).
4. Verify: `mika ask --agent mika-arch "<simple prompt>"` followed by SQL `SELECT * FROM core_memory WHERE agent_id='mika-arch' AND key='self_model'` — confirm mika-arch's memory writes propagate to the table.

Auto-migration would close this gap but adds complexity for a one-time fix. Defer per "Out of scope."

### Patterns this plan follows

- Same shape as the structural read-only enforcement in PR #813 — denylist-as-source-of-truth, applied at LLM-tool-array assembly via `apply_agent_tool_visibility()`. We're shrinking the denylist, not changing the enforcement mechanism.
- The architectural principle this fix embodies (agent self-state vs platform side-effects) is documented in `mika/docs/architecture/review-guide.md` § Orthogonality, commit `2bba6223`. Cite that section in the doc-comment update so future readers don't re-derive.

## Key Technical Decisions

### D1. Drop three names — not refactor to allowlist

The future migration to `[tools].allowlist` (symmetric with the existing `[skills].allowlist` for well-known agents) is mentioned in `crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)" as a separate concern. Wrapping that migration into this fix would: (a) couple a small permission correction with a substantive refactor, (b) require designing the allowlist's relationship to the existing denylist for non-well-known agents (where deny-by-default + allowlist is a different security shape than allowlist-only), (c) bloat the diff for review.

**Decision: minimal change. Drop three names from the existing denylist.** The allowlist migration is its own ticket when there's a structural reason to do it.

### D2. Update the test assertion vs. delete it

Test `test_mika_arch_identity_has_tools_disabled_block` at line 710-719 currently has assertions on three names: `pr_merge_with_gate` (still denied), `a2a_call` (still denied), `update_core_memory` (now allowed). The natural fix is to flip the third assertion to `!toml.contains("\"update_core_memory\"")` rather than delete it — the inverted assertion is a regression guard against future re-bundling.

**Decision: flip to negative assertion + add explanatory comment** linking to review-guide § Orthogonality.

### D3. Manual operator migration vs. auto-migration

`provision_well_known_agents`'s `agent_exists` short-circuit means the const change won't propagate to existing mika-arch installs. Three options:

- **Manual operator edit + restart** (this plan's choice). Operator follows the documented step. ~30 seconds.
- **Auto-migration in `provision_well_known_agents`:** read existing identity.toml, compare `[tools].disabled` against `MIKA_ARCH_DISABLED_TOOLS`, write back if drift detected. ~30 lines + tests for the drift-detection logic. Useful long-term if denylist changes become frequent.
- **Force-overwrite identity.toml on every restart:** would also overwrite operator-customized identity files. Wrong default.

**Decision: manual for v1.** The denylist is unlikely to change frequently. Auto-migration is YAGNI.

**Why auto-migration is more than 30 lines (peer-review addition 2026-04-26):** an auto-migration mechanism would need to distinguish *stale drift* (the operator hasn't updated yet) from *intentional drift* (the operator has customized identity.toml deliberately and provisioning shouldn't clobber it). That's a separate architectural question — `MIKA_DISABLE_AGENT_PROVISIONING` exists precisely to handle the "intentional drift" case for the broader agent-provisioning surface. Designing a "is this drift intentional or stale?" predicate is real architecture, not 30 lines of drift-detection code. The clean rule for future: if the same denylist edit pattern recurs within a quarter, that's the signal to invest. Until then, manual operator step is correct YAGNI.

### D4. Documentation update target

The principle this fix embodies is captured in `mika/docs/architecture/review-guide.md` § Orthogonality (commit `2bba6223`). Two places need to point at it:

- The doc-comment on `MIKA_ARCH_DISABLED_TOOLS` (well_known_agents.rs:199-221): add a sentence in the "Notably allowed" section explaining memory writes are agent-scoped self-state per the orthogonality principle, with a path reference.
- `mika/crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)": update the description to no longer claim "memory writes" are denied, and add a parenthetical pointing at the review-guide.

**Decision: both updates, both citing the same review-guide path.** Avoids the rule-stated-twice problem.

## Open Questions

### Resolved during planning

- Drop names vs. switch to allowlist → D1 (drop names).
- Test fix shape → D2 (flip assertion).
- Migration shape → D3 (manual).
- Doc citations → D4 (both updates).

### Deferred to implementation

- Exact wording of the doc-comment paragraph and the CLAUDE.md update. Should match the review-guide § Orthogonality phrasing without verbatim duplication.
- Whether the operator manual-migration step should be a script in `mika-platform/scripts/` (e.g., `migrate-mika-arch-denylist.sh`) or just inline shell commands in the deploy notes. Lean inline — one-time fix, scripting adds maintenance debt.

## Output Structure

```
mika/
├── crates/mika-agent/
│   ├── src/well_known_agents.rs              # MODIFY — drop 3 names + update doc comment + flip test
│   └── CLAUDE.md                             # MODIFY — update denylist § scope description
└── docs/plans/
    └── 2026-04-26-002-fix-mika-arch-memory-tools-plan.md  # this file
```

No new files. No new tests beyond the assertion flip.

## Implementation Units

- [ ] **Unit 1: Const edit + doc comment**

**Goal:** Remove the three names + update the doc comment to remove "Memory mutations" as a denied category and fold the same information into "Notably allowed" as a positive fact (peer-review refinement 2026-04-26 — negative-fact comments rot; positive facts persist).

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs:222-256` — drop lines 223-226 (the `// Memory mutations` comment and the three names). Cleanest deletion; no inverted "NOT denied" marker (negative-fact comments are documentation antipattern — rot when read in isolation).
- Modify: `crates/mika-agent/src/well_known_agents.rs:199-221` — drop "Memory mutations" from the Categories list (line 208); rewrite the "Notably allowed" section to include memory writes as a positive fact:
  > *"Notably allowed: `send_message` (mika-arch needs to deliver verdicts to whoever asked for the review), `update_core_memory`/`store_fact`/`update_fact` (memory writes are agent-scoped self-state — constitutive of being an agent, not platform side-effects; see `docs/architecture/review-guide.md` § Orthogonality)."*

The positive framing locates exception-information in the section that's already supposed to hold exception-information; the structural test (Unit 1) enforces the invariant at the const level so the comment doesn't need to.

**Tests:** Existing `test_mika_arch_disabled_tools_does_not_include_send_message` (line 721-728) covers the structural invariant. Add a parallel test, **named to encode the invariant** (not the current state — peer-review addition 2026-04-26):

```rust
#[test]
fn test_mika_arch_disabled_tools_excludes_agent_self_state() {
    // Memory writes mutate the agent's own self-state (5 core memory blocks
    // + 4 facts categories, scoped to agent_id='mika-arch'). They are
    // constitutive of being an agent, not platform side-effects.
    // See mika/docs/architecture/review-guide.md § Orthogonality.
    let agent_self_state_tools = ["update_core_memory", "store_fact", "update_fact"];
    for tool in &agent_self_state_tools {
        assert!(
            !MIKA_ARCH_DISABLED_TOOLS.contains(tool),
            "{tool} must remain visible to mika-arch (agent self-state, not platform side-effect)"
        );
    }
}
```

The test name encodes the invariant (agent self-state must not be denied) rather than the current state (memory writes specifically). When future agent-self-state tools surface (e.g., `update_self_summary`), adding a name to the local array is a one-line append — no rename, no refactor. Local array is the structure; no separate const, no premature abstraction.

This is the regression guard that prevents future re-bundling.

- [ ] **Unit 2: Flip the existing test assertion**

**Goal:** Update `test_mika_arch_identity_has_tools_disabled_block` (well_known_agents.rs:710-719) to assert the *absence* of `update_core_memory` from the rendered toml.

**Approach:** Change line 716 from:
```rust
assert!(toml.contains("\"update_core_memory\""));
```
to:
```rust
// Memory writes are NOT denied — agent-scoped self-state, not platform side-effect.
// See review-guide.md § Orthogonality (commit 2bba6223).
assert!(!toml.contains("\"update_core_memory\""));
```

The inverted assertion is the regression guard at the rendered-toml level. Unit 1's new test guards at the const level. Defense in depth.

- [ ] **Unit 3: CLAUDE.md update**

**Goal:** Update `crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)" to reflect the corrected scope.

**Approach:** Find the paragraph saying "denies the full mutational built-in tool set (memory writes, skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations) while keeping `send_message` allowed." Update to: remove "memory writes" from the denied list, expand "Notably allowed" to include memory writes, add a parenthetical citing review-guide § Orthogonality.

- [ ] **Unit 4: Deploy migration documentation**

**Goal:** Document the manual operator step required for existing mika-arch installs.

**Approach:** Add a paragraph to the PR description (and to `mika/CLAUDE.md` § "Local Dev Environment" if appropriate) explaining:

> **Existing mika-arch deploy migration:** This change updates `MIKA_ARCH_DISABLED_TOOLS` but does not auto-migrate existing `~/.mika/agents/mika-arch/identity.toml` files (the provisioning path's `agent_exists` short-circuit prevents re-rendering). After deploy: edit `~/.mika/agents/mika-arch/identity.toml`, remove the lines `"update_core_memory",`, `"store_fact",`, `"update_fact",` from the `[tools].disabled` array, restart mika-server. Verify with: `mika ask --agent mika-arch "<test prompt>"` followed by `sqlite3 ~/.mika/data/mika.db "SELECT key, length(value) FROM core_memory WHERE agent_id='mika-arch'"`.

No code change for Unit 4 — pure documentation.

- [ ] **Unit 5: PR description with explicit security-envelope summary** (peer-review addition 2026-04-26)

**Goal:** The PR description is the durable artifact that lives next to the diff. For a change that relaxes the security envelope, the threat-model summary must live with the diff — not buried in working notes, this plan, the review-guide, or the friend-review chat that produced the decision. Future-CC, future-Vincent, or anyone auditing the security envelope a year from now should get the threat model in one place by reading the PR description.

**PR description must include four explicit sections:**

1. **What's changing.** Three tools (`update_core_memory`, `store_fact`, `update_fact`) become available to mika-arch via removal from `MIKA_ARCH_DISABLED_TOOLS`.
2. **What's NOT changing.** Read-only-platform invariant unchanged. All platform-mutation tools (run_shell, run_gh, run_claude_pilot, pr_merge_with_gate, set_config, write_agent_file) remain denied. All cross-agent tools (a2a_call, delegate_task, run_team) remain denied. All agent/team mutation tools (create_agent, create_team, etc.) remain denied. All skill mutation tools remain denied. All task mutation tools remain denied.
3. **Why.** Memory writes mutate the agent's own self-state (`agent_id='mika-arch'`-scoped core_memory + facts tables); they are persistence, not side-effect. The original denylist bundled them with platform mutations because it enumerated mutational tools without separating *what gets mutated*. Cite `mika/docs/architecture/review-guide.md` § Orthogonality (commit `2bba6223`) for the principle.
4. **Threat model — worst case.** Prompt-injection vector: attacker-controlled content (issue body, PR description, file fetched via gh_read) instructs mika-arch to update her own self_model in some adversarial way. **Worst case:** garbage in mika-arch's own core memory blocks for her next session. **Recoverable:** operator can clean up via direct DB writes (already demonstrated in the operator-seed path 2026-04-26). **Bounded:** writes are agent-scoped; cannot escape to platform state, other agents, the codebase, or external systems. **Same vector exists today** for mika-dev and mika-qa with far more dangerous tools (PR merge, run_claude_pilot); the structural defense is the agent's grounding (system prompt, role instructions, citation discipline), not tool denial.

**Files:** none — pure PR-description content. Capture in the PR template or in the body when opening the PR.

**Why this is its own unit:** the PR description is a load-bearing artifact for security-envelope changes specifically. Treating it as "implementation noise" loses the audit trail. Treating it as a unit puts it on the checklist with the same weight as the code changes.

## Acceptance criteria

- [ ] `cargo test -p mika-agent well_known_agents` passes (existing tests + the new memory-writes regression test).
- [ ] `cargo clippy -p mika-agent` clean.
- [ ] `cargo fmt --check` clean.
- [ ] `MIKA_ARCH_DISABLED_TOOLS.len()` is 26 (was 29).
- [ ] `crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)" reflects the corrected scope and cites review-guide § Orthogonality.
- [ ] Manual deploy-migration step documented (PR description + verification commands).

## Validation post-deploy

After PR merges + `make deploy` + manual identity.toml edit + restart:

1. `mika ask --agent mika-arch "Use update_core_memory to update your self_model block to confirm tool availability."`
2. Verify the call succeeded via `tool_calls` table:
   ```sql
   SELECT tool_name, success FROM tool_calls
     WHERE session_id IN (SELECT id FROM sessions WHERE agent_id='mika-arch' ORDER BY started_at DESC LIMIT 1)
     AND tool_name IN ('update_core_memory', 'store_fact', 'update_fact');
   ```
3. Verify the operator-seeded core_memory rows from 2026-04-26 are still present **and unchanged** — explicit regression check, not just presence (peer-review refinement 2026-04-26):
   ```sql
   SELECT key, length(value), token_count FROM core_memory WHERE agent_id='mika-arch' ORDER BY key;
   ```
   Expected: 5 rows with these exact lengths (recorded post-seed 2026-04-26):
   - `current_priorities`: 1060 chars, ~265 tokens
   - `key_people`: 182 chars, ~45 tokens
   - `self_model`: 1278 chars, ~319 tokens
   - `user_summary`: 783 chars, ~195 tokens
   - `workflows`: 1195 chars, ~298 tokens

   If lengths differ from these baselines (and mika-arch hasn't been invoked between seed and verification), `apply_overrides` or `apply_agent_tool_visibility` is silently mutating data — investigate before proceeding. Lengths matching baseline = init path is data-clean (which the source-read confirms it is, but verified-right ≠ assumed-right).

4. Sanity check that platform-mutation tools are still denied:
   ```
   mika ask --agent mika-arch "Use pr_merge_with_gate on PR #1."
   ```
   Expected: tool not available; mika-arch reports the denial.

## Related

- senara-solutions/mika#818 — this ticket.
- senara-solutions/mika#811 / PR #813 — introduced `MIKA_ARCH_DISABLED_TOOLS` with the original (over-broad) bundle.
- `mika/docs/architecture/review-guide.md` § Orthogonality (commit `2bba6223`) — the principle this fix embodies (agent self-state vs platform side-effects).
- mika-arch session `83519e10-…` — where mika-arch surfaced the gap.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — companion compound from the dogfood that surfaced model-calibration concerns; same agent.
- `project_decisions_in_flight.md` (auto-memory) — operator-side state tracking the durable-seed deferral and the uniform-allowlist deferral, both still pending post-#817 dogfood validation.
