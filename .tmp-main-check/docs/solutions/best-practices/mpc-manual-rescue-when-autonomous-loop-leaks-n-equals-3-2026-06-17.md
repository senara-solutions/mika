---
module: orchestration, dispatch, substrate-engineering
tags: [autonomous-loop, manual-rescue, mpc-pattern, substrate-pivot, n-equals-3, dispatch-lib, content-monitor, settings-local-leak]
problem_type: best-practice
category: best-practices
date: 2026-06-17
ticket: control-monitor (founding case)
applies_when:
  - "Autonomous loop fails n=3 with the same observable pattern on a single repo/area"
  - "Substrate bug is identified but the slate of in-flight tickets is time-sensitive"
  - "Operator has explicitly authorized non-loop implementation paths for this scope"
resolution_type: discipline
---

# MPC manual rescue — when the autonomous loop leaks n=3 on a contained scope

## TL;DR

When the autonomous dispatch loop fails n=3 same-day with the same observable failure mode on a single contained scope (here: every dispatch on the control-monitor repo produced a draft PR whose only changed file was `.claude/settings.local.json`), the operator-authorized recovery pattern is:

1. **Pivot the in-flight slate to manual implementation by the meta-platform claude (MPC).** Don't sit on the broken loop while the slate stalls.
2. **File the substrate bug as a tier-1 ticket and ship the structural fix.** The manual rescue is the bandage; the substrate fix is the durable repair.
3. **Re-deploy and resume autonomous dispatch.** Manual implementation is a contingent escape hatch, not the new normal.

This pattern shipped control-monitor v1 (8 tickets, #5–#12) inside two days while the autonomous-loop substrate gap was being fixed in parallel.

## Founding case (2026-06-16 → 2026-06-17)

**Symptom (n=3):** Three consecutive claude-pilot dispatches on cm tickets (cm#5 / cm#8 / cm#10) exited with `commit-pushed-no-pr` and the rescued content was a `.claude/settings.local.json` file (143 lines of operator-machine-specific Claude Code permission allowlists) — zero real implementation in the rescued commit. PRs #16, #17, #19 were closed as garbage; cm-substrate-gap was filed as **mika#1552**.

**Root cause:** `dispatch-lib.sh`'s mika#1282 dirty-worktree-rescue path excluded `.claude/commands/` (mika#1288) and `.claude/claude-pilot.json` (mika#1419) but missed `.claude/settings.local.json`. When the pilot session did very little real work, the only dirty content was that allowlist — and the rescue captured it as "pilot impl."

**Manual recovery:** Samidarko-claude's spool message ratified mika-platform-claude (MPC) as the manual implementer for the remaining cm tickets, pending substrate fix.

**Slate shipped via MPC manual implementation** (2026-06-16 → 2026-06-17):
- cm#5 — cm-adapter (MailboxAdapter + MikaCliAdapter + MailboxScanner) — PR #18 ✅
- cm#6 — SSE + event_log (broadcast bus + `/api/v1/events`) — PR #22 ✅
- cm#7 — Rust CLI binary (six subcommands wrapping cm-api) — PR #20 ✅
- cm#8 — frontend dashboard (entity grid + threads + compose + SSE live) — PR #23 ✅
- cm#9 — docker-compose + Makefile + .env.example — PR #21 ✅
- cm#10 — sessions + Hyprland (ps/hyprctl/kill/wtype wrappers) — PR #24 ✅
- cm#11 — services (OpenRC rc-service family wrapper) — PR #25 ✅
- cm#12 — project lifecycle (workspace discovery + git wrappers) — PR #26 ✅
- cm frontend control panels (Sessions/Services/Projects tabs surfacing #10/#11/#12) — PR #27 ✅

**Substrate fix (parallel track):** mika#1552 PR #1553 extended the rescue exclusion pathspec at all four sites in `dispatch-lib.sh` to include `:!.claude/settings.local.json` and the general wildcard `:!.claude/*.local.*` (covers future `hooks.local.json`-class files). 251/251 dispatch-lib tests pass; `make deploy` confirmed the fix at `~/.mika/agents/mika-dev/skills/_shared/dispatch-lib.sh` at all four sites.

## The discipline

### When MPC manual is the right move

All of these conditions need to hold:

1. **n=3 same-class failure** — three independent dispatches on the same scope reproduce the same observable leak/break. Below n=3, the failure could be coincidence; at n=3 it's a structural signal.
2. **Contained blast radius** — the failure is scoped (here: cm-only; other repos' loops would still work modulo the same latent gap firing). MPC manual is not for substrate-wide outages; those are loop-breakers that take priority.
3. **Operator authorization** — explicit ratification from samidarko or Vincent before MPC takes the keyboard. Per [[feedback-no-direct-impl-use-mika-spawn]], cm/cpp/wizzard/cloud/skills are MPC-owned scopes; mika repo is dispatch-only. cm being repo-moved into senara-solutions earlier the same week (2026-06-15) is what made this scope MPC-eligible — see [[doctrine-defer-via-move-on-n-equals-1-2026-06-16]].
4. **Substrate ticket filed and prioritized tier-1** — the manual rescue must be paired with a structural fix ticket, not a memory note. Without that pairing, the autonomous loop never recovers and "manual mode" becomes the new normal (which violates the prime directive that ships through `/mika` quality pipeline).

### How to run MPC manual implementation

The exact same `/mika` quality steps run, just executed inline by MPC instead of in a claude-pilot subprocess:

- One worktree per ticket under `.claude/worktrees/`
- Read existing patterns first; mirror surrounding code shape
- Unit tests for soft logic, not exhaustive integration tests (those need live infra)
- `cargo check --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` + targeted `cargo test` per crate
- Commit with `Pipeline-Exempt: code-only` (or `docs-only` as appropriate) trailer when the change carries no plan-docs callout
- `gh pr create` + monitor checks + merge

**What's structurally different from autonomous dispatch:**
- No `dev-groom` architect canvass — MPC reads the issue body for spec and groom-call coverage. This is acceptable for small/medium tickets with clear specs and operator-defined scope (cm tickets all had detailed YAGNI-disciplined specs); it is NOT acceptable for broad-scope refactors that need architect view.
- No QA review subprocess — MPC's own clippy/check/test runs are the QA. Live integration tests are deferred to "next time the operator runs the feature."

### How to know when to stop manual and resume loop

- Substrate fix is shipped to `main`
- Substrate fix is deployed (`make deploy` confirmed; the deployed copy at `~/.mika/agents/*/skills/_shared/dispatch-lib.sh` carries the fix)
- At least one autonomous dispatch on the previously-failing scope succeeds (or, when not testable due to webhook flow being down, the substrate fix's unit tests pass and operator authorizes resuming dispatch)

The cm shipping arc ended in a hybrid state: substrate fix shipped + deployed, autonomous loop not yet retested live (webhook flow gated on operator ngrok session). Next mika-dev dispatch will be the live validation.

## Counter-discipline (what NOT to do)

- **Don't normalize MPC manual** — every additional manual session burns operator confidence in the loop. Track the count; if MPC-manual shipping exceeds autonomous shipping for >1 day on a healthy repo, the loop has a deeper problem than a single leak pattern.
- **Don't skip the substrate ticket** — if the failure pattern isn't filed as a tier-1 substrate fix the same session it's detected, the silent gap normalizes and the next leak class (n=2 on a new pattern, then n=3, then permanent manual mode) is institutional inevitability.
- **Don't over-broaden scope** — MPC manual is for the specific in-flight slate that's blocked. Don't pull other unrelated tickets into manual mode because "the loop's broken anyway." Other tickets continue to dispatch via the loop; only the contained-scope slate diverts to manual.

## Related

- [[feedback-n-equals-2-is-the-signal]] — the n=2 rule for filing; MPC manual extends it to n=3 for diverting the implementation surface.
- [[feedback-substrate-ownership-change-the-tyre]] — substrate bugs in cpp/wizzard/cloud/skills/mika-platform are MPC-owned, not file-and-forget. cm is in the same MPC scope after the 2026-06-15 repo move.
- [[feedback-no-direct-impl-use-mika-spawn]] — the routing rule that makes cm MPC-eligible (cm is in MPC's "I own this" scope).
- [[feedback-manual-rescue-is-contract-evidence-not-throughput]] — manual rescues are disconfirmation of the loop contract, not throughput. Tracks the n=3 evidence; don't celebrate manual shipping volume as productivity.
- `docs/solutions/best-practices/manual-fallback-with-gap-flagging-2026-05-05.md` — adjacent doctrine: convert one-shot fallbacks into durable work by filing gap tickets.
- mika#1552 — the substrate fix that closed the cm leak class (PR #1553, 2026-06-17).
- mika#1282 / #1288 / #1419 — the lineage of `dispatch-lib` rescue-exclusion fixes that #1552 extends.
