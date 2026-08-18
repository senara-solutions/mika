# Egress-Search No-Log Audit — Three-Layer Runbook

Module: [`crates/mika-gateway/src/egress_search/`](../src/egress_search/)
Owner: mika-gateway
Ticket: [mika#1810](https://github.com/senara-solutions/mika/issues/1810) — E4 of milestone [#1806](https://github.com/senara-solutions/mika/issues/1806)
Sibling docs: [`egress-search.md`](./egress-search.md) — substrate architecture (E1/E2)

---

## Purpose

The Prime 2026-07-19 invariant is **« zéro liage + zéro rétention »**. E1 built
the substrate. E2 wired the Brave client. E3 lints identity/request de-liage.
**E4 closes the retention half — verified end-to-end.**

The doctrinal claim is *no-log par construction, vérifié*. Construction alone
is not enough: a self-host that logs by default at the network layer, or
persists silently to disk, is **worse** than Brave, because it re-ties the
tenant to the query on our side.

This doc names the three layers we audit, the checks that cover each, and —
critically — what we deliberately **do not** audit and why.

---

## The three layers

| # | Layer                | Cover                                                         | How enforced                                                                                    |
| - | -------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| 1 | Application logs     | Substrate emits no tracing / println / dbg lines beyond Q4 audit | Build-time lint + Q4 CapturingLayer runtime test in `egress_search::tests`                       |
| 2 | Network metadata     | iptables/nft, proxy access logs, VPC flow logs                | **Substrate SPEC (this doc)** — implementation is mika-cloud follow-up, not this ticket           |
| 3 | Persistence          | No file writes, no SQLite writes, no cache tables             | Build-time lint (source scan) + runtime audit script (SQLite schema probe)                       |

The two automated surfaces:

- **Build-time lint** — `scripts/verify-egress-no-log.sh`, wired to CI as the
  `egress-no-log-lint` job. Scans `crates/mika-gateway/src/egress_search/*.rs`
  for logging macros + persistence calls. Any hit outside the two-event Q4
  allowlist fails the build. Runs on every PR.
- **Runtime audit** — `scripts/audit-egress-no-log.sh`, run manually in dev
  or prod. Reads live env config, greps the mika-gateway log file for the
  Q4 event shape, probes the SQLite DB for substrate-shaped tables. Exits
  1 on leak, 2 on Layer 2 manual-audit-owed (default), 0 when Layer 2 has
  been confirmed clean out-of-band (`MIKA_AUDIT_SUPPRESS_L2_WARN=1`).

Together they cover Layer 1 and Layer 3. Layer 2 is deliberately outside
their reach — see § Layer 2 below.

---

## Layer 1 — Application logs

### What we catch

The build-time lint (`scripts/verify-egress-no-log.sh`) rejects, in the
production code of `egress_search/*.rs`:

- Any `debug!` / `warn!` / `error!` / `trace!` macro (short or `tracing::`-qualified).
- Any `println!` / `eprintln!` / `print!` / `eprint!` / `dbg!`.
- Any `log::*!` macro.
- Any `info!` call whose call block does NOT declare
  `event = "search_requested"` or `event = "search_egress"` — the two audit
  lines the Q4 CapturingLayer test enforces the field-shape of.

The runtime audit (`scripts/audit-egress-no-log.sh`) then confirms live
behavior:

- Only `search_requested` / `search_egress` `event` names appear on
  `search_*` lines in the gateway log file.
- Neither event carries any of the forbidden field names (`query`,
  `tenant_id`, `tenant_hash`, `user_id`, `chat_id`, `customer_id`,
  `api_key`, `retry_after`, `url`) — using `jq` when available, falling
  back to grep otherwise.

The Q4 CapturingLayer test
(`egress_search::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`)
runs on every `cargo test` and asserts by construction that the substrate's
two events carry only the allowlisted fields — and further that the query
bytes never appear in ANY captured event, including third-party (reqwest,
hyper) emissions.

### What we do not catch

- **Third-party crates outside our target prefix.** The Q4 test asserts a
  cross-source byte assertion (the sensitive query bytes must appear in no
  field of any captured event, regardless of source), which catches URL
  logging by reqwest/hyper today. If a future release of one of those
  crates ships a new emission shape the test does not visit, the assertion
  covers it because it operates on ALL captured events. But the shape of
  the CapturingLayer is exhaustive only for the emissions it visits — if a
  third-party dependency emits directly to stderr via `eprintln!` bypassing
  tracing, the test will not see it. This is why the adversarial runtime
  test (§ Adversarial) captures the full stdout+stderr stream.
- **`RUST_LOG=trace` in production.** Elevated log level does not violate
  the substrate discipline (the substrate emits zero debug/trace lines),
  but it can uncover chattier third-party output. The runtime audit's
  Layer 1a notes this and asks the operator to re-run Layer 1b with
  production traffic on that setting.
- **Log lines emitted before the substrate is invoked.** The gateway's
  request-log middleware logs `POST /internal/search` with path + status
  + latency — that is expected and correct (the URL is a fixed path, not
  the query). Any middleware that started logging the request body would
  be a separate discipline break, caught by review of `routes.rs`, not
  this substrate lint.

---

## Layer 2 — Network metadata (SPEC ONLY — mika-cloud follow-up)

**This layer's implementation is out of scope for this ticket.** E4 delivers
the specification; the actual K8s / iptables / proxy config lives in a
mika-cloud follow-up.

The substrate MUST run in an environment where the network path to the
search upstream carries no metadata log. That is a substrate-owner
property, not a code property, and must be verified by the operator at
deploy time.

### Required properties

**iptables / nftables:**

- The egress chain that permits `api.search.brave.com` (or the equivalent
  IP set) MUST NOT carry a `--log-prefix` target on that hop.
- No `NFLOG` / `ULOG` / `LOG` target on the egress-search flow.
- If a broader logging rule exists on the OUTPUT / FORWARD chain, it MUST
  either exclude the search-upstream destination or the operator MUST
  document the residual risk in the deploy runbook (this constitutes a
  known-and-accepted deviation, not a passed audit).

**HAProxy / Envoy / nginx (only relevant if a proxy sits in the path):**

- **HAProxy** — `option httplog` OFF on the search-upstream backend, or
  `no log` on the frontend for that route.
- **Envoy** — `access_log: []` on the listener/cluster carrying search
  upstream traffic.
- **nginx** — `access_log off;` on the location serving the search-upstream
  proxy path.

**Cloud / K8s VPC flow logs:**

- VPC flow logs enabled at the broad scope MUST either exclude the
  search-upstream destination or the operator MUST accept and document
  that flow-record accumulation is happening upstream.

### What this ticket does NOT deliver

- No iptables rules committed to `mika-cloud`.
- No K8s NetworkPolicy or Envoy config committed.
- No enforcement that a given deploy actually holds these properties.

That work belongs in a mika-cloud follow-up ticket. The runtime audit
(`scripts/audit-egress-no-log.sh`) explicitly emits a Layer 2 warning
(exit code 2 by default) on every run to force the operator to attest.
The `MIKA_AUDIT_SUPPRESS_L2_WARN=1` downgrade is the operator's signed
statement that the substrate side has been confirmed out-of-band for
this deploy.

### Failure mode → SearXNG escalation trigger

If a deploy environment cannot honor these properties — e.g., an
observability platform mandates per-request tracing at the network
layer, or a corporate proxy enforces access logging with no exception
— that is the SearXNG escalation signal (milestone [#1806](https://github.com/senara-solutions/mika/issues/1806)
E6, currently design-only). The ticket text is explicit:

> Si l'audit E4 révèle qu'on ne peut PAS vérifier end-to-end (ex : proxy
> logging obligatoire, ou observability impose per-query trace), c'est
> LE signal pour escalader à SearXNG self-hosted.

Escalation to E6 is an operator decision (Prime bearing), not an
automated fallback.

---

## Layer 3 — Persistence

### What we catch

The build-time lint rejects, anywhere in `egress_search/*.rs` (production
OR tests):

- `File::create`, `std::fs::write`, `fs::write(`, `OpenOptions`, `write_all`.
- `sqlx::query`, `insert_into`, `INSERT INTO`, `rusqlite`.

The runtime audit probes the SQLite DB (path configurable via
`MIKA_GATEWAY_DB`; defaults to `~/.mika/data/mika.db` for an agent host):

- **Substrate-shaped tables** — a table named `search_egress*`,
  `brave*`, `search_upstream*`, or `search_cache*` only exists if
  someone added substrate-side persistence. Hard fail if any are
  present.
- **Informational sweep** — any table with `search` in the name (KG
  lexical `fts_search`, `vec_search`, `search_content`) is called out
  as NOTE, not LEAK. These are legitimate KG surfaces; the operator
  should spot-check they hold no substrate-originated egress content.
- **`query`-columned tables** — any table with a `query` column is
  listed as informational, so the operator can confirm the content is
  agent-side (tool_calls, kg_*, memory) and not substrate-side.

### What we do not catch

- **Kernel-side networking buffers.** TCP send buffers, socket queues,
  the network card's DMA regions — all hold in-flight bytes for a
  bounded window before they leave the host. This is uncontrollable
  and transient; there is no auditable persistence path. It is a
  known residual and the doctrine accepts it (untrackable ≠ auditable
  failure).
- **File-descriptor state and process memory.** The `SearchRequest`
  struct is on the stack while the request is in flight and dropped
  when the function returns; the `secrecy::SecretString` fields
  zeroize on drop. This is behavior we assert by review, not by
  runtime probe.
- **Third-party dependency writes.** reqwest / hyper / rustls do not
  persist request bodies to disk today; a future release that added a
  disk cache would be caught only when we upgrade + re-run the lint.
  Dependency review at upgrade time is the mitigation, not this
  substrate lint.
- **Non-SQLite persistence surfaces.** The audit probes SQLite because
  mika-agent uses SQLite. mika-gateway itself uses Postgres, and a
  substrate that grew Postgres persistence would need a separate probe
  variant. Today the substrate uses neither (source lint enforces zero
  writes), so the runtime probe is a defense-in-depth check, not the
  primary control.

---

## Ce qu'on n'audite PAS et pourquoi

The three residuals above summarized in one place, because being explicit
about the boundary is the discipline:

- **Brave-side logging.** Brave sees the query itself — that is inherent
  to the API call. The doctrine's frame is *we don't leak from OUR side*.
  A user's decision to accept Brave's terms is Brave's contract with
  them; the substrate does not attempt to hide the query from Brave.
- **CDN / ISP hops.** Cloud-provider infrastructure, upstream ISPs,
  Brave's own edge — all outside our control. The doctrine delivers
  *strip total from OUR side*. Attempting to control third-party network
  metadata is out of scope and would misrepresent what the substrate
  guarantees.
- **Kernel-side networking buffers.** Uncontrolled, transient, no
  auditable persistence — see Layer 3 above. Auditing them would demand
  a level of introspection (per-packet inspection, kernel-mode agents)
  that itself would violate the no-log invariant.

The value of naming these residuals is honesty. A substrate that claimed
to strip *everything* including the parts it doesn't control would be
false advertising; the substrate that names what it delivers and what
it doesn't is trustworthy.

---

## Adversarial runtime test

Q4 discipline is verified at two levels:

- **Structural (build-time)** — the CapturingLayer test in
  `egress_search::tests` asserts the field shape of the two audit events
  via the tracing subscriber. It runs on every `cargo test`.
- **Adversarial (runtime)** — the runtime audit script exercises the
  live log file against real production traffic, catching any
  emission that survived the build-time filter (elevated `RUST_LOG`,
  third-party crate release with new emissions, etc.).

Neither replaces the other. The structural test is the primary control;
the adversarial audit is the empirical verification. The two together are
the "vérifié" half of "no-log par construction, vérifié".

---

## Running the audit

**Build-time (CI + local dev):**

```
bash scripts/verify-egress-no-log.sh
```

Exit 0 = clean. Exit 1 = source-level violation, see stderr for the
specific pattern + file:line.

**Runtime (dev + prod):**

```
# defaults: log at ~/.mika/logs/mika-gateway.log, DB at ~/.mika/data/mika.db
bash scripts/audit-egress-no-log.sh

# custom log file / DB path
MIKA_GATEWAY_LOG_FILE=/var/log/mika-gateway/gateway.log \
MIKA_GATEWAY_DB=/var/lib/mika/data.db \
    bash scripts/audit-egress-no-log.sh

# after Layer 2 has been confirmed by the operator this deploy
MIKA_AUDIT_SUPPRESS_L2_WARN=1 bash scripts/audit-egress-no-log.sh
```

Exit codes:
- `0` — every layer clean (Layer 2 confirmed via env var).
- `1` — leak detected (Layer 1 or Layer 3). See stderr for the specific
  finding. **Do not deploy** until root-caused.
- `2` — Layer 1 + Layer 3 clean, Layer 2 manual audit still owed (default
  posture on every run).

---

## Cross-references

- Milestone: [#1806](https://github.com/senara-solutions/mika/issues/1806) — egress-controlled search backend
- E1 (substrate keystone): [#1807](https://github.com/senara-solutions/mika/issues/1807) → merged PR [#1909](https://github.com/senara-solutions/mika/pull/1909)
- E2 (Brave client): [#1808](https://github.com/senara-solutions/mika/issues/1808) → merged PR [#1911](https://github.com/senara-solutions/mika/pull/1911)
- E3 (dé-liage identity ↔ request): [#1809](https://github.com/senara-solutions/mika/issues/1809) → PR [#1912](https://github.com/senara-solutions/mika/pull/1912)
- E4 (no-log verified end-to-end): [#1810](https://github.com/senara-solutions/mika/issues/1810) — this doc
- E5 / E6 sibling tickets: [#1811](https://github.com/senara-solutions/mika/issues/1811), [#1812](https://github.com/senara-solutions/mika/issues/1812) — SearXNG escalation is E6
- Substrate architecture doc: [`egress-search.md`](./egress-search.md)
- Q4 STRIP TOTAL invariant test:
  `crates/mika-gateway/src/egress_search/mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`
- Build-time lint: [`scripts/verify-egress-no-log.sh`](../../../scripts/verify-egress-no-log.sh)
- Runtime audit: [`scripts/audit-egress-no-log.sh`](../../../scripts/audit-egress-no-log.sh)
