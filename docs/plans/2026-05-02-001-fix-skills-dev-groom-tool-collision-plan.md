---
title: "fix(skills): consolidate run_claude_pilot to single skill to unblock dev-groom dispatch"
type: fix
status: active
date: 2026-05-02
origin: senara-solutions/mika#932
---

# fix(skills): consolidate run_claude_pilot to single skill to unblock dev-groom dispatch

## Spec Deviation Ratified (path a, 2026-05-02)

mika-arch first-pass review (session `5db0ffe0-89ab-4e1c-ba39-3f70785412ab`) flagged that Unit 2 modifies `mika/skills/bundled/self-dev/system_prompt.md`, which the issue body's `## Out of Scope` section originally listed as out-of-scope (*"Any change to mika-arch or self-dev system prompts"*). Operator ratified the scope expansion: self-dev/system_prompt.md is in scope and is required for the fix to land — without it, Unit 1's enum widening ships dead code (mika-dev would never call `run_claude_pilot` with `skill="dev-groom"`).

The issue body has been amended (Out-of-Scope item updated to exempt `self-dev/system_prompt.md`). Ratification recorded in [comment-4363607424](https://github.com/senara-solutions/mika/issues/932#issuecomment-4363607424) on mika#932.

## Overview

`dev-pilot/tools.json` and `dev-groom/tools.json` both register a tool named `run_claude_pilot` with conflicting `skill` enum schemas. The engine collapses duplicate tool-name registrations; only one survives. mika-dev calls `run_claude_pilot(skill="dev-groom", ...)`, the surviving schema rejects it, dispatch fails.

dev-groom is a sub-skill of self-dev, peer to dev-pilot. Both dispatch through the same `run_claude_pilot` mechanism, differing only in entry command (`/mika` vs `/mika-groom-ticket`).

Fix: single tool, union enum, handler-side dispatch on `$SKILL`. Two units.

## Problem Frame

Two skills cannot register the same exec-tool name with conflicting schemas. The shipped engine's tool-registration layer collapses duplicates instead of namespacing them, and `dev-pilot`'s variant wins. Confirmed live on session `bc1de55b-5454-4ad0-9501-67e2eef6fa99` at 2026-05-02T09:59:21, blocking task `c6af3d36`. Engine error: `Skill 'dev-groom' is not a valid skill. Valid values: ["dev-pilot"]`.

The shared library `mika/skills/bundled/_shared/dispatch-lib.sh` already extracts `SKILL` from input JSON at line 106. The fix consolidates the tool definition into one host skill (dev-pilot) with a union enum, moves the skill→entry-command mapping into the shared lib, and teaches self-dev's system prompt to dispatch dev-groom directly.

## Requirements Trace

- **R1** — `mika ask --agent mika-dev "groom <repo>#<n>"` results in mika-dev calling `run_claude_pilot(skill="dev-groom", ...)`, the schema accepts it, the lib resolves `/mika-groom-ticket`, the dispatched session runs.
- **R2** — DB query `SELECT json_extract(input, '$.skill') FROM tool_calls WHERE tool_name='run_claude_pilot' ORDER BY created_at DESC LIMIT 1;` returns `"dev-groom"` for a successful invocation (success=1, no schema rejection in output).
- **R3** — Existing `mika ask --agent mika-dev "implement <repo>#<n>"` flow continues to work unchanged: the `dev-pilot` skill value still routes to `/mika`. No regression on implementation dispatches.

## Scope Boundaries

- **In scope**: `mika/skills/bundled/dev-pilot/`, `mika/skills/bundled/dev-groom/`, `mika/skills/bundled/_shared/dispatch-lib.sh`, `mika/skills/bundled/self-dev/system_prompt.md`. Possibly `mika/CLAUDE.md` and `mika/docs/` if they document the per-skill `tools.json` pattern (verify via grep during implementation).
- **Out of scope**:
  - `dependencies = ["dev-pilot"]` on dev-groom's `skill.toml`. self-dev's prompt directly teaches both dispatch targets; no skill-dependency pull mechanism needed.
  - Engine code changes (no Rust edits).
  - DB schema changes. CLI surface changes.
  - Sprint_id stamping (mika-platform#73). PR-writer plan-doc citation (mika#931).

## Context & Research

### Relevant Code and Patterns

- `mika/skills/bundled/dev-pilot/tools.json` — current `run_claude_pilot` definition with `enum: ["dev-pilot"]`. Becomes the union-enum host.
- `mika/skills/bundled/dev-groom/tools.json` — current `run_claude_pilot` definition with `enum: ["dev-groom"]`. Deleted.
- `mika/skills/bundled/dev-pilot/handlers/run.sh` — current handler, hardcodes `/mika`. Updated to call `dispatch_claude_pilot` without an entry-command argument.
- `mika/skills/bundled/dev-groom/handlers/run.sh` — current handler, hardcodes `/mika-groom-ticket`. Deleted along with its `handlers/` directory.
- `mika/skills/bundled/_shared/dispatch-lib.sh` — already extracts `SKILL` from input JSON at line 106. Gains a `case` switch that maps `dev-pilot → /mika`, `dev-groom → /mika-groom-ticket`. Header doc at lines ~12-16 updated.
- `mika/skills/bundled/self-dev/system_prompt.md` — gains a parallel block teaching mika-dev when to call `run_claude_pilot(skill="dev-groom", ...)`, mirroring the wording and parameter contract of the existing dev-pilot dispatch instructions.
- `mika/skills/bundled/dev-groom/skill.toml` — unchanged (keywords intact for activation).
- `mika/skills/bundled/dev-groom/system_prompt.md` — unchanged (instructions for what grooming work entails inside the dispatched session).

### Institutional Learnings

- mika#893 — original consolidation that introduced `_shared/dispatch-lib.sh`. The pattern is established; this plan extends it to also consolidate the tool definition and the skill→command mapping.
- mika#923 (p0, separate ticket, open) — `mika skills update` doesn't propagate `_shared/` and stale per-skill files don't get cleaned up. Drives the post-deploy manual cleanup step below.

### External References

None — purely an internal skill-registration fix.

## Key Technical Decisions

- **dev-pilot is the host skill** for the consolidated `run_claude_pilot` tool. It's the existing owner; lower disruption.
- **dev-groom becomes prompt-only** (`skill.toml` + `system_prompt.md` only). No `tools.json`, no `handlers/`. Its keywords still trigger and its prompt content still appears in the turn.
- **Skill→entry-command mapping lives in `_shared/dispatch-lib.sh`**, not in per-skill handlers. The lib already owns input parsing; centralizing the mapping there keeps handlers minimal and prevents drift across siblings.
- **No skill-dependency pull on dev-groom.** self-dev (always_on) is the parent of both dev-pilot and dev-groom; self-dev's prompt directly teaches mika-dev when to dispatch with each skill value. The dependency-pull mechanism is unnecessary and was rejected during planning review.
- **Handler signature change.** `dispatch_claude_pilot` no longer takes an entry-command argument; the lib derives it from `$SKILL`. Existing callers (dev-pilot's handler) drop the argument.

## Open Questions

### Resolved During Planning

- **Q: Where does the consolidated tool live?** A: `dev-pilot/tools.json`.
- **Q: How does the handler get `$SKILL`?** A: `_shared/dispatch-lib.sh` already parses it (line 106) and now also owns the skill→command mapping. Handler does not need to know about it.
- **Q: How does mika-dev know to call `run_claude_pilot(skill="dev-groom")` for grooming work?** A: self-dev's system prompt teaches it directly (Unit 2). No dependency-pull, no keyword chaining.

### Deferred to Implementation

- **Q: Does `mika/CLAUDE.md` or any file under `mika/docs/` document the per-skill `tools.json` pattern?** Resolve via `grep -rn "tools.json" mika/CLAUDE.md mika/docs/` during implementation. If yes, update to reflect the host/sibling pattern from Unit 1's lib-doc rewrite. If no, no docs change beyond the lib header.

## Findings (Pass-1 Architect Review, 2026-05-02)

Architect first-pass session: `5db0ffe0-89ab-4e1c-ba39-3f70785412ab`. Six findings produced; resolutions below.

### F1 — Self-dev scope expansion (BLOCKING → RATIFIED)

Architect flagged that Unit 2 modifies `self-dev/system_prompt.md`, contradicting the issue's original Out-of-Scope list. Resolved via path (a): operator ratified the scope expansion. Issue body amended; ratification recorded in [comment-4363607424](https://github.com/senara-solutions/mika/issues/932#issuecomment-4363607424). See "Spec Deviation Ratified" block at the top of this plan.

### F2 — Split Unit 1 into 1a/1b (BLOCKING → REJECTED)

**Considered and rejected.** Revert-atomicity insurance for a doc-and-config change is not justified: any realistic revert is a full PR revert, not a surgical revert of one commit inside the PR. Splitting adds ceremony without paying out in any realistic scenario. Unit 1 stays as a single commit.

### F3 — `dispatch_claude_pilot` callsite verification (BLOCKING → APPLIED)

Widened the callsite grep across all repos in the workspace:

```
grep -rn 'dispatch_claude_pilot\|dispatch-lib\.sh' \
  mika/ mika-platform/ mika-cloud/ mika-skills/ claude-pilot-py/
```

Results:
- `mika/skills/bundled/dev-pilot/handlers/run.sh:7` — `dispatch_claude_pilot "/mika"` (in scope; modified by this plan)
- `mika/skills/bundled/dev-groom/handlers/run.sh:7` — `dispatch_claude_pilot "/mika-groom-ticket"` (in scope; deleted by this plan)
- `mika/skills/bundled/_shared/dispatch-lib.sh:398` — function definition (in scope; modified by this plan)
- `mika/skills/bundled/_shared/dispatch-lib.sh:4,14` — header doc references (in scope; updated by this plan)
- `mika/scripts/test-dispatch-symmetry.sh` — references the lib path in shellcheck/sourcing context but does NOT call `dispatch_claude_pilot`. Verified via `grep -nE "dispatch_claude_pilot|case.*skill" mika/scripts/test-dispatch-symmetry.sh` returning no matches.
- `mika-platform/`, `mika-cloud/`, `mika-skills/`, `claude-pilot-py/` — zero callsites.

**Conclusion:** No external callers. The `dispatch_claude_pilot` API change (loses first positional arg) is safe to ship; only the two handlers in scope use it.

### F4 — Sentinel comment with refactor threshold (BLOCKING → APPLIED)

Sentinel comment block added above the `case` switch in `_shared/dispatch-lib.sh`. See Unit 1 approach. Threshold pinned at N>5 siblings → escalate to Option C ticket.

### F5 — Cleanup command in Operational Notes (SHARPENING → APPLIED)

Cleanup command moved into the Operational Notes section as a copy-pasteable block (not just PR description). See below.

### F6 — Shell-level unit test for `case` switch (SHARPENING → CONFIRMED NOT ADDED)

No shell-level test added. Shell-test infrastructure does not exist for `_shared/dispatch-lib.sh`; adding one for a single arm is scope creep. The unknown-skill `* → exit 1` arm is defense-in-depth — primary guard is engine-layer enum validation in `dev-pilot/tools.json`. Live dispatch with each known skill value verifies the happy paths (Unit 1 + Unit 2 test scenarios).

## Implementation Units

- [ ] **Unit 1: Consolidate `run_claude_pilot` into dev-pilot, move mapping to shared lib**

**Goal:** Single source of truth for `run_claude_pilot`. Schema accepts `["dev-pilot", "dev-groom"]`. Skill→entry-command mapping lives in `_shared/dispatch-lib.sh`. Per-skill handlers are minimal.

**Requirements:** R1, R3.

**Dependencies:** None.

**Files:**
- Modify: `mika/skills/bundled/dev-pilot/tools.json`
- Delete: `mika/skills/bundled/dev-groom/tools.json`
- Delete: `mika/skills/bundled/dev-groom/handlers/run.sh` and the `mika/skills/bundled/dev-groom/handlers/` directory
- Modify: `mika/skills/bundled/_shared/dispatch-lib.sh`
- Modify: `mika/skills/bundled/dev-pilot/handlers/run.sh`

**Approach:**
- In `dev-pilot/tools.json`:
  - Widen `skill.enum` from `["dev-pilot"]` to `["dev-pilot", "dev-groom"]`.
  - Update the tool-level `description` and the `skill` parameter `description` to reflect dual-purpose dispatch (implementation work via `dev-pilot`, grooming work via `dev-groom`).
  - Update the `prompt` field's description to mention both entry commands.
- Delete `dev-groom/tools.json` entirely.
- Delete `dev-groom/handlers/run.sh`, then `rmdir mika/skills/bundled/dev-groom/handlers`.
- In `_shared/dispatch-lib.sh`:
  - Inside `dispatch_claude_pilot`, after `$SKILL` is parsed (line 106 region), add a `case` switch that derives `ENTRY_COMMAND`. Prefix with the sentinel comment block (architect F4):
    ```sh
    # SIBLING SKILL DISPATCH MAPPING (mika#932)
    # Each arm maps a sibling skill to its slash-command entry point.
    # Adding a sibling requires:
    #   1. Add a new arm here.
    #   2. Widen dev-pilot/tools.json `skill.enum` to include the new value.
    #   3. Update self-dev/system_prompt.md to teach mika-dev when to dispatch.
    # Threshold for refactor: if N>5 siblings, escalate to skill-scoped tool
    # registries (Option C from mika#932). Until then, this case switch is
    # the contract.
    case "$SKILL" in
      dev-pilot)  ENTRY_COMMAND="/mika" ;;
      dev-groom)  ENTRY_COMMAND="/mika-groom-ticket" ;;
      *) echo "Unknown skill: $SKILL" >&2; exit 1 ;;
    esac
    ```
  - Replace any prior use of the function's first positional argument (the entry command from the caller) with `$ENTRY_COMMAND`.
  - Update the header doc (lines ~12-16). Replace the obsolete "Adding a third sibling skill ... A tools.json with the run_claude_pilot entry (skill enum set to the new skill name)" with: *"One host skill (dev-pilot) owns the `run_claude_pilot` tool with a union-enum `skill` parameter. Sibling skills in the self-dev family extend the enum and add a case in this lib's skill→entry mapping. Sibling skills do not register their own `run_claude_pilot`."* Cross-reference mika#932.
- In `dev-pilot/handlers/run.sh`:
  - Remove the `/mika` argument from the `dispatch_claude_pilot` call. The handler now reads `dispatch_claude_pilot` (no args).

**Patterns to follow:**
- Existing JSON Schema shape in `dev-pilot/tools.json` — preserve all other fields (`task_id`, `iteration_context`, `long_running`, `estimated_duration_secs`).
- Existing minimalism of `dev-pilot/handlers/run.sh` (~6 lines, sources lib, calls one function). Preserve that.

**Test scenarios:**
- Happy path: `cargo test` passes (existing tests that load bundled skills still work).
- Happy path: `cargo clippy --all-targets --all-features -- -D warnings` clean.
- Edge case: `case` switch rejects an unknown `$SKILL` value with exit 1 and a clear stderr message. Verify by direct shell invocation: `SKILL=other bash -c 'source mika/skills/bundled/_shared/dispatch-lib.sh; ...'` style.
- Integration (live, post-deploy): `mika ask --agent mika-dev "groom mika-platform#73"` produces a `run_claude_pilot` tool call with `success=1` and `json_extract(input, '$.skill')='dev-groom'`. Per-DB:
  ```sql
  SELECT id, tool_name, success, json_extract(input, '$.skill') AS skill
  FROM tool_calls WHERE tool_name='run_claude_pilot' ORDER BY created_at DESC LIMIT 3;
  ```
- Regression (live, post-deploy): `mika ask --agent mika-dev "implement <repo>#<n>"` continues to dispatch with `skill="dev-pilot"` and routes to `/mika`. No regression on the implementation flow.

**Verification:**
- `cat mika/skills/bundled/dev-pilot/tools.json | jq '.[0].input_schema.properties.skill.enum'` returns `["dev-pilot","dev-groom"]`.
- `ls mika/skills/bundled/dev-groom/tools.json` returns "No such file or directory".
- `ls mika/skills/bundled/dev-groom/handlers/` returns "No such file or directory".
- `grep -A6 "Adding a third sibling skill" mika/skills/bundled/_shared/dispatch-lib.sh` shows the new guidance.
- `grep dispatch_claude_pilot mika/skills/bundled/dev-pilot/handlers/run.sh` shows no entry-command argument.

---

- [ ] **Unit 2: Teach self-dev to dispatch dev-groom**

**Goal:** mika-dev knows when to call `run_claude_pilot(skill="dev-groom", ...)` for grooming work, mirroring the existing dev-pilot dispatch instructions.

**Requirements:** R1.

**Dependencies:** Unit 1 (consolidated tool schema must accept `dev-groom`).

**Files:**
- Modify: `mika/skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Locate the existing instructions in `self-dev/system_prompt.md` that teach mika-dev when to call `run_claude_pilot(skill="dev-pilot", ...)`. Add a parallel block for `skill="dev-groom"` covering grooming work.
- Mirror the wording and parameter contract of the existing dev-pilot dispatch instructions. Keep the existing structure; do not rewrite. Add, don't replace.

**Patterns to follow:**
- Existing dev-pilot dispatch block in `self-dev/system_prompt.md`. The grooming block should be structurally identical, only differing in the `skill` value, the trigger conditions (when to groom vs implement), and any grooming-specific prompt text.

**Test scenarios:**
- Integration (live, post-deploy): a "groom <repo>#<n>" prompt to mika-dev results in a `run_claude_pilot(skill="dev-groom", ...)` tool call. Same DB query as Unit 1 confirms.
- Regression (live, post-deploy): an "implement <repo>#<n>" prompt to mika-dev still results in `run_claude_pilot(skill="dev-pilot", ...)`. Mika-dev does not confuse the two.

**Verification:**
- `grep -c 'skill="dev-groom"' mika/skills/bundled/self-dev/system_prompt.md` returns ≥1.
- Live test post-deploy per the integration scenario above.

## System-Wide Impact

- **Interaction graph:** mika-dev receives "groom" prompt → self-dev's system prompt instructs the dev-groom dispatch path → mika-dev calls `run_claude_pilot(skill="dev-groom", ...)` → schema accepts it → handler invokes `dispatch_claude_pilot` (no args) → lib parses input, derives `ENTRY_COMMAND="/mika-groom-ticket"` from the `case` switch → claude-pilot child runs grooming pipeline → callback to mika-dev. The chain has one new mapping (skill→command) in the lib and one new prompt block in self-dev. Both in well-trodden code paths.
- **Error propagation:** Unknown `$SKILL` value → lib's `case` exits 1 → exit trap captures stderr tail → callback delivers HANDLER CRASH with the unknown-skill message → mika-dev sets task to `blocked` and notifies. Matches the current rejection path's UX with a clearer message.
- **State lifecycle risks:** None. No DB schema changes, no migration, no on-disk state changes. Worktree creation is unchanged.
- **API surface parity:** `run_claude_pilot` tool remains the public surface for both dispatch flows. Only its `skill` enum widens — additive, backwards-compatible.
- **Integration coverage:** Cross-layer scenario worth confirming live: the integration scenarios in Unit 1 and Unit 2's test sections. Unit-test-on-schema doesn't prove this; only live invocation does.
- **Unchanged invariants:** `_shared/dispatch-lib.sh` core logic (worktree setup, env scrubbing, exit trap, callback delivery, JSON parsing of stdin) is unchanged. Only the skill→command mapping is added. The `dispatch_claude_pilot` function signature changes (loses its first positional arg) — handlers update accordingly.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `dispatch_claude_pilot` API change (loses entry-command arg) breaks any caller outside dev-pilot's handler | Verified scope: the only existing callers were dev-pilot and dev-groom handlers, both updated/deleted in this plan. `grep -rn dispatch_claude_pilot mika/skills/` during implementation to confirm no third caller. |
| Bundled skill caching (`mika skills update`) doesn't propagate the deletion of `dev-groom/tools.json` and `dev-groom/handlers/` | This is mika#923 (already filed, p0). Document the manual post-deploy cleanup in the PR description: `rm -rf ~/.mika/agents/<agent>/skills/dev-groom/{tools.json,handlers}`. CI/test envs running against fresh `~/.mika/agents/` are unaffected. |
| Regression on existing `/mika` dispatch path | Unit 1 test scenarios include explicit regression check via DB query. Run live before declaring done. |

## Documentation / Operational Notes

### Post-deploy cleanup (until mika#923 ships)

For each agent that previously had dev-groom installed, run the following to remove stale registration files left behind by `mika skills update`:

```sh
rm -rf ~/.mika/agents/<agent>/skills/dev-groom/{tools.json,handlers}
```

Affected agents at deploy time: `mika-dev` (verified). Enumerate any others post-deploy via `ls ~/.mika/agents/*/skills/dev-groom/`. After mika#923 ships, this cleanup becomes automatic.

### Other notes

- After PR ships and `make deploy` runs, manually verify the deployed dev-groom directory contains only `skill.toml` + `system_prompt.md`: `ls ~/.mika/agents/mika-dev/skills/dev-groom/`.
- Document the manual cleanup as a one-time operator step in the PR description (in addition to the copy-pasteable block above).
- No rollout flag, no migration, no monitoring change.
- If `grep -rn "tools.json" mika/CLAUDE.md mika/docs/` reveals references to the per-skill `tools.json` pattern, update to reflect the host/sibling shape.

## Sources & References

- **Origin issue:** [senara-solutions/mika#932](https://github.com/senara-solutions/mika/issues/932)
- Current tool definitions: `mika/skills/bundled/dev-pilot/tools.json`, `mika/skills/bundled/dev-groom/tools.json`
- Current handlers: `mika/skills/bundled/dev-pilot/handlers/run.sh`, `mika/skills/bundled/dev-groom/handlers/run.sh`
- Shared dispatch library: `mika/skills/bundled/_shared/dispatch-lib.sh` (line 106 reads `SKILL`; lines ~12-16 contain obsolete guidance)
- Originating consolidation: mika#893 (introduced `_shared/dispatch-lib.sh`)
- Live failure: session `bc1de55b-5454-4ad0-9501-67e2eef6fa99`, task `c6af3d36`, 2026-05-02T09:59:21
- Related open follow-ups: mika#923 (skill install path doesn't propagate `_shared/`), mika-platform#73 (sprint_id stamping), mika#931 (PR-writer plan-doc citation)
