---
type: milestone
issue: mika#1188
title: "Milestone: deprecate mika-relay (tier1 expansion + deterministic policy + retirement)"
date: 2026-05-30
sub_issues:
  - mika#1191 (Phase A — tier1 expansion, CLOSED)
  - mika#1192 (Phase B — deterministic policy file, CLOSED)
  - mika#1193 (Phase C — retire mika-relay agent, OPEN)
---

# Milestone Plan: Deprecate mika-relay (mika#1188)

## Problem

`mika-relay` uses an LLM call to classify permission decisions that are fundamentally deterministic. This causes:

1. **Latency:** ~7s round-trip per permission event through Kimi-k2.6.
2. **Drift:** 14% of relay responses are prose instead of JSON (mika#1161), causing auto-deny or stall.
3. **Cost:** LLM calls for decisions that are 86% deterministic-shaped.

The corrected goal (per architect first-pass on session `a91a5323`) is to replace the LLM-based relay with deterministic in-process policy inside `claude-pilot`, not to rely on any SDK-built-in classifier (the SDK provides hooks, not classifiers).

## Strategy

Three-phase sequential migration with soak windows between phases:

```
Phase A (tier1 expansion)  →  merge  →  soak 3 days
Phase B (deterministic policy file)  →  merge  →  soak 7 days
Phase C (retire mika-relay)  →  merge  →  soak 7 days (milestone close)
```

Each phase is independently shippable with measurable improvement. Rollback is a single PR revert at each step.

## Phase Summary

### Phase A — Port permission-policy rules into tier1.py (mika#1191) ✅ CLOSED

**What shipped:** Expanded `claude-pilot-py/src/claude_pilot/tier1.py` with TIER 1, TIER 1.5, and TIER 3 rules ported from the `permission-policy` bundled skill's deterministic classification logic.

**Plan:** `docs/plans/2026-05-17-004-feat-1191-tier1-expansion-plan.md`

**Key changes:**
- Extended `is_tier1_auto_approve()` with rules from `permission-policy` skill's TIER 1 allowlists.
- Added TIER 1.5 patterns (context-dependent approvals, e.g., git operations in worktree).
- Ported TIER 3 deny patterns (destructive operations, secret exposure).
- Replay harness for validation against recent relay invocations.

**Acceptance met:** ≥80% of recent relay invocations resolved locally; ≥5× latency drop on tier1-resolved events.

### Phase B — Deterministic policy file replaces relay LLM call (mika#1192) ✅ CLOSED

**What shipped:** A `policies/permissions.yaml` policy file in `claude-pilot-py` that deterministically handles all permission events that tier1.py cannot classify. New `mika notify` CLI verb for escalation transport.

**Plan:** `docs/plans/2026-05-17-005-feat-1192-deterministic-policy-file-plan.md`

**Key changes:**
- `policies/permissions.yaml` — declarative permission rules for non-tier1 events.
- Policy engine in `claude-pilot-py` that loads and evaluates the YAML rules.
- `mika notify` CLI verb replaces relay-as-escalation-shim.
- `transport.py` modified to try local policy before relay fallback (graceful migration).

**Acceptance met:** Zero calls to `mika ask --agent mika-relay` in the claude-pilot dispatch path.

### Phase C — Retire mika-relay agent (mika#1193) 🔲 OPEN

**What remains:** Delete all code references to `mika-relay`, remove the `permission-policy` bundled skill, clean up `.claude/claude-pilot.json` configs across all repos, and run DB cleanup migration.

**Plan:** `docs/plans/2026-05-17-006-chore-1193-retire-mika-relay-plan.md` (on branch `chore/1193/retire-mika-relay-agent-permission-policy-skill`)

**Depends on:** Phase B merged + soaked ≥7 days.

**Key changes:**
1. `well_known_agents.rs` — remove `MIKA_RELAY` const, identity, soul, and provisioning path.
2. `skills/bundled/permission-policy/` — delete entire directory (build.rs auto-discovers, so removal is sufficient).
3. Well-known agent identity consts — remove `permission-policy` from disabled_skills lists.
4. `.claude/claude-pilot.json` across `mika-platform/`, `mika/`, `mika-skills/`, `mika-cloud/` — remove relay command/args entries.
5. DB cleanup migration — DELETE agent rows WHERE id = 'mika-relay' with cascade verification.
6. Documentation updates — update CLAUDE.md files, add deprecation callouts to historical docs.

**Acceptance criteria:**
- `rg -i "mika-relay"` returns zero hits in code (Rust, Python, TOML, JSON).
- All tests pass (`cargo test`, `uv run pytest`).
- `make deploy` builds cleanly; `mika status` shows no `mika-relay` agent.
- 7-day post-deploy soak: zero fabrication-class failures, zero relay messages.

## Sequencing Constraints

| Gate | Condition | Status |
|------|-----------|--------|
| A → B | Phase A merged + soaked 3 days | ✅ Met |
| B → C | Phase B merged + soaked 7 days | ⏳ Verify soak window before dispatching C |

**Before dispatching Phase C:** Verify Phase B's soak window (≥7 days since merge) by checking `gh pr list --repo senara-solutions/mika --state merged --search "1192"` merge date.

## Architect-Ratified Decisions

From milestone-level mika-arch first-pass (session `d22526a1`), ratified by operator second-pass:

1. **Premise correction:** SDK provides hooks, not classifiers. Real work is tier1 expansion + deterministic policy + retirement.
2. **Drop relay TIER 2** ("research and answer technical questions") with deny+escalate fallback — surface rarely used.
3. **Soak windows:** A→B: 3 days, B→C: 7 days.
4. **`mika notify` CLI verb** for Phase B escalation transport (over relay-as-shim or new HTTP endpoint).

## Non-Blocking Notes (Carried from Milestone Grooming)

- **NF5:** Phase A's replay-harness false-positive spec needs tightening (anti-undercount safeguards).
- **NF6:** Cross-reference `server/permission_pre_classifier.rs` from mika#935 for rule-parity (different layer, but rule shapes should align).
- **NF7:** Verify `transport.py` graceful-fallback on missing `.claude/claude-pilot.json` before Phase C ships.

## Milestone Acceptance Criteria

- **M-AC1.** ✅ All three sub-issues filed, sequenced, and linked under this parent.
- **M-AC2.** ✅ Each sub-issue groomed end-to-end with Phase 0 Pin and two-pass mika-arch review reaching GROOMED.
- **M-AC3.** ✅ After Phase A: median permission-event latency for tier1-resolved events dropped by ≥5×.
- **M-AC4.** ✅ After Phase B: zero calls to `mika ask --agent mika-relay` in dispatch path.
- **M-AC5.** 🔲 After Phase C ships + 7 days: no fabrication-class failures; `rg -i "mika-relay"` returns zero code hits.

## Risks

1. **Phase B soak window not met.** Phase C cannot ship until 7 days post-B-merge. Verify before dispatch.
2. **DB cascade misses tables.** Schema has 30+ tables; some may reference `agent_id` without explicit FK. Verify on copy of production data.
3. **Undiscovered relay consumers.** Grep for `--agent mika-relay` across all repos before Phase C merge. Any hit is a blocker.
4. **`transport.py` crash on missing config.** NF7 — if not handled gracefully, Phase C needs a `claude-pilot-py` code fix.

## Related

- mika#1161 — `mika-relay` drift on Kimi-k2.6 (superseded by this milestone)
- mika#935 — Rust pre-classifier (engine-side, rule-parity cross-reference)
- `docs/plans/721-dedicated-mika-relay-agent.md` — original plan that created `mika-relay`
