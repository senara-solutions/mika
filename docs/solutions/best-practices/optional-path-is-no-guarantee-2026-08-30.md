---
module: skills/shell-exec, egress-substrate, tool-execution
tags: [doctrine, egress, allowlist, single-controlled-egress, defense-in-depth, fail-closed, measured-in-production]
problem_type: security-invariant
category: best-practices
issue: mika#1991
date: 2026-08-30
---

# An architectural guarantee carried by an OPTIONAL path is not a guarantee

## Context

The `fetch_url` builtin (mika#1969) reaches the network through the gateway's
controlled-egress substrate, whose compile-time allowlist (`service-public.fr`,
`ants.gouv.fr`, `impots.gouv.fr`, `data.gouv.fr`) is the argument that made it
acceptable under the single-controlled-egress invariant. That argument holds
only if `fetch_url` is the sole way an agent reaches the network.

It was not. The T1 mission-passeport measurement (2026-08-24, real conditions,
bit-identical template to T0) showed, over one shot:

- `fetch_url` calls: **0** (skill exposed, `always_on`, verified loaded before the shot)
- `run_shell` + `curl` calls: **78**
- hosts reached: `service-public.fr` (62), `rendezvouspasseport.ants.gouv.fr` (10),
  **`www.bouscat.fr` (8 — OFF allowlist, reached with no obstacle)**,
  `legifrance` (2), `timbres.impots` (2)

No deliberate evasion: `curl` is simply the most natural path for the model.
The allowlist constrained **none** of this traffic — it went around, through
`run_shell`.

## The principle

**An architectural guarantee carried by an OPTIONAL path is not a guarantee.**

While an unconstrained path exists, an allowlist describes only the traffic of
the callers who choose the enforced door. It is not a property of the system;
it is a property of the well-behaved caller. Security invariants may not depend
on which door a caller picks.

Corollary of the older non-transit doctrine (`2026-08-22-structural-four-layer-doctrine-bake-non-transit-mika1798.md`):
**construct the incapacity, don't promise the restraint.** The fix must not rest
on "the model will learn to prefer `fetch_url`" — a single shot cannot even
distinguish learning from variance. The constrained path must be made
*mandatory*, and the unconstrained path *unavailable*.

## Pattern

When a guarantee is enforced at one substrate (here: the gateway allowlist),
close every parallel path that reaches the same capability without transiting
it. Prefer removing the capability from the parallel path over persuading the
caller not to use it:

1. **Refuse the direct capability on the parallel path.** `run_shell` now
   refuses `curl` / `wget` (boundary-aware lexical scan, same class as the
   mika#1957 L3 gated-CLI block). The refusal names the sanctioned path
   (`fetch_url` / `web_search`). This is the code-level increment shipped in
   mika#1991.

2. **Do NOT duplicate the enforced policy onto the parallel path.** Re-copying
   the four gouv.fr hosts into the shell handler would create a second source
   of truth — the very scattered-guarantee this principle warns against — and
   would trip `scripts/verify-egress-uniqueness.sh`, whose job is to keep the
   allowlist the sole property of `crates/mika-gateway/src/egress_fetch/`. The
   handler stays allowlist-ignorant: it refuses *all* direct fetchers, so an
   off-allowlist host is refused (bypass closed) and an allowlisted host is
   still reachable — through `fetch_url`, which enforces the allowlist.

3. **Name the residual and the complete closure.** A lexical scan is
   defense-in-depth, not a sole gate: token-splitting, glob/variable assembly,
   `base64 | sh`, and non-`curl` transports (`nc`, `/dev/tcp`, a python
   `urllib` one-liner) still slip a byte-level match. The complete incapacity
   is at the network layer — a fresh network namespace with no route plus a
   forced redirect to the substrate gateway (mika#1969 AC5 iptables policy, and
   the bwrap `--unshare-net` + unix-socket proxy the dev-pilot already runs in
   `scripts/mika-pilot-egress-proxy`). This ticket promotes that layer from
   "deferred" to "necessary": it is exactly the layer that was missing.

## Falsifiable check

Re-shoot the T1 template: **0 successful direct-network `curl` from `run_shell`**.
Regression scenario "fetch this URL" resolves through the substrate or fails
explicitly — never by silently reaching an off-allowlist host.

Anti-vacuity: the gate is scoped to direct HTTP fetchers; ordinary shell
commands still run, and the allowlisted read path (`fetch_url`) is unweakened.
See `crates/mika-agent/tests/shell_exec_egress_containment.rs`.
