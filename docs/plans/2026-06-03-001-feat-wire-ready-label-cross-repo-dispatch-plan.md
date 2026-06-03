---
title: "feat(dispatch): wire ready-label dispatch to non-mika repos"
type: feat
status: active
date: 2026-06-03
issue: senara-solutions/mika#1382
---

# Wire `ready`-label autonomous dispatch to non-mika repos

## Problem

A `ready` label on a `mika-cloud` / `mika-skills` / `claude-pilot-py` issue never triggers an
autonomous `dev-pilot` dispatch. Confirmed live: mika-cloud#105 was `ready`-labelled (2026-06-02
19:21), the label was consumed, but the engine logged **zero** dispatch activity for it; no
`run_claude_pilot`, no PR, no `ready_label_dispatch_stalled` event. Every one of ~15
`ready_label_dispatch_stalled` events over 3 weeks has `location: senara-solutions/mika#...`, and
**no `mika-platform-dev[bot]` merge exists in any non-mika repo's history**. The autonomous loop
serves exactly 1 of 6 repos.

## Root cause — it is NOT the agent-side marker (premise correction)

The original ticket body named `is_ready_label_dispatch_marker` / `parse_ready_label_location` as
loci. **Those are already repo-agnostic and are not the defect** — verified by reading the code:

- `crates/mika-agent/src/webhook_dispatch.rs:50` — `is_ready_label_dispatch_marker(msg) =
  msg.starts_with(READY_LABEL_DISPATCH_MARKER)`; the marker is `[GitHub] Issue labeled ready on
  senara-solutions/<any-repo>#<n>`. Fires for any repo.
- `crates/mika-gateway/src/github.rs:271` — `route_event("issues", Some("labeled"), _) =>
  Some("mika-dev")`. Repo-agnostic.

The break is **upstream of the agent**, in two gateway-layer concerns:

### Layer 1 — webhook delivery (does the event reach the gateway at all?)

The gateway only acts on events GitHub actually delivers to `POST /webhook/github`. mika's repo-level
hooks show only an AWS Amplify hook; the Mika gateway hook is presumably **org-level**
(`senara-solutions`), which would cover all repos — but this could not be verified from the
orchestrator token (`admin:org_hook` scope missing). **Open question O1.**

### Layer 2 — repo → container routing (`resolve_github_container_url`, `github.rs:834-897`)

```
SELECT customer_id, agent_mapping FROM github_repos WHERE repo_full_name = $1
```

- Repo **found** → route to that customer's container.
- Repo **not found** (`Ok(None)`):
  - **multi-tenant** (no `MIKA_AGENT_BASE_URL`) → **event dropped** (`warn!("GitHub repo not
    registered and no fallback configured")`).
  - **single-tenant** (`MIKA_AGENT_BASE_URL` set) → falls back to that base URL → reaches the one
    mika-server, where `route_event` picks `mika-dev`.

`github_repos` is populated by **customer provisioning** (gateway migration 004, `add-customer`).
Internal repos (mika-cloud/skills/cpp) are never provisioned as customers → no row.

### Tenancy split (the crux — Open question O2)

The autonomous loop's mika-dev runs on the **dev host** (OpenRC, one `mika-server` with all
well-known agents). If that gateway has `MIKA_AGENT_BASE_URL` set (single-tenant), Layer-2 routing
already works via fallback — meaning the *local* blocker is **Layer 1 (delivery)**, not the registry.
In **multi-tenant prod** (K8s, no base-url fallback), the *same* missing-registry-row path **drops**
the event. So the fix likely needs **both** layers, and the dominant blocker differs by environment.
The plan must not assume one without verifying.

## Open questions (resolve in Phase 0 of implementation, before coding)

- **O1 — delivery.** Does the `senara-solutions` org (or per-repo) GitHub webhook actually deliver
  `issues.labeled` events for mika-cloud/skills/cpp to the gateway? Verify via `gh api
  orgs/senara-solutions/hooks` (needs `admin:org_hook`) or the gateway access log
  (`grep webhook/github gateway.log` for non-mika `repo_full_name`). If events never arrive, no
  amount of routing code helps — the webhook config is the fix.
- **O2 — tenancy.** Is the loop's gateway single-tenant (`MIKA_AGENT_BASE_URL` set) or multi-tenant?
  Inspect the running gateway's config. Determines whether Layer-2 work is needed for the *local*
  loop or only for prod.
- **O3 — drop-guard safety (architect Q4, MANDATORY before U1 ships).** Trace
  `is_unauthorized_webhook_dispatch` (`crates/mika-agent/src/webhook_dispatch.rs`) and any downstream
  code that processes a routed webhook event and consults `github_repos`. Confirm: does any guard
  assume a routed event implies a `github_repos` row exists? The allowlist widens "legitimate" from
  "has a row" to "has a row OR is an internal repo" — the allowlist path produces a routed event whose
  row lookup returns `None`. U1's tests must prove the allowlist-routed event has the same shape/
  metadata as the registry-routed one (or that any distinction is safe). If a downstream guard keys
  on row-existence, that is a blocker surfaced here, not discovered in review.
- **O4 — `github_repos` scope (architect Q1 contingency).** Confirm `github_repos` is strictly
  customer-scoped (no internal-repo rows expected). Approach B is correct only if internal repos are
  semantically *not* customers; if the table already carries non-customer rows, reconsider.

## Approaches (Layer 2 fix — decide during grooming/architect review)

**A. Register internal repos in `github_repos`.** Insert rows mapping `senara-solutions/mika-cloud`
etc. → the customer_id/container that hosts mika-dev. Zero code; per-repo manual SQL (or a small
provisioning script). Fragile: every new internal repo needs a row; couples internal-repo routing to
the customer table it doesn't semantically belong in.

**B. Gateway internal-repo allowlist (preferred).** Add a small, explicit allowlist of org-internal
repos in the gateway that resolves to the well-known mika-dev route directly, bypassing the
customer-registry lookup. Durable (new internal repos add one list entry), semantically honest
(internal repos are not customers), but is a mika-core change with tests. Sits as a branch in
`resolve_github_container_url` before/after the `github_repos` query.

**Lean: B.** A is a stopgap that re-introduces manual per-repo wiring (the exact class of fragility
the loop is meant to remove). Architect first-pass (session `a138e223`) concurred: **B**, contingent
on O4 confirming `github_repos` is customer-scoped.

**Sequencing note (architect Q3 — operator decision).** Because the dominant blocker differs by
environment (O2), the architect recommends *optionally splitting* this into two tickets: **(A)
delivery-config** — O1 verification + webhook config, unblocks the local loop immediately, zero code;
**(B) routing-code** — the allowlist (U1), unblocks multi-tenant prod. A is a prerequisite for
end-to-end verification of B (can't test cross-repo dispatch if events never arrive). The O1/O2 gates
in this plan achieve the same sequencing within a single ticket, so the split is a cleanliness-vs-
coordination-overhead call for Vincent — not a blocker. Default: keep as one ticket, gated.

## Implementation units (assumes O1 = events ARE delivered; if not, the fix is webhook-config only)

### U1. Gateway internal-repo routing allowlist

- **Goal:** A non-customer org-internal repo resolves to the mika-dev-hosting route without a
  `github_repos` row, in both tenancy modes.
- **Files:** `crates/mika-gateway/src/github.rs` (the `resolve_github_container_url` function +
  an `INTERNAL_REPOS` const or config); `crates/mika-gateway/CLAUDE.md` (document the allowlist).
- **Approach:** Define the internal-repo set (`senara-solutions/mika`, `mika-cloud`, `mika-skills`,
  `claude-pilot-py`, and any others). When `repo_full_name` is in the set and not found in
  `github_repos`, resolve to the same route the single-tenant fallback uses (the mika-server hosting
  the well-known agents) rather than dropping. Keep `route_event` unchanged — it already yields
  mika-dev. **Allowlist is a compiled const** (architect Q2: security-adjacent, changes on release
  cadence not ops cadence; YAGNI on env-tunability — adding a repo is a code change + restart at a
  quiescent boundary, the normal deploy path). **Gated on O3** — do not ship until the drop-guard
  trace confirms the allowlist-routed event cannot violate `is_unauthorized_webhook_dispatch` or any
  downstream row-existence assumption.
- **Test scenarios:**
  - `resolve_github_container_url` for `senara-solutions/mika-cloud` with no `github_repos` row and
    no `agent_base_url` → resolves to the internal route (not `None`/drop).
  - Same repo **with** a `github_repos` row → existing customer route still wins (allowlist does not
    override an explicit registration).
  - A genuinely-unknown external repo with no row and no base-url → still drops (allowlist is a
    closed set; no broadening of the drop guard).
  - Single-tenant mode (`agent_base_url` set) → behavior unchanged for internal repos (fallback
    already covered them; allowlist must not double-route or change the target).
- **Verification:** `cargo test -p mika-gateway`; a `ready` label on a mika-cloud canary issue
  produces a `run_claude_pilot` dispatch end-to-end (gated on O1).

### U2. (conditional on O1) Webhook delivery for internal repos

- **Goal:** GitHub delivers `issues` events for the internal repos to the gateway.
- **Files:** none in-repo — this is GitHub webhook configuration (org-level hook events, or per-repo
  hooks). Document the required config in `crates/mika-gateway/CLAUDE.md`.
- **Approach:** Confirm/extend the org webhook to include `issues` events for the internal repos, or
  add per-repo hooks. Operator action with `admin:org_hook`. **Cross-repo:** if a provisioning script
  belongs in mika-cloud (where `add-customer`/infra scripts live), file a secondary ticket there.
- **Test scenarios:** `Test expectation: none` — config verification, not code. Confirm via the
  gateway access log showing an inbound `issues.labeled` for a non-mika repo.

## Acceptance criteria

- [ ] O1 and O2 are answered and recorded before any code lands (delivery confirmed; tenancy known).
- [ ] A `ready` label on a mika-cloud (and mika-skills, claude-pilot-py) issue produces an
      autonomous `dev-pilot` dispatch that reaches `run_claude_pilot` — verified end-to-end on a
      canary issue.
- [ ] An explicit `github_repos` registration for a repo still takes precedence over the allowlist
      (no regression to customer routing).
- [ ] Unknown external repos still drop in multi-tenant mode (no broadening of the routing surface).
- [ ] `cargo test -p mika-gateway` green; gateway CLAUDE.md documents the internal-repo allowlist.

## Scope boundaries

- **Out of scope:** the chronic `ready_label_dispatch_stalled` on mika# (separate intermittent —
  mika#1383); the mika-dev stale-id fabrication (mika#1384); the notification severity work (mika#1381).
  This ticket is purely the cross-repo *delivery + routing* gap.
- **Out of scope:** changing `route_event` or any agent-side marker/guard — already repo-agnostic.
- **Deferred to follow-up:** if O2 shows the loop gateway is multi-tenant in a way that needs
  per-environment config, and if a mika-cloud provisioning-script change is required for U2, that
  ships as a secondary `mika-cloud` ticket (companion-PR cross-reference).

## Risk / rollback

Low blast radius: U1 adds a branch that only fires for a closed allowlist of org-internal repos and
only when no `github_repos` row exists; it cannot reroute customer traffic (explicit rows win) and
cannot broaden the external-repo drop. Rollback = revert the gateway diff. The dominant *risk* is
mis-diagnosis: if O1 reveals events never reach the gateway, U1 is inert and U2 (webhook config) is
the real fix — which is why O1/O2 gate the code.

## Evidence

- `crates/mika-agent/src/webhook_dispatch.rs:50`; `crates/mika-gateway/src/github.rs:271`
  (route_event), `:834-897` (resolve_github_container_url, the drop/fallback branches)
- gateway migration 004 (`github_repos`); `crates/mika-gateway/CLAUDE.md` § GitHub Webhook Integration
- 2026-06-02/03 ops: mika-cloud#105 `ready` consumed, zero engine dispatch; all
  `ready_label_dispatch_stalled` locations `mika#`; zero bot merges on non-mika repos. See mika#1382
  comment (2026-06-03) for the full investigation trail.
