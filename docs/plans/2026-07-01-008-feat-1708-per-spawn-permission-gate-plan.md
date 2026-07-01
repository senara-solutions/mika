---
type: feat
issue: 1708
title: Per-spawn permission-policy gate — Option C implementation (mika#1686 architect-ratified design)
status: draft
---

# Plan — mika#1708 per-spawn permission-policy gate (Option C)

## Ticket + design

mika#1708 — implementation of Option C from mika#1686. **Design already ratified by architect** session `22d21b66-eacd-4120-bb0a-cc11ce5b4f5d` (2026-07-01 ~11:35Z) after Prime-ruled C 2026-07-01 ~10:40Z.

**Full design lives in the ticket body** (`mika/docs/plans/2026-07-01-008-feat-1708-per-spawn-permission-gate-plan.md` and cross-references mika#1708 body). This plan file is the implementation sequencing — the design itself is not relitigated here.

## Problem (brief — full context in ticket body)

Current cpp permission-policy uses syntactic pattern-matching on shell text (`tier1.py` + `permissions.yaml`). n=13+ blocks in 2 days on legitimate compositional shell. Move gate to per-spawn evaluation at invocation-time.

## Committed positions (from architect-ratified design)

Copied here for plan-on-branch coherence — see mika#1708 body for the full rationale:

1. **Decomposition: `bashlex` upfront-parse.** Python-native. Predictable, testable. Fail-safe DENY on unsupported constructs (heredocs, process substitution, backticks, arithmetic expansion).
2. **State-tracking: `cwd_stack`.** cd is NOT a no-op — mutates state for subsequent spawns. Also tracks `export`/`unset`/`alias`/`unalias`/`set`/`shopt`. Rejects `eval`/`source`/`.` (dynamic execution).
3. **Rule shape: allowlist-by-binary + per-binary safety functions.** `POLICY[binary] = is_safe_<binary>(argv, cwd) -> bool`.
4. **Migration: 3 phases with measurable thresholds.** Phase 1 opt-in (`MIKA_PERMISSION_POLICY_MODE=per_spawn`). Phase 2 flip default after N=50 dispatches + zero blocks. Phase 3 retire classic after M=7 days + zero rollbacks.
5. **Fork strategy: (a) senara-solutions/claude-pilot-py-fork** — motto-aligned. **BUT Vincent-scope decision** per architect F4 pre-implementation gate.

## Fork strategy resolved 2026-07-01

Pre-implementation gate from prior architect F4 pass is **CLEARED**:

- Vincent made `senara-solutions/claude-pilot-py` **public** 2026-07-01 ~14:17Z. Repo pre-existed under this name; no separate `-fork` repo needed.
- SSC ratified OSS shape 2026-07-01 ~15:29Z: Apache 2.0, evaluator-only scope, upstream-first PR posture, boundary-at-commit-one (no Mika-specific policy contents in the public repo — those stay in the mika repo, loaded by cpp as config).
- Prime ruled 2026-07-01 ~15:00Z: (a) and (b) are one road with a merge point. Fork now (closes wedge today) AND open upstream PR to `anthropics/claude-agent-sdk` in parallel. Fork's success condition = its own retirement. (c) rejected.

**Upstream PR track lives on `senara-solutions/claude-pilot-py#67`** — blocked-by this ticket; opens once the evaluator lands on cpp `main`.

**Public-repo hygiene already applied** (commit `6dcbd62` on cpp main): Apache 2.0 LICENSE, repo description, three name-leak scrubs in plan docs.

## Implementation phases

**Phase 1 — bashlex integration on `senara-solutions/claude-pilot-py` (~1-2 days):**
- Land PR directly on `senara-solutions/claude-pilot-py` (public repo, no fork needed).
- Add `bashlex` dependency in `pyproject.toml`.
- Author `claude_pilot/per_spawn.py` — new module implementing the decomposition + state-tracking + per-binary evaluator. Does NOT touch existing `permissions.py` yet.
- **Evaluator-only scope (SSC boundary discipline):** the module provides the generic per-spawn engine API. No Mika-specific policy contents. Test fixtures use synthetic "shell command safety" patterns unrelated to Mika's actual deployment.
- Unit tests: decomposition (AC1), state-tracking built-ins (AC2), per-binary safety functions (AC3) — 30+ test cases covering supported + unsupported shell + built-in classifications.

**Phase 2 — mode selection + audit events (~1 day):**
- `claude_pilot/permissions.py` — read `MIKA_PERMISSION_POLICY_MODE` env var, dispatch to `tier1.py` (classic) or `per_spawn.py` (new) — default classic (AC4).
- Emit `audit_events` (kind = `perm_policy_mode`) on every dispatch (AC5).
- Rollback trigger: on any per_spawn permission-policy block, emit `perm_policy_rollback` audit event (AC6). Global env-var flip logic in mika-spirit (not cpp — cpp side is stateless per-spawn evaluation).

**Phase 3 — mika-side wiring (~0.5 day):**
- Update `MIKA_PERMISSION_POLICY_MODE` env var recognition in `crates/mika-common/src/settings.rs`.
- Update mika-spirit to write env var back to `~/.mika/.env` on rollback trigger + emit audit events.

**Phase 4 — docs + integration test (~0.5 day):**
- `crates/mika-agent/CLAUDE.md § permission-policy` — describe classic vs per_spawn modes + migration triggers + rollback procedure (AC8).
- `claude-pilot-py/README.md` — mode selection guide.
- Integration test in `crates/mika-agent/tests/eval/` — mocked spawn event stream + assert deny/allow decisions (AC7).

**Phase 5 — ship + Phase 1 rollout (~0.5 day):**
- Merge Vincent-approved. Deploy. `MIKA_PERMISSION_POLICY_MODE=classic` default.
- Operator manually flips a canary dispatch to `per_spawn` via env override for one ticket, verifies audit event fires. Documented in PR body.

**Phase 6 — Post-ship: Phase 2 migration monitoring (ongoing):**
- Monitor audit_events. When N=50 + zero blocks, ratify with Vincent for default flip (AC9). NOT part of the initial dispatch — followup operator action after real-world validation.

## Verification

- **Unit tests:** decomposition + safety functions + built-in classifications — `pytest claude-pilot-py-fork/tests/test_per_spawn.py`.
- **Integration test:** `cargo test -p mika-agent --test perm_policy_integration` — mocked spawn stream + assert decisions.
- **Manual canary:** `MIKA_PERMISSION_POLICY_MODE=per_spawn mika ask --agent mika-dev "verify cd + grep + sed passes"` — command that used to block classic mode now passes; audit_event emitted.
- **Regression:** `MIKA_PERMISSION_POLICY_MODE=classic` still works, dispatches unchanged.
- **`cargo test -p mika-agent`** — clean.

## Risks (design-level, not implementation-level — design was ratified)

1. **bashlex library maintenance.** External dependency. Version pin + audit for security. If library gets abandoned upstream, fork it too.
2. **Rollback semantics.** Global env var flip is drastic (all agents flip together). Alternative: per-agent env override. Architect noted global is appropriate for safety-critical policy failure.
3. **Recursion depth on command substitution.** `$(cmd $(inner_cmd))` recurses. Depth limit: 5 (arbitrary; catch pathological cases). Deny beyond depth 5 with diagnostic.
4. **cpp fork maintainability.** Vincent-scope per F4. If fork rots, upstream PR becomes the path even if slower.
5. **State-tracking edge cases.** `cd -` (previous dir), `cd ~` (home), `cd $VAR` (env expansion). Design coverage: only handle static paths in cd; deny variable-expansion paths as unsupported (fail-safe). Documented in AC1.

## Out of scope

- Actual denylist CONTENTS changes — Vincent-scope, separate ticket if any surface.
- `tier1.py` retirement — Phase 3 in the migration (post-ship, not part of this dispatch).
- ScheduleWakeup / non-Bash tool intercepts (mika#1687 territory).
- Kernel-level approaches (ptrace, LD_PRELOAD) — rejected in design.

## References

- mika#1686 — parent class question (Prime-ratified C 2026-07-01)
- mika#1708 — this ticket
- Architect design session `22d21b66-eacd-4120-bb0a-cc11ce5b4f5d` (2026-07-01 ~11:35Z, ratified)
- Prime session `00000000` ruling (2026-07-01 ~10:40Z)
- `claude-pilot-py/src/claude_pilot/permissions.py:69,80,318` — current invocation-time hook
- `claude-pilot-py/src/claude_pilot/tier1.py` — Phase 3 retire target
- `claude-pilot-py/src/claude_pilot/policies/permissions.yaml` — Phase 3 retire target
- [[project-mika-orchestrator-seat-prime-routing-pattern]] — the seat pattern
- Vincent direction 2026-07-01 morning "keep pushing" motto + "announced-yesterday"
