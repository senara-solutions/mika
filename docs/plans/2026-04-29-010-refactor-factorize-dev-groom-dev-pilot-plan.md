---
title: "refactor(skills): factorize dev-groom and dev-pilot — same plumbing, only differ on entry slash command"
type: refactor
status: draft
date: 2026-04-29
issue: 893
---

# refactor(skills): factorize dev-groom and dev-pilot

## Why

dev-groom and dev-pilot are conceptually the same dispatch shape: both wrap a headless Claude Code session via claude-pilot, both create/reuse a worktree on a derived branch, both pass operator-supplied context as the entry prompt. **The only meaningful difference is the slash command claude-pilot enters with**:

- dev-pilot → `/mika`
- dev-groom → `/mika-groom-ticket`

Today they're maintained as independent copies. The divergence has produced concrete drift:
- dev-groom's `tools.json` is empty `[]` (vs dev-pilot's full schema).
- dev-groom's `handlers/run.sh` is 133 LOC (vs dev-pilot's 441 LOC); slug derivation is inline + drifts from centralized `scripts/derive-branch-name` (filed as **mika#892**, regression of mika-platform#58).
- Empirical evidence: today's mika#886 dev-groom run produced a slug that diverged from the parallel `/mika-groom-ticket` session's slug for the same ticket.

This refactor closes mika#892 by construction and prevents future divergence drift.

## Goal

After this refactor:
- The only files that differ between dev-pilot and dev-groom are `skill.toml` (manifest) and `system_prompt.md` (per-skill prompt).
- `tools.json` and `handlers/run.sh` are shared — one schema, one script, parameterized by `skill` field.
- Slug derivation goes through centralized `scripts/derive-branch-name` (closes mika#892 by construction).
- Adding a third sibling skill (e.g., `dev-explore`) requires only adding a manifest + system_prompt + an entry in the command-lookup table — no handler edits, no tools.json edits.

## Phase 0 — pre-implementation verification

### Pin 1 — bundled skill discovery + `_shared/` exclusion

`crates/mika-agent/build.rs` walks `skills/bundled/` at build time and generates `BUNDLED_SKILL_MANIFESTS`. Pre-commit verification:

1. Read `build.rs`. Pin the **exact line range** where directory iteration over `skills/bundled/*` happens.
2. Pin the **current exclusion filter** (if any) — does the loop already skip directories without `skill.toml`? Does it skip leading-underscore names?
3. Pin the **proposed addition** to that loop: `if dir_name.starts_with('_') { continue; }` (or equivalent). State the diff target line range and the rationale.

If (a) — two manifests pointing at same handler — works without change to build.rs: option B is feasible (single module, two manifests).
If only (b) — `_shared/` excluded from discovery — the proposed exclusion filter is needed: option A is the path.

Pin the verification result HERE before coding Change 2 or Change 4.

### Pin 2 — runtime tool registration

When two skills both expose `run_claude_pilot`, mika-agent's tool registration must not collide. Verify in the agent runtime: does each skill register its own copy of `run_claude_pilot`, or is the tool name globally namespaced and second registration is a no-op? Read the relevant code in `crates/mika-agent/src/skills/` to understand.

This affects how dev-groom's tools.json should look:
- If tool names are per-skill: dev-groom needs its own `tools.json` with `run_claude_pilot` entry (skill enum `["dev-groom"]`).
- If tool names are global: dev-groom can leave `tools.json` empty and the tool is shared from dev-pilot's exposure (current "working by accident" state).

### Pin 3 — `scripts/derive-branch-name` invocation from inside the handler

dev-pilot's handler currently invokes `scripts/derive-branch-name` via path lookup. Verify that lookup works in:
- Dev mode (handler runs in mika-platform repo with `scripts/` adjacent)
- Containerized deploy (Dockerfile may not include `mika-platform/scripts/` — script may need vendoring into mika repo)

If containerized deploy doesn't have access: vendor `derive-branch-name` and `derive-worktree-path` into `mika/scripts/` (propagated copy, like `mika-platform/.claude/commands/` propagation pattern). Add a CI sync check to keep them aligned.

## Approach

Recommend **Option A — two-skill copy with shared library** as v1 (smallest blast radius). If Pin 1 verifies that build.rs supports option B (two manifests, one handler directory), reconsider during implementation.

### Change 1 — Centralize slug-derivation invocation (with sync-check spec, architect F4)

dev-pilot's handler currently invokes `scripts/derive-branch-name` from the meta-repo. dev-groom's handler does inline derivation. Both must invoke the centralized script (resolves mika#892).

**Containerized-deploy fallback (per Pin 3):** if `mika-platform/scripts/` is not mounted in production containers, vendor copies into `mika/scripts/derive-branch-name` and `mika/scripts/derive-worktree-path`. Add CI sync gate:

```yaml
- name: Verify vendored derivation scripts match canonical
  run: |
    diff mika/scripts/derive-branch-name mika-platform/scripts/derive-branch-name || exit 1
    diff mika/scripts/derive-worktree-path mika-platform/scripts/derive-worktree-path || exit 1
```

(Step lives in mika-platform's CI since it has both repos accessible; or mika's CI checks out mika-platform read-only.)

**Operator decision required if Pin 3 reveals container deploy can't access either copy:** The Rust-builtin path (compile slug derivation as `mika-agent` builtin tool) is cleaner long-term but requires a Vincent decision because it inverts the script-as-source-of-truth relationship from mika-platform#58. Halt and surface if this branch fires.

### Change 2 — Move shared logic to `mika/skills/bundled/_shared/dispatch-lib.sh`

**Extraction boundary (architect F3 BLOCKER):**

**Per-skill (stays in `<skill>/handlers/run.sh`):**
- Shebang line
- `source <relative-path>/_shared/dispatch-lib.sh`
- One variable assignment: `ENTRY_COMMAND="/mika"` (dev-pilot) or `ENTRY_COMMAND="/mika-groom-ticket"` (dev-groom)
- One function call: `dispatch_claude_pilot "$ENTRY_COMMAND"`
- That's it. < 10 LOC for either skill (well under the < 30 LOC ceiling).

**Shared (moves to `_shared/dispatch-lib.sh`):**
- JSON input parsing (read stdin, extract repo/issue/prompt/task_id/iteration_context)
- Slug derivation (invoke centralized `derive-branch-name`)
- Worktree path computation (invoke centralized `derive-worktree-path`)
- Worktree creation (with idempotent reuse)
- Env scrubbing (MIKA_* env vars per `feedback_command_on_cli` discipline)
- Relay invocation (mika-relay permission gating)
- EXIT trap setup (the existing trap with crash-tail-to-RESULT semantics; once mika#887 ships, the trap will include trace-tail logic, naturally migrating in)
- claude-pilot subprocess `exec` with the entry slash command parameterized

**Function signature exported by `_shared/dispatch-lib.sh`:**

```sh
# Single entrypoint. Args: $1 = entry slash command (e.g., "/mika" or "/mika-groom-ticket")
# Reads JSON from stdin (handler's input).
# Sets up worktree, scrubs env, invokes relay, exec's claude-pilot.
# Does NOT return — exec replaces the shell process.
dispatch_claude_pilot() {
    local ENTRY_COMMAND="$1"
    # ... all the shared logic ...
    exec claude-pilot --command "$ENTRY_COMMAND" --task-id "$TASK_ID" --cwd "$WORKTREE_DIR" -- "$PROMPT"
}
```

That's the single API surface. Internal helpers (e.g., `_parse_input_json`, `_set_up_worktree`, `_scrub_env`) are file-private (underscore-prefixed) and not part of the contract.

**dev-pilot's handler after Change 2 (target shape):**
```sh
#!/bin/bash
set -e
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot "/mika"
```

**dev-groom's handler after Change 2:**
```sh
#!/bin/bash
set -e
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot "/mika-groom-ticket"
```

Both < 5 LOC. Only difference is the slash command argument.

The `< 30-LOC` ceiling in R2 is loose — actual target is < 10 LOC each.

### Change 3 — dev-groom `tools.json` populated (Pin 2 decision tree, architect F2 BLOCKER)

**Decision tree based on Pin 2 verification:**

- **(a) Per-skill namespacing — tool names scoped to the skill (preferred outcome):**
  - dev-groom's `tools.json` mirrors dev-pilot's schema exactly with `skill: ["dev-groom"]` enum.
  - Both skills register their own `run_claude_pilot` tool. No collision.
  - This is the cleanest resolution and matches dev-pilot's existing shape.

- **(b-i) Global namespacing + rename — tool names shared across skills, requiring unique names:**
  - dev-groom registers a tool named `run_claude_pilot_groom` (suffix-disambiguated).
  - dev-groom's `tools.json` exposes the renamed tool with the same schema; mika-dev's LLM would need to learn the new name (system prompt + skill keyword updates).
  - Higher blast radius. Requires a Vincent decision before adopting.

- **(b-ii) Empty `tools.json` (status quo) — REJECTED.** The plan's own problem statement identifies this as a bug ("working by accident"). Not an acceptable resolution; the empty-tools.json must be eliminated by this PR regardless of Pin 2 outcome.

**Branch condition gate:** Phase 0 Pin 2 verifies which case applies. If (a): proceed with the simple Change 3. If (b): halt and surface to Vincent — the (b-i) path requires explicit operator decision because of the LLM-prompt blast radius.

### Change 4 — Build.rs `_shared/` exclusion + CI gate (architect F1 BLOCKER)

**Build.rs change:**

Read `crates/mika-agent/build.rs` (Pin 1 verification). At the bundled-skill discovery loop, add:

```rust
// Skip non-skill subdirectories (convention: leading underscore)
if entry.file_name().to_str().is_some_and(|n| n.starts_with('_')) {
    continue;
}
```

Pin the exact line where the loop iterates and where the filter should be inserted. Filter before `skill.toml` parsing.

**CI gate content (specified, not deferred):**

Add `bundled-skill-shared-exclusion` job to `.github/workflows/ci.yml` (mika repo). Step:

```yaml
- name: Verify _shared/ not registered as a bundled skill
  run: |
    # Build the agent with verbose skill registration logging.
    cargo build -p mika-agent --features dump-bundled-skills 2>&1 | tee build-skills.log
    if grep -E '"_shared"|skill name: _shared' build-skills.log; then
      echo "ERROR: _shared/ directory was registered as a bundled skill — convention violation."
      exit 1
    fi
    # Also verify _shared/ is not present in BUNDLED_SKILL_MANIFESTS via cargo expand or similar.
```

If `dump-bundled-skills` feature flag doesn't exist, this Change adds it as a build-time `cfg(debug_assertions)` print that lists discovered skill names.

**Acceptance:** the CI gate fails loudly if any future change to `build.rs` or `skills/bundled/` directory layout accidentally registers a `_*`-prefixed directory as a skill.

If Pin 1 reveals build.rs needs adjustment for `_shared/` to be excluded from skill discovery, update `build.rs` accordingly. Add a CI gate (similar to `byte-slice-lint`, `loop-select-lint`) that asserts `_shared/` doesn't accidentally register as a bundled skill.

## Critical files

| Purpose | Path |
|---|---|
| Source — dev-pilot handler | `mika/skills/bundled/dev-pilot/handlers/run.sh` (441 LOC) — refactor to thin wrapper |
| Source — dev-groom handler | `mika/skills/bundled/dev-groom/handlers/run.sh` (133 LOC) — refactor to thin wrapper |
| New — shared dispatch lib | `mika/skills/bundled/_shared/dispatch-lib.sh` |
| Modify — dev-groom tools.json | `mika/skills/bundled/dev-groom/tools.json` (currently `[]`) |
| Verify — build-time discovery | `crates/mika-agent/build.rs` |
| Centralized slug derivation | `mika-platform/scripts/derive-branch-name`, `mika-platform/scripts/derive-worktree-path` (or vendored mika/scripts/ copies) |

## Out of Scope

- **Merging dev-groom into dev-pilot as a runtime parameter.** The user-facing skill identity (two distinct keyword sets, two distinct intents) stays.
- **Adding a `dev-explore` or third sibling skill** in this PR. The factorization is verified by structural property (R3 below) but the third skill is a follow-up if needed.
- **mika#887's trace logic implementation** — #887 lands its surgical fix in `dev-pilot/handlers/run.sh`. This refactor's Change 2 then extracts the whole handler body to the shared lib, naturally absorbing the trace logic. No cross-ticket scope dependency: this plan does NOT modify or pre-shape #887's landing site.
- **Engine-side claude-pilot CLI changes** — the entry slash command is selected by the handler before exec, not by claude-pilot itself.

## Acceptance Criteria

- [x] R0 (Phase 0 gate): All 3 pinned facts (Pin 1-3) verified before commit. Implementer halts and surfaces to operator on any disagreement.
- [x] R1: dev-groom's `tools.json` is non-empty and exposes its own `run_claude_pilot` entry with skill enum `["dev-groom"]` (or global tool name per Pin 2).
- [x] R2: dev-pilot's and dev-groom's `handlers/run.sh` are < 30 LOC each (thin wrappers around `_shared/dispatch-lib.sh`).
- [x] R3: Adding a hypothetical third sibling skill (e.g., `dev-explore`) requires only: a new `skill.toml`, a new `system_prompt.md`, and one entry in the command-lookup table inside `_shared/dispatch-lib.sh`. No handler script edits in dev-pilot/dev-groom directories. Verified by a documentation comment + ideally a structural test.
- [x] R4: Slug derivation in both handlers invokes `scripts/derive-branch-name` from the centralized location (closes mika#892 by construction). Redispatching the same ticket via either skill produces the same branch slug.
- [x] R5: A test (or make target) verifies symmetric behavior: synthetic JSON input dispatched through both skills produces identical worktrees and branch refs; only the runtime artifact difference is which Claude Code command was invoked (visible in `/var/log/claude-pilot/<task_id>.log`'s `[prompt]` line).
- [x] R6: `bash -n` and `shellcheck -s bash` pass on all three files (`dev-pilot/handlers/run.sh`, `dev-groom/handlers/run.sh`, `_shared/dispatch-lib.sh`).
- [x] R7: `cargo build` succeeds with `_shared/` directory present in `skills/bundled/` (build.rs handles the exclusion correctly).
- [x] R8: An end-to-end smoke test dispatches a known small ticket via dev-pilot AND via dev-groom; both produce a worktree, both reach the entry slash command stage, both exit cleanly. Verifies the refactor doesn't break either dispatch path.

**Note on mika#887 sequencing:** If mika#887 ships first (sprint sequence #887 → #893), the BASH_XTRACEFD trace logic lives in `dev-pilot/handlers/run.sh` at refactor time. Change 2 extracts the WHOLE handler to `_shared/dispatch-lib.sh` — the trace logic comes along as part of the handler body. No separate migration step. If sprint order is reversed (#893 first), mika#887 lands directly in the shared lib once #893 establishes the substrate. Either order works without a cross-ticket scope change in this plan.

## Verification

1. **Static checks:** `bash -n` + `shellcheck -s bash` on all three handler/lib files.
2. **Build smoke:** `cargo build -p mika-agent` succeeds; `BUNDLED_SKILL_MANIFESTS` includes both dev-pilot and dev-groom (and NOT `_shared`).
3. **Handler symmetry test:** synthetic JSON input fed to dev-pilot and dev-groom; assert worktree paths and branch refs are identical, only the entry command differs.
4. **Regression smoke:** dispatch a small ticket via dev-pilot (e.g., the existing `/mika` flow against any p3 task) — must complete cleanly. Same for dev-groom (e.g., `mika ask --agent mika-dev "groom <some-test-ticket>"`).
5. **Slug-stability regression:** redispatch a previously-groomed ticket (e.g., one whose body already has a `Branch:` callout). Verify the dispatcher uses the existing slug (not re-derives).

## Cross-references

- mika#892 — slug regression in dev-groom; closes by construction once Change 1 lands
- mika#887 — BASH_XTRACEFD trace injection in dev-pilot handler; sprint-sequenced before this refactor so its trace logic can migrate to shared lib in Change 4
- mika-platform#58 — original centralization of slug derivation (the discipline this refactor honors)
- mika-platform#59 / #60 — implementation of #58
- `mika-platform/CLAUDE.md` § Cross-Repo Development — discipline rule "invoke the scripts; do not re-derive"
- mika#886 — canonical reproduction surface for the divergence drift this refactor eliminates
- `mika/skills/bundled/dev-pilot/handlers/run.sh` (existing reference shape, 441 LOC)
- `mika/skills/bundled/dev-groom/handlers/run.sh` (existing divergent shape, 133 LOC)

## Sequencing & Risk

- **Risk: build.rs doesn't recognize `_shared/`** as non-skill. Mitigated by Pin 1 + Change 5 (CI gate).
- **Risk: tool registration collision** (two skills exposing same `run_claude_pilot` tool name). Mitigated by Pin 2 outcome — either per-skill registration works, or the tool name needs unique suffix.
- **Risk: regression on existing dev-pilot dispatches.** Mitigated by R7-R9 — static checks + symmetric smoke tests + regression smoke.
- **Risk: containerized deploy breaks if `scripts/derive-branch-name` not accessible.** Mitigated by Pin 3 outcome — vendor into `mika/scripts/` if needed with CI sync.
- **Sequencing:** Independent of mika#887 and mika#889. If #887 ships first, its trace logic comes along when Change 2 extracts the dev-pilot handler. If #893 ships first, #887's later landing target shifts to the shared lib substrate. No cross-ticket scope dependency in this plan.

## Grooming history

- /ce:plan (operator-drafted, well-specified ticket body) → mika-arch first-pass review (pending) → revisions (TBD) → mika-arch second-pass (pending GROOMED).
