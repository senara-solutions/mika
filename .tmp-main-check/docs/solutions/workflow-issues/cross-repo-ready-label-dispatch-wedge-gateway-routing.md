---
title: "Wedged autonomous loop on non-mika repos: ready-label dispatch drops at the gateway, not the agent"
module: mika-gateway
component: webhook-routing
problem_type: workflow_issue
track: knowledge
tags: [autonomous-loop, dispatch, ready-label, gateway, github_repos, webhook, multi-tenant, debugging]
date: 2026-06-03
issue: 1382
---

# Ready-label dispatch is wedged for non-mika repos — debug the gateway, not the agent

## Context

A `ready` label on a **non-mika** repo issue (`mika-cloud`, `mika-skills`, `claude-pilot-py`) is
consumed but produces **no autonomous dispatch** — no `run_claude_pilot`, no PR, and (the tell) **no
`ready_label_dispatch_stalled` event either**. mika# issues, by contrast, *do* reach the agent and
*do* stall-and-alarm. So the autonomous loop silently serves 1 of 6 repos, and the failure is
invisible because the stall guard never fires.

Surfaced when mika-cloud#105 was groomed + `ready`-labelled (2026-06-02) and never built.

## Guidance — the diagnostic map

When the loop is wedged with "**`ready` consumed but nothing dispatched**", check layers in this
order. The instinct to look at the agent-side marker/guard is **wrong** — those are repo-agnostic.

1. **Did the agent even get the event?** Grep the server log for the agent turn:
   `grep 'run_gh.*remove-label.*ready' $MIKA_SPIRIT_LOG_FILE` for the issue number. If the *agent*
   stripped the label, it received the marker; if not, the strip came from an ack path and the agent
   never saw it — a **routing-level miss**, not a dispatch-level miss.
2. **Presence vs absence of a stall event is the discriminator.** `grep ready_label_dispatch_stalled
   $MIKA_SPIRIT_LOG_FILE | jq .location` — every location being `senara-solutions/mika#...` (and
   never your repo) means non-mika events never reach the agent turn where the guard lives. Absence
   of the alarm is itself the signal.
3. **The drop is in the gateway, at repo→container resolution.** `resolve_github_container_url`
   (`crates/mika-gateway/src/github.rs:834-897`) does
   `SELECT customer_id, agent_mapping FROM github_repos WHERE repo_full_name = $1`. That table is
   populated by **customer provisioning** (gateway migration 004 / `add-customer`). Internal repos
   are not customers → no row → on `Ok(None)` the event is **dropped** in multi-tenant
   (no `MIKA_AGENT_BASE_URL`) or **falls back** to the base URL in single-tenant. So the *same*
   missing row behaves differently per environment — multi-tenant prod drops, single-tenant dev may
   route via fallback.

### What is NOT the cause (verified repo-agnostic)

- `is_ready_label_dispatch_marker` (`crates/mika-agent/src/webhook_dispatch.rs:50`) —
  `msg.starts_with(READY_LABEL_DISPATCH_MARKER)`; the marker is
  `[GitHub] Issue labeled ready on senara-solutions/<any-repo>#<n>`.
- `route_event` (`crates/mika-gateway/src/github.rs:271`) — `("issues","labeled") => Some("mika-dev")`
  for any repo.

Both fire for every repo. Time spent reading the agent marker/guard is wasted; start at the gateway.

## Why this matters

- **Silent + alarmless.** Unlike the mika# stall (which logs `ready_label_dispatch_stalled` and fires
  an operator notification), the cross-repo miss produces *no* signal. The loop can be dead for 5 of 6
  repos for weeks unnoticed — especially if the operator has muted the noisy notification channel.
- **Misleading agent self-report.** When asked why a dispatch stalled, mika-dev fabricated progress by
  quoting a **stale** `task`/`session` id (from a `run_claude_pilot` task hours earlier) as a fresh
  re-dispatch. Always verify a claimed task/session id against the server-log timestamp before
  believing a "re-dispatched, awaiting callback" claim (`feedback_mika_dev_llm_fabricates`).

## When to apply

Any "the loop didn't pick up my ticket" report. First branch on repo: if it's a non-mika repo, this
is almost certainly the gateway registry gap — go straight to step 3. If it's mika#, look for a
`ready_label_dispatch_stalled` event (that path is the chronic stall, mika#1383, a different defect).

## Related

- `mika#1382` — fix: wire ready-label dispatch to non-mika repos (gateway internal-repo allowlist).
  This doc ships on that branch.
- `mika#1383` — chronic `ready_label_dispatch_stalled` on mika# (the *other* failure mode).
- `mika#1384` — mika-dev stale-id fabrication on the stall path.
- `crates/mika-gateway/CLAUDE.md` § GitHub Webhook Integration (multi-tenant routing via `github_repos`).
- Companion build-error learning shipped the same session: `@senara-solutions/ui` 401 — GitHub
  Packages npm requires auth even for public packages (`mika-cloud/docs/solutions/build-errors/`, PR #107).
