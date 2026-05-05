---
ticket: mika#984
title: "fix(skills): validate_required_fields silently no-ops on production schema"
type: fix
status: groomed-pending
labels: [bug, p0-critical]
branch: fix/984/skills-validate-required-fields-silently
related_prs: [969]
---

# Plan — mika#984: validate_required_fields silently no-ops on production schema

## Problem

PR #969 added `validate_required_fields()` at `execute_skill_tool` entry to reject dispatches missing required fields *before* spawning a subprocess. The runtime check fires at `crates/mika-agent/src/skills/executor.rs:202` and reads `skill_tool.definition.input_schema.get("required").as_array()`.

In production, two consecutive `run_claude_pilot` dispatches with `{"prompt":"mika#666","task_id":"<uuid>"}` (no `skill` field) were *accepted* by the engine — the `tool_calls.output` recorded `Task submitted (long-running). ID: <child>`. Both subprocesses then exited 1, the parent task is stuck `blocked`, and the `validate_dispatch_readiness` global guard (mika#583) freezes the milestone#13 queue behind `mika#666`.

Either:
1. `required` is absent from the in-memory `input_schema` at validation time, OR
2. The dispatch input *does* satisfy the schema's `required` array (e.g. it predates `skill`)

PR #969's existing tests construct `input_schema` fixtures by hand. **No test exercises the production load path** (`skills/bundled/dev-pilot/tools.json` → embedded `BUNDLED_SKILL_MANIFESTS` → `seed_bundled_skills()` → on-disk per-agent skill dir → `load_tools_json()` → `ResolvedSkillTool.definition.input_schema`).

## Verified state (post-architect-pass-1)

The first-pass architect review (session `549b88bb-2436-4e87-94e5-5c67634d5aae`) flagged two blockers: (a) H1 status not updated post-verification, (b) no Phase 0 pin of the call chain. Both addressed below.

### H1 — FALSIFIED (verified 2026-05-06 during pass-1 brief preparation)

```
$ cat ~/.mika/agents/mika-dev/skills/dev-pilot/tools.json | jq '.[0].input_schema.required'
["skill","prompt","task_id"]   # ← correct

$ cat skills/bundled/dev-pilot/tools.json | jq '.[0].input_schema.required'
["skill","prompt","task_id"]   # ← matches

$ cat /proc/8821/environ | tr '\0' '\n' | grep -i bundled
                               # ← empty: MIKA_DISABLE_BUNDLED_SKILLS not set
```

The on-disk per-agent file for the dispatching agent (mika-dev) has the post-`06dc9e40` shape. Re-seed env var is unset. **The plan's original Step 2A does not apply.**

### Phase 0 — Call chain pin (architect blocker #2)

`run_claude_pilot` dispatch path from agent loop to subprocess spawn:

| # | Site | Behavior |
|---|------|----------|
| 1 | `agent.rs:2650` (`dispatch_to_skill`) | Calls `executor::execute_skill_tool(skill_tool, input, ...)` for ALL skill tools (builtins/MCP routed elsewhere) |
| 2 | `executor.rs:192` (`execute_skill_tool`) | **PUBLIC, single entry point.** No alternative exists. |
| 3 | `executor.rs:202` | `validate_required_fields(skill_tool, &input)` — short-circuits with structured error if any required field is missing or null |
| 4 | `executor.rs:207–223` | Long-running gate: `ToolHandler::Exec { long_running: true }` + `Some(long_running_ctx)` → `execute_long_running(...)` |
| 5 | `executor.rs:229–247` | `long_running: true` + `None` ctx → returns the "Tool 'X' is declared long_running but cannot run in the current context (callback turn, silent mode, or CLI test)" error (the message we saw in the 22:31 callback retry) |
| 6 | `executor.rs:844` (`execute_long_running`) | **PRIVATE** (no `pub`). Single internal caller at line 214. Cannot be reached by bypassing line 202's validation. |

**Architect's F3 hypothesis ("long-running dispatch path bypasses `execute_skill_tool` entirely") is structurally invalid.** Verified by `grep -rn "execute_long_running\|skills::executor::execute_" crates/mika-agent/src/` — outside `executor.rs` itself, zero callers. The function is private and reachable only through line 214, which is gated by line 202's validation.

This rules out F3 but **deepens the puzzle**: validation IS in the dispatch path, was reached, and yet did not reject the malformed input. So the failure must be one of:

- **F4 — Schema in-memory mismatches schema on disk.** `load_tools_json` reads from disk at startup; the in-memory `ResolvedSkillTool.definition.input_schema` is what `validate_required_fields` reads. If a registry-assembly step between `load_tools_json` and `execute_skill_tool` mutates `input_schema.required`, validation operates on a different shape than the disk file shows. Candidate sites: `apply_overrides`, `apply_identity_allowlist`, `apply_agent_tool_visibility`, `inject_skills_and_resolve_tools`, `inject_task_id_field`. The plan's verification chain (§ Verification chain) shows `inject_task_id_field` only appends, never removes — but the other layers haven't been audited line-by-line.
- **F5 — Input mutation between LLM emission and validation.** The `input: serde_json::Value` parameter to `execute_skill_tool` is what validation reads. If something rewrites `input` between the LLM's tool_use block and the call site, validation's view differs from `tool_calls.input` (which is saved later). The saved input shows no `skill` field; validation might have seen one (or vice versa). This is testable by adding a structured-log emission inside `validate_required_fields` capturing both the input keys and the schema's required array — that single-line trace pins the layer.
- **F6 — Race / non-deterministic schema source.** Multiple agents on the same server share a `SkillRegistry` per-agent (per CLAUDE.md). If a registry rebuild fires concurrently with a dispatch (e.g., `skills_dirty` flag + hot-reload from `handlers.rs`/`a2a.rs`), the schema read could be from a half-rebuilt state. Lower probability — there's no reason to think a rebuild fired today — but worth ruling out via timestamp comparison if F4/F5 don't conclude.

The plan's Steps 2A/2B below are kept intact for audit-trail clarity, but **execution will skip Step 2A** (H1 false) and treat Step 2B as the entry to the F4/F5/F6 trace described in Phase 0.5 below.

### Phase 0.5 — Active investigation surface (replaces former Step 2B)

**Prerequisite (operational, per pass-2 U2):** before the F5 trace lands, cancel mika#666's wedged callback child task `723937bf` via `mika tasks cancel 723937bf` to release the `validate_dispatch_readiness` global dispatch slot. The F5 trace requires a dispatch attempt to fire; mika#666 is the deterministic reproducer; the next `ready`-label re-application after fix-deploy needs a clean queue head. Cancellation is in-scope as a one-line operational prerequisite; it is not a separate ticket.

1. **F5 first** (cheapest): add a `tracing::warn!` at the top of `validate_required_fields` emitting:
   - `tool_name`
   - `input_keys = input.as_object().map(|o| o.keys().collect::<Vec<_>>()).unwrap_or_default()`
   - `required_raw = skill_tool.definition.input_schema.get("required").cloned()` (the **pre-`as_array()`** value, per pass-2 F7)
   - `required_fields_observed` (the post-`as_array()` parsed Vec)

   Logging the raw `required` value before `as_array()` distinguishes "schema lacks `required`" (raw is `None`) from "schema has malformed `required`" (raw is `Some(non-array)` — silently passes today, see Step 3.5) from "schema is correct, input is malformed" (raw is `Some(array(...))`, parsed Vec matches, input lacks `skill`). The single line yields a three-way diagnosis on first fire.

   Build, deploy, re-trigger via `gh issue edit 666 --repo senara-solutions/mika --add-label ready` (the operational prerequisite above must already be in place). The bug is deterministic for mika#666.

2. **F4 second** (if F5 inconclusive or points at registry-mutation):
   - **U1 check first (cheapest, per pass-2):** assert `dev-pilot` is the unique registrant of the tool name `run_claude_pilot` — `grep -rn '"run_claude_pilot"' skills/bundled/*/tools.json` and check the `SkillRegistry` at runtime via `mika skills list --json`. If a second registrant shadows `dev-pilot` with a different schema, `validate_required_fields` would read the shadow's `input_schema` while we audited dev-pilot's. This is a degenerate F4.
   - Then walk `apply_overrides`, `apply_identity_allowlist`, `apply_agent_tool_visibility`, `inject_skills_and_resolve_tools`, and `inject_task_id_field` line-by-line for any write to `input_schema` or its `required` array. Pin the drop site or rule out registry mutation entirely.

3. **F6 last** (if F4/F5 both inconclusive): correlate dispatch timestamps against registry-rebuild timestamps in `server.log` (`grep -E "skills.*reloaded|registry.*rebuild" /var/log/mika/server.log`).

**Instrumentation lifecycle (per pass-2 U4):** the F5 `warn!` ships in this PR and stays in code. Level is `DEBUG` for the rapid-path "schema correct, input correct, required fully present" case (silent in production), and `WARN` for any other shape (rare; surfaces immediately if a future regression in this class lands). The split is one-line at the bottom of `validate_required_fields`: `if all_required_fields_present_and_input_complete { tracing::debug!(...) } else { tracing::warn!(...) }`. Removing the instrumentation post-fix would leave the next regression invisible; keeping it adds zero noise on the happy path.

The original Step 2B's "compare via `mika skills info dev-pilot --json`" approach is preserved as a complementary check for F4 — diff the on-disk per-agent state against the in-memory registry view at runtime.

### Step 3.5 — Validator hardens against malformed schema (per pass-2 F7)

Today `validate_required_fields` reads `required` via `.as_array()`. If `required` is present but not an array (malformed schema — `null`, an object, a string), `as_array()` returns `None`, the existing path emits a `skill_tool_malformed_schema_skipped_validation` warn, and the function **silently returns `None` (passes)**. This is the second silent-pass mode in the same function: missing `required` (intentional, schemas without required-fields are valid) AND malformed `required` (a real bug we should reject) both end at the same return point.

**Change:** split the two cases. Missing `required` (key absent) → return `None` as today. Present but non-array → return `Some(ToolOutput::error(...))` with a `malformed_required_schema` structured error and the same WARN log. The implementer touches `validate_required_fields` directly; the change is ~6 lines (replace the silent fall-through after the existing warn with an explicit error return).

This is defense-in-depth — it doesn't fix the production bug (where `required` is correctly an array, per the source file we inspected) but it closes the second silent-pass gap so any future schema-shape drift surfaces as a structured tool error instead of a silent accept.

## Root-cause hypotheses (ranked, original)

The ticket lists "load-path serialization drops `required`" as the leading hypothesis. Tracing the code (see § Verification chain below) does not support that — every layer preserves `serde_json::Value` and `inject_task_id_field` only appends to `required`, never removes. Two stronger hypotheses survive:

### H1 — Stale on-disk `tools.json` from before `skill` became required (most likely)

Commit `06dc9e40` (2026-04-28) added `"skill"` to `required` in `skills/bundled/dev-pilot/tools.json`, changing it from `["prompt","task_id"]` to `["skill","prompt","task_id"]`. Bundled skills are re-seeded on startup by `seed_bundled_skills_if_needed()` in `crates/mika-agent/src/startup.rs:33`, which writes the embedded content over the agent's per-agent skill dir at `~/.mika/agents/<name>/skills/dev-pilot/tools.json`.

`MIKA_DISABLE_BUNDLED_SKILLS=true` skips the re-sync entirely (documented escape hatch). If the running mika-server starts with that env var set — the operator hot-patches workflow per `project_skill_propagation_lock.md` notes — the on-disk `tools.json` is whatever was there at provisioning time. An agent provisioned before 2026-04-28 still has `required: ["prompt","task_id"]`, the dispatch supplies both, validation correctly passes, and the subprocess crashes downstream because `dispatch-lib.sh` post-#06dc9e40 does require `--skill`.

This matches every piece of evidence in the ticket: binary is post-#969, on-disk source is post-#06dc9e40, but the *agent's* tools.json is pre-#06dc9e40. The ticket's "regression is in the load-path serialization" framing collapses into "the load path reads stale state on disk that the build-time embed doesn't get to overwrite."

### H2 — `inject_task_id_field` interaction or input-schema mutation in registry assembly

The schema observed by `validate_required_fields` is the post-`inject_task_id_field` schema (line 1748). The injection only ever *appends* `task_id` to `required` (line 1387–1394). But there are downstream assembly steps in the registry — `apply_overrides`, `apply_identity_allowlist`, the LLM-tool-array filter — and any of them rewriting `input_schema` (or its `required` array) would land here. Lower probability than H1 but cheap to rule out.

### H3 — Provider-side serialization path (sibling to AC #4)

`run_claude_pilot` is invoked by the LLM through whatever Rust → provider conversion produces the Anthropic-compatible tool definition. If that conversion strips `required`, the LLM never receives schema-level enforcement and freely emits dispatches without `skill`. **This is orthogonal** to the engine-side `validate_required_fields` check (which reads from `input_schema` directly, not from a provider-converted shape) but it is part of AC #4 and a real defense-in-depth gap.

## Verification chain (built during plan drafting)

| Layer | File:line | Behavior on `required` |
|---|---|---|
| Source on disk | `skills/bundled/dev-pilot/tools.json` | Has `["skill","prompt","task_id"]` (post-`06dc9e40`) |
| Build-time embed | `crates/mika-agent/build.rs:118` (`generate_bundled_skills_table`) → `build_support/bundled_skills_discover.rs` | `include_str!`s file content verbatim — bytes preserved |
| Static table | `crates/mika-agent/src/bundled_skills.rs:147` (`include!(... "/bundled_skills_generated.rs")`) | Embedded as raw `&'static str` |
| Per-agent seed | `crates/mika-agent/src/bundled_skills.rs:252` (`seed_bundled_skills`) called by `crates/mika-agent/src/startup.rs:44` | `std::fs::write` of embedded bytes — preserved unless `MIKA_DISABLE_BUNDLED_SKILLS=true` skips it |
| On-disk per-agent | `~/.mika/agents/<name>/skills/dev-pilot/tools.json` | **Read at runtime by `load_tools_json`** |
| Runtime parse | `crates/mika-agent/src/skills/index.rs:1716` (`serde_json::from_str::<Vec<SkillToolDef>>`) | `SkillToolDef.input_schema: serde_json::Value` — preserved |
| Schema mutation | `crates/mika-agent/src/skills/index.rs:1748` (`inject_task_id_field`) | Appends `task_id` to `required` if missing; never drops |
| `ResolvedSkillTool` | `crates/mika-agent/src/skills/index.rs:1751` | Schema stored as-is |
| Validation entry | `crates/mika-agent/src/skills/executor.rs:202` (`validate_required_fields`) | Reads `input_schema.get("required").as_array()` |

The only place this chain *can* drop `required` between source and validation is the on-disk per-agent file when re-seed is suppressed. Hence H1's prominence.

## Acceptance criteria (from ticket, restated for plan)

1. **Reproduce.** Emit `run_claude_pilot` with `{"prompt":"x","task_id":"<uuid>"}` (no skill). Engine returns structured `missing_required_field` error WITHOUT spawning a subprocess.
2. **Trace the load path.** Document the layer that loses `required`. After this plan's investigation, document either (a) the registry-assembly drop site if H2 holds, or (b) the operational stale-on-disk gap if H1 holds.
3. **End-to-end test (cross-layer assertion).** Load the production-shape `dev-pilot` manifest through the *real* `load_tools_json` (filesystem read, not hand-constructed `serde_json::Value`) and assert `input_schema.get("required") == Some(array(["skill","prompt","task_id"]))` after `inject_task_id_field` runs.
4. **Provider-format serialization test.** Assert the Rust → Anthropic tool-definition conversion preserves `required` so LLM-side enforcement also works.
5. **Re-run #969 unit tests against the production-loaded schema** (not the hand-constructed fixture).
6. **End-to-end verification.** Re-dispatch `mika#666`: dispatch must succeed end-to-end OR be rejected synchronously with `missing_required_field`.

## Plan

### Step 1 — Reproduce on fresh agent (pure observability, no code)

Goal: distinguish H1 from H2/H3 *before* writing any test or fix.

1. On the running mika-server's host, capture:
   - `env | grep MIKA_DISABLE_BUNDLED_SKILLS` from the running process (`/proc/<pid>/environ | tr '\0' '\n' | grep BUNDLED`)
   - `cat ~/.mika/agents/mika/skills/dev-pilot/tools.json | jq '.[0].input_schema.required'`
   - `cat skills/bundled/dev-pilot/tools.json | jq '.[0].input_schema.required'` (in the deployed source tree)
2. Compare the three values.
   - If on-disk per-agent `required` is `["prompt","task_id"]` and source is `["skill","prompt","task_id"]` → **H1 confirmed.**
   - If on-disk per-agent `required` is `["skill","prompt","task_id"]` → H1 falsified, escalate to H2 (registry assembly trace).
3. Capture the exact bytes for the fix-confirmation regression test (no inference; copy from the actual file).

**Output of Step 1:** a one-line verdict in the issue comment ("H1 confirmed: per-agent tools.json predates 06dc9e40, MIKA_DISABLE_BUNDLED_SKILLS=true on PID <n>" or equivalent). This determines which Step 2 branch runs.

### Step 2A — DOES NOT APPLY (H1 falsified, see Verified state above)

The original Step 2A is preserved below for audit-trail clarity. Its operational unblock for mika#666 (Step 2A.1) does not apply because the on-disk file is correct. The structural drift-detection (Step 2A.2) and cross-layer test (Step 2A.3) are still worth shipping as defense-in-depth — they catch the H1-class regression that *would* hit if any operator ran with `MIKA_DISABLE_BUNDLED_SKILLS=true` against a fresh post-#06dc9e40 source. **Steps 2A.2 and 2A.3 ship as part of this PR** even though H1 isn't the active root cause; deletion would just leave the regression vector unguarded.

(original content of Step 2A retained verbatim:)

#### Step 2A — If H1 holds: close the stale-on-disk gap

The fix is operational *and* structural; both, not either. The validate-fields runtime check is correct as-shipped but is rendered useless by stale on-disk state. Three layered actions:

1. **Operational unblock for `mika#666`** (no code):
   - With mika-server stopped, `rm -rf ~/.mika/agents/mika/skills/dev-pilot` then restart with `MIKA_DISABLE_BUNDLED_SKILLS` *unset* so `seed_bundled_skills` repopulates from the post-#06dc9e40 embedded content.
   - Verify `cat ~/.mika/agents/mika/skills/dev-pilot/tools.json | jq '.[0].input_schema.required'` returns `["skill","prompt","task_id"]`.
   - Cancel the stuck `mika#666` callback child task via `mika tasks cancel <child-id>` so `validate_dispatch_readiness` releases the queue.
   - This unfreezes milestone#13 immediately. The runtime validation in #969 takes effect from the next dispatch.

2. **Structural — content-hash drift gate on `MIKA_DISABLE_BUNDLED_SKILLS`** (`crates/mika-agent/src/startup.rs:33`) (per pass-2 U3 — content-hash chosen over schema-version):
   - Embed a per-skill content sha256 at build time (sibling field on `BundledSkill`, computed by `build.rs` over the concatenation of `skill.toml` + `tools.json` + `system_prompt.md`). Cheaper than maintaining a manual `[skill] schema_version` field — every meaningful change to a bundled skill mutates the hash, no operator discipline required.
   - On startup, when re-seed is *disabled*, compute the on-disk content hash and compare to the embedded one. If they diverge, emit a `bundled_skill_drift` ERROR log (one line per affected skill, naming the skill and showing both hashes truncated to 12 chars).
   - Do NOT force-re-seed when the operator explicitly opted out — that breaks the documented hot-patch workflow. The point is *visibility*, not auto-correction. The error log is the defense.
   - Add a `mika skills doctor` subcommand (or extend `mika status`) that surfaces the same drift on-demand without restart.

3. **Structural — cross-layer test (AC #3)**:
   - New integration test at `crates/mika-agent/tests/skills_load_path.rs` that:
     - Calls `seed_bundled_skills(tmp.path())` against the *real* embedded `dev-pilot` content.
     - Calls `load_tools_json(tmp.path().join("dev-pilot"))`.
     - Asserts the resulting `ResolvedSkillTool.definition.input_schema.required` contains exactly `{"skill","prompt","task_id"}` as a set (using set-equality, not array-equality, since `inject_task_id_field` may append).
   - This is the test PR #969 missed: it exercises the load path end-to-end against the *actual* file, not a fixture.

### Step 2B — If H1 falsified, switch to H2 (registry assembly trace)

If on-disk `required` is correct but validation still no-ops, the loss happens between `load_tools_json` and `validate_required_fields`. Steps:

1. Insert a one-shot debug log at `executor.rs:117` (right before the `.get("required")` read) emitting `tool_name`, `input_schema_keys`, and the raw `required` value. Build, deploy, re-trigger the dispatch, capture the log.
2. Compare against the schema returned by `load_tools_json` — read the registry directly via `mika skills info dev-pilot --json` (or add such a CLI surface if absent) and diff.
3. The drift point is between those two reads. Likely candidates: `apply_overrides`, `apply_identity_allowlist`, `apply_agent_tool_visibility`, the LLM-tool-array conversion (`inject_skills_and_resolve_tools` per `crates/mika-agent/CLAUDE.md`).
4. Fix is local to whichever layer is mutating the schema. Add an integration test asserting `required` survives that layer's transform.

### Step 3 — Provider-format serialization test (AC #4, both branches)

Independent of H1/H2 outcome:

1. Locate the Rust → provider tool-conversion site (search `LlmToolDefinition` and `tools` field assembly in `crates/mika-common/src/`).
2. Add a unit test that:
   - Constructs a `ResolvedSkillTool` with `input_schema.required = ["a","b"]` and at least one `properties` entry.
   - Runs the conversion to `LlmToolDefinition` (or the Anthropic-shape struct).
   - Serializes via `serde_json::to_value`.
   - Asserts the serialized JSON contains `input_schema.required == ["a","b"]`.
3. If the conversion drops `required`, fix it. If it preserves it, the test is the regression guard.

### Step 4 — Reproducer test (AC #1)

1. Add a focused `executor.rs` test (alongside #969's existing tests) that:
   - Loads the *real* dev-pilot `ResolvedSkillTool` via `load_tools_json` (factor a test helper that reads the embedded content into a tmpdir).
   - Calls `execute_skill_tool` with `input = {"prompt":"x","task_id":"<uuid>"}` (no `skill`).
   - Asserts the returned `ToolOutput` is an error with `"missing_required_field"` and `"field":"skill"`.
   - Asserts no subprocess is spawned (`long_running_ctx = None` or check no spawn-side-effect — the structural answer is that `validate_required_fields` runs *before* the long-running gate, so a None ctx is sufficient as the "would have spawned" sentinel).

### Step 5 — Re-validate #969 tests (AC #5)

1. Re-run `cargo test -p mika-agent skills::executor` — the existing #969 unit tests should still pass.
2. The new test from Step 4 is what closes the gap PR #969 missed.

### Step 6 — End-to-end verification (AC #6)

1. After deploying, dispatch `mika#666` once via `mika ask --agent mika-dev "implement mika issue#666"` (or apply the `ready` label).
2. Watch the engine logs:
   - If the dispatch input includes `skill: "dev-pilot"`, expect a successful subprocess spawn and PR within the autonomous loop's normal window.
   - If the LLM omits `skill`, expect a structured `missing_required_field` ToolOutput in the same turn — not a subprocess crash, not a frozen queue.

## Out of scope (explicit, mirrors ticket)

- Same-shape-recently-failed dedup guard in `validate_dispatch_readiness` — file as P1 follow-up.
- Reframing engine's "Task submitted" return value before subprocess liveness — see mika#980.
- Strict FIFO milestone queue semantics — separate scope.
- Schema-version drift visibility for *all* bundled skills — Step 2A.2 narrows to dev-pilot/run_claude_pilot for this PR; broader rollout is a follow-up.

## Risk and reversibility

- **Step 2A.1 (operational unblock)** is destructive — `rm -rf` of an agent skill dir. Mitigation: only the dev-pilot subdir, only after Step 1 has captured the existing content for the regression test, and only after stopping mika-server (so no in-flight dispatch loses its tool definition mid-turn).
- **Step 2A.2 (drift detection)** is non-destructive — log-only, opt-in via existing env var.
- **Step 2A.3 / Step 4 / Step 3 (tests)** are pure additions. No production behavior change.
- **Step 2B (debug log + trace)** is one-line debug log on a hot path; remove before merge.

## Files touched (estimated)

- `crates/mika-agent/src/startup.rs` — add drift-detection in `seed_bundled_skills_if_needed` (Step 2A.2)
- `crates/mika-agent/src/bundled_skills.rs` — add `schema_version` field on `BundledSkill` and helper for on-disk comparison (Step 2A.2)
- `crates/mika-agent/tests/skills_load_path.rs` — new integration test (Step 2A.3)
- `crates/mika-agent/src/skills/executor.rs` — add real-file reproducer test (Step 4)
- `crates/mika-common/src/...` (TBD) — provider-format serialization test (Step 3)
- (if H2) the layer that drops `required` — single-file fix, scope determined at trace time

## What this plan deliberately does NOT do

- Does NOT rewrite `validate_required_fields` itself. The function is correct given a correct schema. Defending it from a bad schema is a different (broader) discipline — schema validation at startup, which is its own ticket.
- Does NOT add JSON Schema-level type/enum validation in the validator. AC #4 is about provider-side enforcement; engine-side validation stays scoped to `required`-presence (the explicit #955 scope).
- Does NOT consolidate `MIKA_DISABLE_BUNDLED_SKILLS` semantics. The escape hatch stays. Visibility, not removal.
