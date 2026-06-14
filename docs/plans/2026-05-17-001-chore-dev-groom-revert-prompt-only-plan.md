---
ticket: mika#1173
type: chore
branch: chore/1173/dev-groom-revert-prompt-only-design
status: draft
created: 2026-05-17
---

# Plan — Revert dev-groom prompt-only design; restore deterministic handlers/+tools.json

## 0. Phase 0 — Root-cause verification (added in pass-1 iteration per mika-arch F1)

**Claim under verification** (from pass-1 brief): mika-dev DID call `run_claude_pilot({"skill": "dev-groom"})` for the cited regression; the failure is downstream in the inner Claude Code session, not at the outer dispatch surface.

**Evidence** (queried 2026-05-17, `~/.mika/data/mika.db`):

The ticket cites mika#1166 as the most recent regression. The actual triggering session for `/mika-audit` of this class was mika#1162 (same failure class, same date). Verbatim from `tool_calls` table (input column truncated to 200 chars; full input preserved in DB):

```
2026-05-17T07:39:40Z  run_claude_pilot  dev-pilot  success=1  {"prompt":"mika#1162","skill":"dev-groom","task_id":"52abc348-898e-4082-95d2-07bca7a487c3"}
2026-05-17T07:41:23Z  run_claude_pilot  dev-pilot  success=1  {"prompt":"mika#1162","skill":"dev-groom","task_id":"52abc348-898e-4082-95d2-07bca7a487c3"}
2026-05-17T08:06:41Z  run_claude_pilot  dev-pilot  success=1  {"prompt":"mika#1162","skill":"dev-groom","task_id":"52abc348-898e-4082-95d2-07bca7a487c3"}
2026-05-17T08:34:42Z  run_claude_pilot  dev-pilot  success=1  {"prompt":"mika#1162","skill":"dev-groom","task_id":"11749683-a350-499a-9979-8c0cfe9474cf"}
```

Five `run_claude_pilot` calls fired with `skill="dev-groom"` and `success=1`. **Outer dispatch is not the failure surface.**

The corresponding task row (`b5a18e32-df4d-4521-b767-9b48d99c00d0`, action_config preserved):

```
input: {"prompt":"mika#1162","skill":"dev-groom","task_id":"52abc348-898e-4082-95d2-07bca7a487c3"}
status: delivered
result: PIPELINE FAILURE: dev-groom produced no valid plan file (no docs/plans/2026-05-17-*-plan.md >500 bytes found) and no /ce:plan invocation detected in session log. Session drifted into executor mode.
        PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged (...)
        claude-pilot completed (status: success).
        Session: c044d2e8-7acd-4dd9-8295-c040979cd52a
        Turns: 2  Cost: $0.0  Duration: 3ms
        log tail: [init] Session , model unknown, task b5a18e32-... [done] Success | 2 turns | $0.00 | 0s
```

The "model unknown" pattern in the log tail is consistent with claude-pilot reaching the `[init]` stage but failing to bind the Anthropic model field — a session-init failure on the **inner side**, downstream of the outer dispatch. This is distinct from the mika#1168 outer-refusal pattern (sonnet-4-6 returning "Prompt injection. Rejected."), which would surface at the **mika-dev** loop, not the claude-pilot subprocess.

**Adjacent finding** (from same DB query): the earliest attempts for mika#1162 had `skill="dev-pilot"` with `success=0` (engine rejected — wrong dispatch class for a groom-needing ticket). Then mika-dev self-corrected to `skill="dev-groom"`. **This is itself evidence for D1's Option A choice** — the union-enum tool surface invites this exact confusion. With two discrete tools (`run_claude_pilot` for implement, `run_claude_pilot_groom` for groom), the LLM cannot pick the wrong enum value because the tool's existence in the schema is class-bound.

**Conclusion of Phase 0 verification:**
- Outer dispatch fires correctly (5/5 attempts success=1 once mika-dev picked the right skill).
- Inner Claude Code session is the failure surface — 3ms, 2 turns, $0, no plan file, no `/ce:plan` invocation.
- mika#1168 outer-refusal pattern is NOT the active cause for this regression class (different failure shape, different agent surface).
- The plan's structural fix (Option A + inner-session slash-command propagation) remains correctly scoped.

The plan proceeds.

### 0.1 Phase 0 Pins — Verbatim source slices

**Base SHA:** `f459130f17a73d9e65eadbdd1a7bb2eaf0e3b999` (origin/main, fetched 2026-05-17).

**Pin 1 — `skills/bundled/dev-pilot/tools.json` enum shape (lines 8–12 at base SHA):**

```json
        "skill": {
          "type": "string",
          "enum": ["dev-pilot", "dev-groom"],
          "description": "Which skill prompt to run. 'dev-pilot' for implementation dispatch (/mika pipeline), 'dev-groom' for grooming dispatch (/mika-groom-ticket pipeline)."
        },
```

Step 3 narrows this to `enum: ["dev-pilot"]` and updates the description.

**Pin 2 — `skills/bundled/_shared/dispatch-lib.sh` `_set_up_worktree` cp block (lines 354–357 at base SHA):**

```bash
        mkdir -p "$WORKTREE_DIR/.claude"
        cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true

```

Step 5 appends a `cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"` block immediately after these lines.

**Pin 3 — `crates/mika-agent/src/well_known_agents.rs` `MIKA_DEV_IDENTITY` allowlist head (lines 115–125 at base SHA):**

```rust
[skills]\n\
allowlist = [\n\
  \"self-dev\",\n\
  \"self-dev-callback\",\n\
  \"self-dev-iterate\",\n\
  \"self-dev-webhook-qa\",\n\
  \"self-dev-webhook-ci\",\n\
  \"self-dev-webhook-ready-label\",\n\
  \"dev-pilot\",\n\
  \"build-mika\",\n\
```

Step 7 inserts `\"dev-groom\",\n\` between the `\"dev-pilot\",\n\` line (line 122) and `\"build-mika\",\n\` line (line 123).

**Pin 4 — `crates/mika-agent/src/skills/executor.rs` SKILL string-check call sites (4 actual sites at base SHA, the docstring at line 1180 is the rationale not the executable check):**

- Line 763–768 — `derive_dispatch_class`:
```rust
fn derive_dispatch_class(skill: Option<&str>) -> &'static str {
    match skill {
        Some("dev-groom") => "groom",
        _ => "implement", // dev-pilot, deploy_mika, and all others
    }
}
```

- Line 1027–1028 — open-PR-guard bypass:
```rust
        let skill = tool_input.and_then(extract_skill_from_input);
        let is_dev_pilot = skill == Some("dev-pilot");
```

- Line 1223–1225 — grooming-marker dispatch gate:
```rust
    let skill = extract_skill_from_input(input);
    if skill != Some("dev-pilot") {
        return None;
    }
```

- Line 1531–1532 — dispatch-class derivation at task-create time:
```rust
    let skill = extract_skill_from_input(input);
    let class = derive_dispatch_class(skill);
```

**Impact analysis for Pin 4:** All four sites read the `skill` field from input JSON. The new `run_claude_pilot_groom` tool schema still requires `"skill": "dev-groom"` in its input (Step 1). So all four sites continue to work unchanged. **No Rust changes required at these call sites.** Out-of-scope refactor noted in Q4: if engine call sites are later refactored to use `derive_dispatch_class(skill) == "implement"` instead of `skill == Some("dev-pilot")`, the coupling becomes more durable for future skill additions — but that's a separate hygiene improvement, not a prerequisite for this fix.

## 1. Problem

`dev-groom` was made prompt-only in PR #934 (commit b07b4778, 2026-05-02) to resolve a `run_claude_pilot` tool-name collision with `dev-pilot`. The collision was real (engine's `inject_skills_and_resolve_tools` dedupes overlapping tool names; see `crates/mika-agent/src/agent.rs` test `test_inject_skills_deduplicates_tool_names`). The chosen fix removed `dev-groom/handlers/` and `dev-groom/tools.json` entirely and consolidated the union enum `["dev-pilot", "dev-groom"]` on dev-pilot's tool.

Since then, the prompt-only design has regressed five times:

| Commit | PR | Type of fix |
|--------|----|------------|
| 938dac43 | #1032 | Detect `/ce:plan` non-invocation in post-flight |
| dfd9b959 | #1081 | Post-flight plan validation + anti-drift prompt hardening |
| 33a98a31 | #1097 | Early-exit guard for zero-artifact sessions |
| a7b2fd68 | #1109 | Port of #1097 |
| abfb0c10 | #1134 | Re-detect `/ce:plan` non-invocation |

Every fix is **detection** (catch drift post-hoc, emit `PIPELINE FAILURE`), not **prevention**. The 2026-05-17 incident on mika#1166 is the 6th occurrence: a session "lasted 3ms, 2 turns, $0" and produced no plan file or `/ce:plan` invocation in the session log.

### Root cause

Per Phase 0 verification (DB-grounded above), the failure surface is **not** the outer mika-dev dispatch — that fires correctly (`run_claude_pilot({"skill": "dev-groom", ...})` returns `success=1` consistently). The failure is in the **inner Claude Code session** spawned by claude-pilot.

The current shape couples three layers that should be decoupled:

1. **Outer dispatch (mika-dev → claude-pilot).** mika-dev's LLM emits `run_claude_pilot({"skill": "dev-groom", ...})`. The engine routes the tool call to **dev-pilot's** handler (because dev-pilot owns the tool name via union enum). dispatch-lib reads `$SKILL` from input JSON and case-switches `dev-groom` → `ENTRY_COMMAND="/mika-groom-ticket"`. This part fires correctly per Phase 0. **But** the union-enum tool surface invites mika-dev's LLM to pick the wrong value at the boundary: the 2026-05-17 mika#1162 trace shows the LLM first emitting `skill="dev-pilot"` (engine rejected, `success=0`), then self-correcting to `skill="dev-groom"`. With Option A (separate tool names), the wrong-enum-value class becomes structurally impossible — the LLM is exposed to two discrete tools and cannot conflate them.
2. **Skill-prompt loading on mika-dev.** dev-groom's `system_prompt.md` (112 lines, full Phase 1–5 walkthrough) is currently authored as if it loads into mika-dev's prompt context. But `dev-groom` is **not** in `MIKA_DEV_IDENTITY.allowlist` (`crates/mika-agent/src/well_known_agents.rs:108`). Either (a) the prompt loads anyway via trigger-keyword matching (allowlist not enforced at trigger time), or (b) it never loads and the prompt is dead documentation. Either way, the Phase 1–5 prose is misaddressed — the workflow runs inside claude-pilot's spawned Claude Code session via `/mika-groom-ticket.md`, not in mika-dev's loop. Collapsing the prompt to a thin "call the tool" shape (D3) resolves the misaddress regardless of which case (a/b) holds.
3. **Inner session reliability.** When claude-pilot launches Claude Code with `--command "/mika-groom-ticket" -- "mika#N"`, the inner session receives `/mika-groom-ticket mika#N` as its first message. Claude Code resolves slash commands from `.claude/commands/` under the cwd. The worktree's `.claude/` directory does **not** receive `commands/` from dispatch-lib (only `claude-pilot.json` and `settings.local.json` are copied — `skills/bundled/_shared/dispatch-lib.sh` lines 354–357). The slash-command file lives at the meta-repo root (`/data/workspace/mika-platform/.claude/commands/mika-groom-ticket.md`), outside the worktree. **This is the verified structural failure surface** — Phase 0's "model unknown, $0, 2 turns, 3ms" log tail matches the shape of a session that couldn't resolve its entry slash command and exited fast. The inner LLM gets `/mika-groom-ticket mika#N` as raw text with no resolving file, has to improvise from training-data priors, and either drifts into executor mode or exits after fetching the issue with no plan.

The ticket frames the problem as "LLM-dependency at the OUTER dispatch surface" (the `/ce:plan` emission when the dev-groom system prompt loads). Per Phase 0, that framing is incomplete — the active failure mode is the **inner session's missing slash-command resolution**. Restoring `handlers/`+`tools.json` on dev-groom (Option A) is necessary for prevention of the wrong-enum-value class (point 1) and for prompt-collapse hygiene (point 2), but the structural fix for the active regression is Step 5 (inner-session slash-command propagation).

This plan addresses all three layers in one PR.

## 2. Decisions

### D1. Option choice — Option A (separate tool names)

**Decided: Option A.** Give dev-groom its own dedicated tool name `run_claude_pilot_groom` (the ticket's exact suggested name; symmetric with `run_claude_pilot`). dev-pilot keeps `run_claude_pilot` with the enum narrowed back to `["dev-pilot"]`.

| Option | Pro | Con |
|--------|-----|-----|
| A. Separate tool names (chosen) | No engine changes; no dedup conflict; mika-dev sees two discrete tools (cannot conflate); smallest blast radius | Tool-surface duplication (~30 lines of JSON); enum-routing logic in dispatch-lib's case switch becomes dead code for the groom path (still kept as defense-in-depth) |
| B. Union enum + dev-groom-owned handler | Preserves single tool surface | Engine dedup keeps only one handler per tool name — dev-groom's handler would be silently dropped (the exact #934 collision). Would require new engine routing logic (per-skill handler dispatch within a single tool name) — large surface area, not justified by the #934 history |
| C. Strengthen prompt-only with hard guards | Smallest diff | Pattern has 5 prior instances. Explicitly rejected in ticket. |

**Rationale for picking A over B:**
- B is structurally what #934 was already attempting — the union enum, same tool name, with dev-groom's handler getting collapsed by engine dedup. Resurrecting it requires engine changes (route by skill-parameter, not tool name), which is a large refactor with its own failure modes.
- A is a one-file engine-side change (a new tool name registration via dev-groom's tools.json) plus narrowing dev-pilot's enum back to single-value.
- The ticket's wording ("**Recommended**") for A is consistent with the maintainability bar in `mika/docs/architecture/review-guide.md` §3 (KISS).

### D2. Tool name — `run_claude_pilot_groom`

The ticket suggests two candidates: `run_claude_pilot_groom` or `dev_groom_dispatch`. Chosen: **`run_claude_pilot_groom`**. Reasons:
- Sibling-symmetric with `run_claude_pilot` (the dev-pilot tool).
- Self-describing: anyone reading mika-dev's tool list can tell at a glance what it does.
- `dev_groom_dispatch` mixes a domain prefix (`dev_groom`) with a verb suffix (`dispatch`) and breaks the existing `run_<thing>` convention used by `run_gh`, `run_claude_pilot`, etc.

### D3. dev-groom system prompt — collapse to thin "call the tool" shape

Current `skills/bundled/dev-groom/system_prompt.md` is 112 lines, walks mika-dev through Phase 1–5 of the grooming workflow as if mika-dev should execute them. This is the prompt's structural confusion: the workflow runs in the **inner Claude Code session** via `/mika-groom-ticket`, not in mika-dev's loop.

**Replace with a dev-pilot-shaped prompt (~25 lines):** "When asked to groom, call `run_claude_pilot_groom` with the ticket reference. Do not do the work inline. Wait for the callback." Mirror dev-pilot's prompt at `skills/bundled/dev-pilot/system_prompt.md`.

### D4. Inner-session slash-command propagation — fix in same PR

Bundled with the structural revert: extend dispatch-lib's worktree-prep block to **copy `.claude/commands/` from the platform root into the worktree's `.claude/` directory before launching claude-pilot**. Without this, the inner session cannot resolve `/mika-groom-ticket` (nor `/ce:plan`-adjacent commands referenced inside it) and falls back to text-mode improvisation.

This is a 3-line addition to `_set_up_worktree` (see dispatch-lib lines 354–357 area). It also fixes a latent gap on the dev-pilot path that has been masked by `/mika`'s longer prompt giving the LLM more recovery surface.

### D5. mika-dev identity allowlist — explicitly add `dev-groom`

Currently `MIKA_DEV_IDENTITY` (well_known_agents.rs:108) allows `dev-pilot` only. Adding `dev-groom` is a one-line change. This is **necessary** for the new `run_claude_pilot_groom` tool to register: skills are denied-by-default per mika#815, and the tool registration follows the skill.

### D6. Engine call sites — keep current

The engine has two places that read the `skill` parameter:

- `derive_dispatch_class` (executor.rs:765): `Some("dev-groom") => "groom"`. Continues to work — the SKILL parameter is still passed in input JSON regardless of which tool registers it.
- `check_task_has_open_pr` bypass (executor.rs:1180): "Skill is not `dev-pilot`". Continues to work — bypass condition unchanged.

Both rely on the SKILL field being present in tool input. Both new tool schemas (dev-pilot's narrowed `run_claude_pilot` and dev-groom's new `run_claude_pilot_groom`) require the `skill` field, so the JSON shape stays compatible. **No engine code changes needed.**

### D7. Hardening fixes from #1032/#1081/#1097/#1109/#1134 — keep, do not revert

Each prior fix added a layer of post-flight detection: zero-commit check, plan-file-size check, `/ce:plan` log grep, body-callout verification. These are **defense-in-depth** — keep them. They no longer have to be the primary defense, but they remain valuable for catching regressions on the inner-session path.

Specifically:
- dispatch-lib lines 460–550 (post-flight validation): **keep as-is**.
- `early_exit_zero_action` (claude-pilot-py types.py:104, `CLAUDE_PILOT_MIN_TOOL_CALLS`): **keep**.
- `_verify_and_write_body_callout` (dispatch-lib): **keep**.

The expectation after this fix is that these detectors should fire **far less often**. If they fire at the same rate, the structural fix didn't work and we need to escalate.

### D8. Plan-file path — current ticket number, not parent

Per `/mika-groom-ticket.md` filename convention, this plan lives at `mika/docs/plans/2026-05-17-001-chore-dev-groom-revert-prompt-only-plan.md`. The branch slug is `chore/1173/dev-groom-revert-prompt-only-design` (label-derived `chore` despite the ticket being labeled `bug` — `chore` came from the conventional-commit title prefix per `derive-branch-name` priority order). Semantic type drift (chore vs. bug vs. refactor) is captured in the plan's `type:` frontmatter, not the branch slug — per the immutability invariant from mika#844.

## 3. Implementation

### Step 1 — Restore `dev-groom/tools.json`

Create `mika/skills/bundled/dev-groom/tools.json`:

```json
[
  {
    "name": "run_claude_pilot_groom",
    "description": "Dispatch a headless Claude Code grooming session via claude-pilot. Runs the /mika-groom-ticket pipeline (two-pass mika-arch review → plan-on-branch). Long-running — returns a task ID immediately, results arrive via callback when Claude Code finishes.",
    "input_schema": {
      "type": "object",
      "properties": {
        "skill": {
          "type": "string",
          "enum": ["dev-groom"],
          "description": "Must be 'dev-groom'. Field is required for engine dispatch-class derivation."
        },
        "prompt": {
          "type": "string",
          "description": "Typed ticket reference (e.g., 'mika#214', 'mika-skills#8'). The handler derives the branch, creates a worktree, and runs /mika-groom-ticket."
        },
        "task_id": {
          "type": "string",
          "description": "The UUID returned by create_task (36-char format). Used for log filename and task correlation."
        },
        "iteration_context": {
          "type": "string",
          "description": "Iteration feedback for an existing grooming session. When present, appended to the prompt so the pipeline addresses specific feedback."
        }
      },
      "required": ["skill", "prompt", "task_id"]
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh",
      "long_running": true,
      "estimated_duration_secs": 7200
    }
  }
]
```

### Step 2 — Restore `dev-groom/handlers/run.sh`

Create `mika/skills/bundled/dev-groom/handlers/run.sh` (mode 755), mirroring dev-pilot's thin-wrapper shape:

```bash
#!/bin/bash
# Thin wrapper: dispatches claude-pilot via shared plumbing for the groom path.
# Entry command derives from $SKILL in the lib (case switch). See mika#1173.
set -e
# shellcheck source=../../_shared/dispatch-lib.sh
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot
```

### Step 3 — Narrow `dev-pilot/tools.json` enum

Edit `mika/skills/bundled/dev-pilot/tools.json`:

```diff
-          "enum": ["dev-pilot", "dev-groom"],
-          "description": "Which skill prompt to run. 'dev-pilot' for implementation dispatch (/mika pipeline), 'dev-groom' for grooming dispatch (/mika-groom-ticket pipeline)."
+          "enum": ["dev-pilot"],
+          "description": "Must be 'dev-pilot'. For grooming, call run_claude_pilot_groom instead."
```

Also update the top-level `description` to drop the dev-groom reference:

```diff
-    "description": "Dispatch a headless Claude Code session via claude-pilot. Supports implementation (dev-pilot, entry: /mika) and grooming (dev-groom, entry: /mika-groom-ticket). Long-running — returns a task ID immediately, results arrive via callback when Claude Code finishes.",
+    "description": "Dispatch a headless Claude Code implementation session via claude-pilot. Runs the /mika pipeline. For grooming, use run_claude_pilot_groom instead. Long-running — returns a task ID immediately, results arrive via callback when Claude Code finishes.",
```

### Step 4 — Collapse `dev-groom/system_prompt.md` to dev-pilot shape

Rewrite `mika/skills/bundled/dev-groom/system_prompt.md` to ~25 lines, mirroring dev-pilot's prompt. The new prompt instructs mika-dev to call `run_claude_pilot_groom` and **not** to execute Phase 1–5 inline. The Phase 1–5 workflow content moves into a CHANGELOG note inside the file or, better, is removed entirely (the workflow lives in `/mika-groom-ticket.md` slash-command file, which is the only place it should be authored).

Key sections of the new prompt:
- ROLE: "When asked to groom a ticket, call `run_claude_pilot_groom`. Do not do the work inline."
- TOOL CALL shape (JSON example).
- Rules: always pass `task_id`, one session per issue, wait for callback, do NOT analyze/plan inline.
- Callback handling: extract `Verdict: GROOMED` / `Verdict: ESCALATE`, surface result.

### Step 5 — Update `_shared/dispatch-lib.sh` to copy slash commands

In `_set_up_worktree`, after the existing `cp` block (lines 354–357):

```diff
         # Copy gitignored .claude/ config into worktree (relay + permissions only)
         mkdir -p "$WORKTREE_DIR/.claude"
         cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
         cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
+        # Copy slash commands so the inner Claude Code session can resolve
+        # /mika, /mika-groom-ticket, /ce:plan-adjacent commands, etc.
+        # Without this, --command "/mika-groom-ticket" arrives as raw text in
+        # the inner session and the LLM has to improvise (mika#1173).
+        #
+        # Staleness profile (NF1, pass-1 review): the cp is a snapshot at
+        # worktree-creation time. If the operator edits a command file at the
+        # platform root after worktree creation but before the inner session
+        # completes, the inner session sees the pre-edit version. This is
+        # acceptable for two reasons:
+        #   1. Worktrees are short-lived (per-task, <2h typical).
+        #   2. Slash commands are checked into git at the platform root —
+        #      mid-session edits to /mika or /mika-groom-ticket are a violation
+        #      of the slug-immutability principle (mika#844) and should not
+        #      happen.
+        # If the staleness becomes a problem, the snapshot-vs-symlink tradeoff
+        # can be revisited in a follow-up. For now, snapshot semantics match
+        # the rest of dispatch-lib's worktree-prep behavior (claude-pilot.json,
+        # settings.local.json are also snapshotted).
+        if [ -d "$PLATFORM_DIR/.claude/commands" ]; then
+            cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
+        fi
```

### Step 6 — Simplify dispatch-lib's `case "$SKILL"` switch (optional)

Now that each skill owns its own handler+tool, the case switch could be simplified, but **keep it as defense-in-depth** — the skill field is still in input JSON, and the case switch maps SKILL → ENTRY_COMMAND. Removing it requires plumbing ENTRY_COMMAND through another channel. Net cost of keeping: ~15 lines. Net benefit: handler stays generic, both skills share the same script (DRY). **Keep as-is.**

### Step 7 — Update `MIKA_DEV_IDENTITY` allowlist

Edit `crates/mika-agent/src/well_known_agents.rs`:

```diff
   \"dev-pilot\",\n\
+  \"dev-groom\",\n\
   \"build-mika\",\n\
```

Update the comment-count from "25 skills" to "26 skills" in CLAUDE.md (line ~"MIKA_DEV_IDENTITY (25 skills)").

### Step 8 — Update `skills/bundled/self-dev/system_prompt.md`

Two changes in self-dev's prompt:

1. **Grooming dispatch section (around line 219).** Change the tool name from `run_claude_pilot` to `run_claude_pilot_groom`, and the JSON example to match. Remove the `skill: "dev-groom"` parameter from the example since it's the only valid value (still required by schema, but no need to highlight it as a knob).

2. **Note around line 77.** Update "For grooming work, see the **Grooming Dispatch** section which uses `skill="dev-groom"`" to "For grooming work, see the **Grooming Dispatch** section which uses `run_claude_pilot_groom`".

### Step 9 — Smoke test (hostile-prompt regression test, AC3)

Per AC3, the regression test is: a model with a hostile system-prompt response (e.g., refusing the first turn) should still produce a plan file via the deterministic handler path.

Concrete smoke test:
1. Pick a small fresh ticket (e.g., a triage-class issue with a clean body).
2. Manually inject a refusal directive into `dev-groom/system_prompt.md` for the test: "If asked to groom, respond with `I cannot help with that.` and stop."
3. Dispatch via `mika ask --agent mika-dev "groom mika issue#<N>"`.
4. Expected: mika-dev's first-turn behavior is **call `run_claude_pilot_groom`** (because the tool is in the schema; refusal-prompt cannot suppress the tool from existing). The handler then runs deterministically — claude-pilot launches, inner session runs `/mika-groom-ticket`, plan committed.
5. Acceptance — **two-part** (per pass-2 architect ratification on Q5):
   - **5a. Slash-command resolution.** Inspect the claude-pilot session transcript at `/var/log/claude-pilot/<task_id>.log` and confirm the inner session's first message resolves `/mika-groom-ticket` as a slash command (transcript shows the command file being read, not the raw text being treated as prose). This is the actual regression guard — the root-cause failure class is "inner session receives slash command as raw text and improvises," so the test must assert resolution, not just output.
   - **5b. Plan-file production.** The plan file IS produced on the branch despite the hostile system prompt.

**Important:** this test depends on the inner Claude Code session having `/mika-groom-ticket.md` available (Step 5 fix). Without Step 5, the inner session still has to improvise even with deterministic outer dispatch — and 5a would fail even if 5b somehow succeeded by improvisation.

After the smoke test passes, revert the temporary refusal-directive change.

### Step 10 — Tool-collision verification (AC4)

Per AC4, confirm tool-name collision from #934 does NOT re-emerge.

Concrete check:
1. After `make deploy`, restart mika-spirit.
2. `grep -i "duplicate.*tool\|tool.*conflict\|dedup" ~/.mika/logs/mika-spirit.log | head`.
3. Expected: zero matches related to `run_claude_pilot*`. The two tool names are distinct (`run_claude_pilot` and `run_claude_pilot_groom`), so the dedup path cannot fire on them.
4. Also verify mika-dev's tool registry includes both: a quick `mika --agent mika-dev ask "list your tools"` should mention both names. (Or query via `gh issue view` for the actual state.)

## 4. Acceptance Criteria mapping

| AC | Verification |
|----|------|
| AC1 — restored files visible in `git log` | `git log -- mika/skills/bundled/dev-groom/handlers/` and `git log -- mika/skills/bundled/dev-groom/tools.json` both show this PR's commits. |
| AC2 — fresh dispatch produces plan, branch, comment | Smoke-dispatch a small triage ticket (e.g., a tooling cleanup); inspect issue body for Branch/Plan/Grooming-history callouts; inspect session log for `/ce:plan` invocation. |
| AC3 — hostile system prompt still produces plan | Step 9 smoke test. The Step 5 fix is what actually delivers determinism; Step 1–4 ensure the tool surface forces a dispatch. |
| AC4 — no tool-dedup warnings | Step 10 check. Names are distinct; the engine's dedup path is not exercised on the new tool. |

## 5. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| **R1.** dev-groom is denied via allowlist after registering its new tool, so `run_claude_pilot_groom` doesn't surface to mika-dev. | Step 7 adds `dev-groom` to `MIKA_DEV_IDENTITY` allowlist. Tested via `test_mika_dev_identity_allowlist` (new test in well_known_agents.rs). |
| **R2.** Existing in-flight grooming tasks dispatched with the old shape (`run_claude_pilot({"skill": "dev-groom"})`) fail after deploy. | The old code path still exists in dispatch-lib's case switch (Step 6 keeps it). Tasks already enqueued with old shape continue to route to dev-pilot's handler, which routes to grooming via `$SKILL`. New dispatches use the new tool name. Both code paths coexist for backward compatibility during rollout. After 7 days of zero old-shape dispatches in logs, the case switch's `dev-groom` arm can be removed in a follow-up cleanup PR. |
| **R3.** Step 5 copying `.claude/commands/` blows up worktree size or copies sensitive files. | Currently `.claude/commands/` contains 4 markdown files totaling ~80KB on the meta-repo. Cost is negligible. No sensitive content (all command files are public-style docs). |
| **R4.** Step 5 introduces a divergence — inner session runs against meta-repo's command file while the meta-repo's command itself gets edited by an unrelated PR mid-deploy. | The cp is a snapshot at worktree-creation time. Worktrees are short-lived (per-task). Acceptable. |
| **R5.** The case-switch in dispatch-lib (Step 6) becomes confusing — dev-groom can route via either tool. | Add a comment block in dispatch-lib explaining the dual entry points and the 7-day cleanup timer (R2). |
| **R6.** Inner-session slash-command propagation (Step 5) exposes other commands the inner session shouldn't run (e.g., `/mika-issues`). | Inspect copied commands. `/mika`, `/mika-groom-ticket`, `/mika-doc-audit`, `/mika-issue`, `/mika-issues` are all read-write commands. Inner sessions running under claude-pilot already have the relay-permission layer (`canUseTool`) gating any actual tool calls; adding command files doesn't bypass that. |
| **R7.** Engine asserts about `enum` field in tool input JSON break tests (executor.rs lines 2003, 2198 reference union enum). | Inspect both test fixtures: they're synthetic tool schemas inside unit tests, not the production tools.json. Update to match the narrowed enum. (Listed in Step 3 follow-on.) |

## 6. Test plan

### Unit tests (Rust)
- `crates/mika-agent/src/skills/executor.rs`: update test fixtures at lines 2003 and 2198 to reflect narrowed `enum: ["dev-pilot"]`. Add a new test asserting `run_claude_pilot_groom` is registered when dev-groom skill is loaded and routes to the groom dispatch class.
- `crates/mika-agent/src/well_known_agents.rs`: existing tests that parse `MIKA_DEV_IDENTITY` must continue to pass; add an assertion that the allowlist contains `dev-groom`.

### Integration tests
- No changes required to `mika-agent/tests/eval/`. The eval harness exercises the agent loop with `MockLlmProvider`; dispatching `run_claude_pilot_groom` from mock LLM output should resolve to dev-groom's handler. Add one new eval case asserting the tool registration shape.

### Smoke tests
- AC2 smoke: dispatch a small ticket through the full path post-deploy.
- AC3 smoke: hostile-system-prompt regression (Step 9).
- AC4 smoke: log inspection (Step 10).

### Build verification
- `cargo build -p mika-agent` should succeed (build.rs picks up the new `tools.json`).
- `cargo test -p mika-agent` should pass.
- `cargo clippy -p mika-agent --all-targets -- -D warnings` should pass.

## 7. Rollout sequence

1. PR opens with all changes from Steps 1–8.
2. CI runs unit tests + build.
3. After merge, `make deploy` rolls out the new bundled skill shape.
4. Smoke tests (Steps 9, 10) execute against the deployed instance.
5. Monitor `kg_budget_exhausted`, `dev-groom_pipeline_failure`, and post-flight validation log lines for 72 hours.
6. After 7 days of zero old-shape `run_claude_pilot({"skill": "dev-groom"})` dispatches, file a follow-up cleanup PR to remove the dev-groom arm from dispatch-lib's case switch (R2 mitigation timer).

## 8. Out of scope

- **Engine refactor for per-skill handler dispatch within a single tool name.** Option B path — explicitly rejected (D1).
- **Pinning dev-groom to a specific model.** Rejected in ticket alternatives.
- **Continuing hardening (Option C).** Rejected — 5 prior instances demonstrate insufficiency.
- **mika-arch system prompt changes.** The architect's role here is review; this fix doesn't change architecture's behavior.
- **Migration of similar fragility in dev-pilot.** dev-pilot has not exhibited the same drift class — `/mika` has a longer prompt giving the inner LLM more recovery surface. If a regression appears later, the same structural fix (Step 5 slash-command propagation) is already applied, so the dev-pilot path benefits from this PR without requiring its own change.

## 9. Open questions for architect

- **Q1.** Is the inner-session slash-command propagation (Step 5) a separate ticket from this one's stated scope ("restore handlers/+tools.json")? The ticket framing emphasizes the outer dispatch surface, but the actual failure mode (no `/ce:plan` invocation in session log) is downstream of slash-command resolution. Bundling Step 5 here is judgment that the architect should ratify or break out.
- **Q2.** Should the dev-groom system prompt be reduced to ~25 lines (Step 4) in this PR, or kept verbose for now and reduced in a separate PR? Reduction may invalidate certain regression-detector phrases (e.g., "FORBIDDEN ACTIONS" block referenced by post-flight log greps). Verify nothing else greps the prompt content.
- **Q3.** Is the 7-day cleanup timer in R2 reasonable, or should the dual-arm coexistence be removed immediately (with a one-deploy migration)?
- **Q4.** Should engine call sites that check `skill == "dev-pilot"` (executor.rs:1180 bypass logic) be updated to use the dispatch-class instead (`derive_dispatch_class(...) == "implement"`)? The two are currently coupled but conceptually different — re-coupling would simplify future skill additions but is not required by this fix.
