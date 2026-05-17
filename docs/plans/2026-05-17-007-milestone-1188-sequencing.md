---
title: "Milestone 1188 sequencing record"
type: milestone-sequencing
milestone: senara-solutions/mika#1188
github_milestone: senara-solutions/mika/milestone/28
date: 2026-05-17
status: active
---

# Milestone mika#1188 — Deprecate mika-relay (tier1 expansion + deterministic policy + retirement)

## Sub-issues

- mika#1191: feat(claude-pilot) port permission-policy TIER 1 rules into tier1.py (priority: **p1**, plan: `mika/docs/plans/2026-05-17-004-feat-1191-tier1-expansion-plan.md`, branch: `feat/1191/port-permission-policy-tier1-rules-into-tier1-py`)
- mika#1192: feat(claude-pilot) deterministic policy file replaces relay LLM call (priority: **p1**, plan: `mika/docs/plans/2026-05-17-005-feat-1192-deterministic-policy-file-plan.md`, branch: `feat/1192/deterministic-policy-file-replaces-relay-llm-call`)
- mika#1193: chore(mika) retire mika-relay agent + permission-policy skill + config refs (priority: **p2**, plan: `mika/docs/plans/2026-05-17-006-chore-1193-retire-mika-relay-plan.md`, branch: `chore/1193/retire-mika-relay-agent-permission-policy-skill`)

## Dependencies

- mika#1191 → mika#1192: Phase B's `permissions.py` rewrite assumes Phase A's expanded tier1 is in place — the policy-file lookup only fires for events tier1 doesn't auto-approve.
- mika#1192 → mika#1193: Phase C deletes `transport.py` and the 5 `.claude/claude-pilot.json` files. Phase B's B-AC5 verifies `transport.py` handles missing config gracefully — that's the safety net Phase C depends on.

## Recommended GitHub `blockedBy` edits

- mika#1192 blockedBy mika#1191: tier1 expansion must merge first — file via `gh issue edit 1192` with a `Blocked by #1191` body callout.
- mika#1193 blockedBy mika#1192: deterministic policy must be live in production ≥7 days before relay agent can be deleted — file via `gh issue edit 1193` with a `Blocked by #1192` body callout.

## Order

1. **mika#1191** (Phase A) — ships first; standalone value (≥80% relay-call elimination, ≥5× latency drop on tier1-resolved events).
2. **soak ≥3 calendar days** after #1191 merge — verifies no regression in tier1 expansion under autonomous-loop traffic.
3. **mika#1192** (Phase B) — depends on #1191 merged + soaked. 100% relay-call elimination from claude-pilot dispatch path. New `mika notify` CLI verb.
4. **soak ≥7 calendar days** after #1192 merge — verifies deterministic policy + `mika notify` escalation path are stable.
5. **mika#1193** (Phase C) — depends on #1192 merged + soaked. Deletes `mika-relay` agent, `permission-policy` skill, `transport.py`, all 5 `.claude/claude-pilot.json` files, DB cleanup migration (schema v36).
6. **post-deploy soak ≥7 calendar days** after #1193 merge — verifies zero `mika-relay` activity in production (closes original milestone-level AC4 "no fabrication failures").

No parallel sets. Strict serial — A→B→C — is the only correct ordering per the cross-cutting concern analysis (every phase modifies `permissions.py` in different sections; each PR rebases onto the prior phase's merged state).

Soak windows are **calendar days** (not business days). Autonomous-loop runs 24/7; weekend traffic is real traffic.

## Cross-cutting concerns

| Concern | Sub-issues affected | Mitigation |
|---|---|---|
| `claude-pilot-py/src/claude_pilot/permissions.py` modified in all three phases | #1191, #1192, #1193 | Strict A→B→C sequencing eliminates parallel-edit merge conflicts. Each PR rebases onto prior phase's merge. |
| `claude_pilot/transport.py` lifecycle: present (A) → dead-code-gated (B feature flag) → deleted (C) | #1192 (NF7 verifies graceful-fallback), #1193 (deletion) | Phase B's B-AC5 is the safety net for Phase C's deletion. |
| `permission-policy/system_prompt.md` is canonical until Phase A; duplicate-of-tier1 between A and C; deleted in C | #1191, #1193 | Phase A's PR description includes a "system_prompt.md is documentation; tier1.py is canonical" callout. Drift is bounded by the soak windows (≤14 calendar days). |
| 5 copies of `.claude/claude-pilot.json` untouched until Phase C | #1193 | Phase C's PR enumerates per-file diff before deletion; covers inter-soak drift if any operator edits one. |
| Cross-language `INTRA_PLATFORM_AGENTS` sentinel (Rust comment + Python const) | #1191 (creates Python side), #1193 (updates comment prose to drop "mika-relay" mention) | Sentinel contract (5-entry threshold, codegen escalation) preserved across phases. Future automation TBD — out of milestone scope. |
| `MIKA_PILOT_POLICY_DISABLED` env var introduced in B, removed in C | #1192, #1193 | Phase C's PR runs production-grep before merge; if flag is set anywhere, removal becomes a blocker requiring operator coordination. |
| DB migration to schema v36 (Phase C only) | #1193 | Idempotent reverse-dependency-order DELETEs (not cascade-reliant). `MIKA_MIGRATION_CONFIRMED=1` env-var gate ensures explicit operator opt-in before destructive run. |

## Open milestone-level questions

1. **External second-pass routing.** Architect ratified routing the Phase 5 second-pass verdict to Vincent (external) — `mika-relay` is a `WELL_KNOWN_AGENTS` peer to `mika-arch`; removing it is a multi-system structural change.
2. **Cross-language sentinel automation (out of scope, follow-up worthy).** The `INTRA_PLATFORM_AGENTS` sentinel currently has no automated drift check. At 3 entries (well below 5-entry codegen-escalation threshold), the manual review is proportionate. File a follow-up ticket if/when the list grows.
3. **mika#1188 body Description framing.** Architect-noted NF1 — top-of-body Description was already corrected via Errata callout during initial body rewrite. Architect's observation was based on stale tool_history; the body is currently correct. No action.

## Status updates

<!-- Append-only log of changes after the record is committed. -->

- **2026-05-17 — initial record committed (post mika-arch first-pass READY on session `d22526a1`).** Pending Vincent's external second-pass verdict per Phase 5 of `/mika-groom-milestone`.
