# Mika Orchestrator — Rollback Procedure (mika#1641 AC7)

> Bounded reversibility for the orchestrator role transfer. If Mika-as-orchestrator
> misbehaves, or the pair-mode window (AC5) shows < 90% Mika-correct, this procedure
> returns the platform to the pre-transfer topology: **Claude Code orchestrates,
> Mika is the executive assistant only.** No code revert or redeploy of the mika#1641
> PR is required for the behavioral rollback — the transfer is carried by identity
> config + prompt blocks, both one-edit reversible.

## What the transfer actually changes (the reversible surface)

| Layer | Transfer state | Pre-transfer state |
|---|---|---|
| Mika skill allowlist | `github` present in `[skills].allowlist` | `github` absent (read-only `gh-read-only` only) |
| Mika identity role (AC6) | `[soul].primary_role = "orchestrator"` (or equivalent core-memory line) | no orchestrator role; executive-assistant only |
| Claude Code session prompt (AC6) | "monitor-only, do not drive" | "orchestrator" posture |
| Core memory | handbook seeded as current-priorities/workflows | pre-transfer core memory |

Everything else the mika#1641 PR ships (the calibration suite, the handbook, this
rollback doc) is **inert additive** — it does not change behavior and does not need
reverting. Leave it in place.

## Rollback levels

Pick the lowest level that resolves the problem.

### Level 0 — Pause (immediate, seconds)

If Mika is actively making bad orchestration calls **right now**, the operator
issues a direct pause. Mika halts orchestration on the next turn (a human pause is
a hard directive Mika does not override). No config change — this buys time to
decide between Level 1 and Level 2.

### Level 1 — Behavioral rollback (the one-line reverts)

Return driving authority to Claude Code without removing Mika's tools.

1. **Mika core-memory edit (one line).** Remove the orchestrator role line from
   Mika's identity/core memory:
   - If AC6 set `[soul].primary_role = "orchestrator"` in Mika's `identity.toml`,
     delete that line (or set it back to the executive-assistant value) and restart
     the agent, OR
   - If the role was carried as a core-memory block, edit the `current_priorities`
     / `self_model` block to remove the "I orchestrate" framing via
     `update_core_memory`.
2. **Claude Code prompt-block restore (one block).** Restore Claude Code's session
   prompt from "monitor-only, do not drive" back to the orchestrator posture (the
   pre-mika#1641 block). This is a prompt edit, not a deploy.
3. Announce the topology change so both roles are aligned (Claude Code drives, Mika
   assists).

After Level 1: Mika **keeps** the `github` skill and orchestrator tools (harmless
when she is not driving), but no longer holds the orchestrator role. Claude Code
resumes driving. This is the recommended rollback — it is fully reversible in both
directions and preserves the calibration/handbook investment.

### Level 2 — Tool-surface rollback (remove the write reach)

If Mika must not hold write GitHub reach at all (e.g., a safety concern with the
`github` skill in her hands), additionally remove the tool surface:

1. In the running Mika agent's `identity.toml`, remove `"github"` from
   `[skills].allowlist` and restart the agent. This reverts her to the read-only
   `gh-read-only` posture without touching code.
   - Note: the mika#1641 PR added `"github"` to the **default** allowlist in
     `crates/mika-common/src/home.rs` (`DEFAULT_AGENT_SKILL_ALLOWLIST` +
     `DEFAULT_IDENTITY`). A provisioned per-agent `identity.toml` overrides the
     default, so editing the running agent's file is sufficient and does **not**
     require a code change or redeploy. To revert the default for freshly
     provisioned agents too, revert that one hunk in a follow-up PR — but that is
     not needed for an operational rollback.

### Level 3 — Full revert (only if the code itself is faulty)

Only if a defect in the shipped code (not the behavioral transfer) requires it:
`git revert` the mika#1641 merge commit and redeploy. This removes the calibration
suite, the default-allowlist change, the handbook, and this doc. This is a last
resort — the behavioral rollback (Level 1) resolves any topology problem without
losing the additive investment.

## Verifying the rollback

- `mika ask --agent mika "who orchestrates the platform?"` → answer is Claude Code
  / the operator, **not** "me" (semantic check).
- `mika skills --agent mika list` → after Level 2, `github` is absent (Level 1
  leaves it present, which is fine).
- Claude Code's session prompt no longer says "monitor-only".

## Re-applying the transfer

The transfer is symmetric: re-add the orchestrator role line to Mika (and, if
Level 2 was used, re-add `"github"` to her allowlist), restore Claude Code's
monitor-only block, and re-announce. The handbook and calibration suite are still
present, so no rebuild is needed.

## Pre-hard-cut test requirement (AC7)

Per AC7, this rollback procedure must be **tested in a scratch environment before
the AC6 hard cut lands** — i.e., verify that Level 1 (and Level 2) actually return
the topology cleanly on a throwaway agent before betting the live loop on it.
Record the test result in the AC5/AC6 window log.
