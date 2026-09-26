# Fabrication Guard Tool-Gating Audit

**Date:** 2026-05-28
**Issue:** mika#1254
**Scope:** `crates/mika-agent/src/agent.rs` — all post-condition guards that use tool-presence predicates

## Summary

Five guards in the `run_loop` post-condition chain were audited for tool-presence gating patterns (`enabled_tool_names.contains(...)`, `tools.get(...)`, or `available_tool_names` usage). Each is classified as:

- **(A)** Fire regardless — output shape is fabrication if tool unreachable
- **(B)** Stay gated — tool-presence check is correct for the guard's purpose; document why
- **(C)** Refactor — replace with a more precise predicate

## Guards Audited

### 1. Dev-groom fabrication guard (position 5b, #1133)

**Line:** ~1527
**Predicate:** `enabled_tool_names.contains("run_claude_pilot_groom")`
**Purpose:** Detects `Verdict: GROOMED` / `Verdict: ESCALATE` text without a satisfying `run_claude_pilot_groom` call.

**Classification: (C) Refactor**

**Rationale:** The guard uses tool presence as a proxy for role discrimination (dispatcher vs. producer). When the tool is absent — due to loader bug (mika#1251), identity allowlist denial, or bundled-skill exclusion — the guard silently skips. This is the exact scenario where fabrication risk is highest: the LLM cannot dispatch but claims a verdict anyway.

**Fix:** Replace `enabled_tool_names.contains("run_claude_pilot_groom")` with `!is_verdict_producer`, where `is_verdict_producer` is a pre-computed boolean derived from the skill registry. Known verdict-producer skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) are exempted; all other agents have the guard active. The guard now fires when the tool is absent (correct) and skips only for agents whose role is to *produce* verdicts (correct).

### 2. Asserted-unavailability guard (position 6c, #862)

**Line:** ~1704–1710
**Predicate:** `detect_asserted_unavailability(&text, enabled_tool_names)` + `asserted_unavailability_satisfied(&tool_name, enabled_tool_names, &all_tool_summaries)`
**Purpose:** Catches assistant text claiming a tool is unavailable when it IS in the enabled set.

**Classification: (B) Stay gated — correct by construction**

**Rationale:** `enabled_tool_names` is used as *ground truth for the check*, not as a gate on it. The guard asks: "Is the LLM lying about tool availability?" The answer requires knowing what tools are actually available — `enabled_tool_names` provides that. If a tool is NOT in `enabled_tool_names`, the LLM's claim of unavailability is *correct*, and the guard rightly does not fire. The tool-presence predicate IS the check, not a gate before a check.

Reviewed in mika#1254 audit. No change needed.

### 3. Completion-claim guard (position 4, #483)

**Line:** ~5277
**Predicate:** `tools.get("update_task_status").is_none()`
**Purpose:** Detects completion-claim keywords ("merged", "deployed", "completed") without an `update_task_status` call.

**Classification: (B) Stay gated — correct for different reasons**

**Rationale:** This guard is a *nudge for task hygiene*, not a security boundary. Delegates and team agents legitimately lack the `update_task_status` tool (they receive `default_tools()` only). The failure mode when the guard skips (missed nudge) is benign — no fabrication risk, no integrity violation. The guard gates on tool registry presence (`tools.get()` — the full registry, not just enabled names), which is the correct scope: if the agent cannot call the tool, nudging it to do so would be a false positive.

### 4. Prose-style tool call detection (position 2, #569)

**Line:** ~1120–1126
**Predicate:** `detect_prose_style_tool_call(&text)` (uses `available_tool_names` internally)
**Purpose:** Catches `tool_name({"key": "val"})` patterns where the identifier matches a registered tool.

**Classification: Not applicable (false-positive filter, not security gate)**

**Rationale:** The tool-name check exists to *reduce false positives* (avoid matching code examples or prose that happens to look like a function call). It is a quality-of-match filter, not a security predicate. Without the tool-name check, the guard would be too aggressive — a different failure mode from the fabrication-bypass anti-pattern. No change needed.

### 5. Milestone-close-claim guard (position 4b, #797, #1207)

**Line:** ~1350+
**Predicate:** No tool-presence gating.
**Purpose:** Detects "I closed milestone" claims without a satisfying `run_gh` PATCH call.

**Classification: Not applicable (already correct)**

**Rationale:** This guard fires unconditionally on milestone-close claims — it does not check whether `run_gh` is in the tool set. The guard is already shaped correctly for the fabrication-prevention purpose.

## Related: mika#940

**Checked.** mika#940 is about premature EndTurn detection via the text-based tool call detector (#569, position 1–2). It uses `available_tool_names` to avoid false positives, not as a security gate. Same classification as guard #4 above (Not applicable). No change needed.

## Action Items

| Guard | Classification | Action |
|-------|---------------|--------|
| Dev-groom fabrication (5b) | C | Invert predicate: `!is_verdict_producer` (Unit 2–3) |
| Asserted-unavailability (6c) | B | Add inline comment (Unit 4) |
| Completion-claim (4) | B | Add inline comment (Unit 4) |
| Prose-style tool call (2) | N/A | No change |
| Milestone-close-claim (4b) | N/A | No change |
