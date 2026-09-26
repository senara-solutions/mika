# Egress Search — Threat Model (E3 dé-liage identité↔requête)

Module: [`crates/mika-gateway/src/egress_search/`](../src/egress_search/)
Tickets:
- [mika#1809](https://github.com/senara-solutions/mika/issues/1809) — E3 (this doc): dé-liage identité↔requête, request-shape invariant
- [mika#1806](https://github.com/senara-solutions/mika/issues/1806) — parent milestone (egress-controlled search backend)
- [mika#1807](https://github.com/senara-solutions/mika/issues/1807) — E1 substrate (log-side invariant)
- [mika#1808](https://github.com/senara-solutions/mika/issues/1808) — E2 Brave client
- [mika#1810](https://github.com/senara-solutions/mika/issues/1810) — E4 no-retention (net-layer, iptables)
- [mika#1783](https://github.com/senara-solutions/mika/issues/1783) — doctrinal precedent for query-content leakage class

Companion docs: [`egress-search.md`](egress-search.md) (architecture) — this doc is
the E3 sibling that closes the LIAGE half of the Prime invariant ("zéro liage
requête↔identité + zéro rétention"). E4 (#1810) closes RETENTION.

---

## Property under invariant

**Prime, 2026-07-19:** the upstream (Brave today, any future contingency) MUST
NOT be able to re-tie a request to a specific mika tenant / agent / user by
examining any field it sees on the wire — headers, query params, body, source
IP, or timing pattern.

E3 closes the **LIAGE** half structurally: the substrate's outgoing request is
built such that **no tenant-identifying field is present, by construction**.
E4 (#1810) closes RETENTION at the net layer (iptables + no-log on the gateway
egress path). E3 does not attempt to close E4's surface.

**Scope discipline:** this doc analyses the LIAGE surface only.
Query-content leakage (`"quel est mon email vincent@..."` — the query text
itself naming an identity) is a **content-layer** concern, out of scope for
E3, and covered doctrinally by the agent-side scope-of-being disciplines
(see mika#1783 precedent). Notes on the residual risk are in
[§ Query content leak](#query-content-leak-out-of-scope-noted-explicitly) below.

---

## What Brave sees on the wire (exhaustive enumeration)

### Method + URL

```
GET https://api.search.brave.com/res/v1/web/search?q=<query>&count=<n>
```

The base URL is stable (`DEFAULT_BRAVE_ENDPOINT` in
[`egress_search/mod.rs`](../src/egress_search/mod.rs); overridable via
`MIKA_BRAVE_ENDPOINT` only for tests / self-hosted mirrors).

### Headers (EXHAUSTIVE — asserted by
`outgoing_headers_are_shared_only`)

Brave sees exactly the following headers on every request:

| Header                  | Value shape                                    | Rationale                                                   |
|-------------------------|------------------------------------------------|-------------------------------------------------------------|
| `X-Subscription-Token`  | The shared `MIKA_BRAVE_API_KEY` value          | Auth. **Shared** across all tenants (see § API key below). |
| `Accept`                | `application/json`                             | Response content negotiation. Constant string.              |
| `Host`                  | `api.search.brave.com` (or `MIKA_BRAVE_ENDPOINT` host) | Set by `reqwest` from the URL. Constant per deploy.  |
| `User-Agent`            | `reqwest`'s default (`reqwest/<version>`)      | Not per-tenant, not per-agent, not per-user. Constant across the fleet for a given gateway build. See [§ User-Agent](#user-agent) below. |
| `Accept-Encoding`       | `gzip, br, deflate, zstd` (reqwest default)    | Transport-layer. Constant across the fleet.                 |

**NOTHING else.** The E3 lint test `outgoing_headers_are_shared_only`
constructs a request through the real substrate factory and asserts the header
set contains ZERO of: `X-User-*`, `X-Tenant-*`, `X-Agent-*`, `X-Customer-*`,
`X-Session-*`, `X-Request-Id`, `X-Trace-Id`, `Authorization` (the shared auth
travels in `X-Subscription-Token`, not `Authorization`), `Cookie`, or any
name whose lowercase form contains `user`, `tenant`, `agent`, `customer`,
`session`. Any additional per-tenant header fails CI.

#### API key (`X-Subscription-Token`)

**Structurally shared.** [`GatewaySettings.brave_api_key`](../src/settings.rs)
is loaded once from `MIKA_BRAVE_API_KEY` at gateway startup and injected into
the single [`BraveConfig`](../src/egress_search/mod.rs) held by the single
[`SearchEgressClient`](../src/egress_search/mod.rs) `Arc`-shared across every
tenant (see [`egress-search.md § Q3`](egress-search.md)). There is **no code
path** that constructs a per-tenant / per-agent / per-user `BraveConfig` — the
`SearchUpstream` enum has no runtime-mutable variant, and the config is not
mutated after construction. Rotating the key requires a gateway restart.

Consequence: Brave sees the same `X-Subscription-Token` for every request
from every mika tenant. It cannot fingerprint tenants by comparing key values.

#### User-Agent

Set to `reqwest`'s default string (`reqwest/<version>`) — the client is built
by [`build_client()`](../src/egress_search/mod.rs) which does NOT call
`.user_agent(...)`. The value depends only on the compiled `reqwest`
version, so it is constant across the entire fleet for a given gateway
build. It is not fingerprintable to a tenant.

If a future PR ever sets a custom User-Agent, the new value MUST be a fixed
string (e.g. `mika-gateway/1.0`) — never a per-tenant / per-agent string.
The E3 lint asserts that the `user-agent` header, when present, is not a
per-request-varying value; see the test source for the exact form.

### Query params (EXHAUSTIVE — asserted by
`query_params_carry_no_identifier`)

Brave sees exactly the following query params on every request:

| Param   | Value shape                          | Origin                                                |
|---------|--------------------------------------|-------------------------------------------------------|
| `q`     | The user's query string, verbatim    | `SearchRequest.query` (agent-supplied)                |
| `count` | Integer, clamped to `[1, 20]`        | `SearchRequest.max_results` (agent-supplied, clamped) |

**NOTHING else.** The lint test `query_params_carry_no_identifier`
constructs requests with a variety of `max_results` values and asserts the
outgoing URL query string contains ONLY the names `q` and `count`. No
`user`, `tenant`, `agent_id`, `session_id`, `trace_id`, `client_id`,
`customer_id`, etc.

The `q` field carries the caller-supplied query verbatim — see
[§ Query content leak](#query-content-leak-out-of-scope-noted-explicitly) for
the content-layer residual risk this exposes.

The `count` field is a small bounded integer (`[1, 20]` after
[`send_once`](../src/egress_search/brave.rs)'s clamp). It carries no
tenant-identifying bits: agents may legitimately request 1, 5, or 20 results
regardless of tenant, and Brave sees the same three or four distinct values
across all tenants.

### Body

**None.** The Brave call is a `GET` (see
[`send_once`](../src/egress_search/brave.rs)). No request body is ever
transmitted; there is no field there to leak identity.

If a future upstream requires POST, the E3 lint MUST be extended to
enumerate that upstream's body fields explicitly (same discipline as the
header/query assertions).

### Source IP

**Single egress point per E1.** The substrate is a module in the gateway
process; every tenant's search transits the same gateway pod (or the same
horizontally-scaled set of gateway pods behind a shared egress NAT / VPC
gateway). Brave sees a single source IP (or a small IP pool that is
NOT per-tenant).

This is a structural consequence of E1 (§ Q1 placement — module-in-gateway,
not per-tenant service). There is no code path in the substrate that reaches
out from a per-tenant container.

Consequence: Brave cannot infer tenant identity from source-IP alone.
Whether a single mika deployment is behind a shared NAT vs. a dedicated
egress-IP is an infrastructure choice (E4-adjacent, tracked separately) —
the substrate's guarantee is only that the mika process boundary does not
introduce a per-tenant IP.

### Timing

**Natural timing accepted in v1.** The substrate does not add jitter,
does not batch, does not delay. A request sent at wall-clock T reaches
Brave at approximately T + network RTT. Two requests fired one second
apart arrive one second apart.

Consequence: an observer able to correlate mika-tenant activity patterns
(e.g. a party watching both a tenant's telegram inbox and Brave's incoming
request log) could in principle infer some correlation between a tenant
event and a subsequent search request. This is a **known residual risk**
in v1; the mitigations are:

- Cross-tenant traffic mixing: multiple tenants share the same gateway
  egress; a single request is one drop in a stream, not a per-tenant
  singleton.
- Brave itself cannot see mika tenant activity — the correlation requires
  a third party with vantage on both sides.

**v2 improvement candidate (not in E3 scope):** if the threat surfaces
(e.g. we onboard a tenant whose adversary has this vantage), add a jitter
window (± random ms) and/or a small dispatcher pool that shuffles submit
order. Would be added as a `SearchEgressClient` layer without touching
the `brave::execute_brave_search` path. Tracked for consideration only —
no ticket filed until the surface warrants it.

---

## What Brave CAN see vs. CANNOT structurally link

### Brave CAN see

- The **query content itself** (the `q` param) — verbatim.
- **Timing** of the request (wall-clock at receipt).
- **Source IP** — single value across all mika tenants (see § Source IP).
- **User-Agent** — constant across the fleet (see § User-Agent).
- **Request count** — how many requests come from the mika egress in a
  given window (aggregate, not per-tenant).

### Brave CANNOT structurally link

- **Which mika tenant** made the request — no tenant identifier is sent
  in any header, query param, or body field. Enforced by
  `outgoing_headers_are_shared_only` +
  `query_params_carry_no_identifier`.
- **Which agent instance** — no agent identifier is sent.
- **Which claude-code user** (Al vs. Vincent vs. any future family
  member) — no user identifier is sent.
- **Al vs. Vincent** specifically — same guarantee, restated: the
  substrate transmits no bit that would distinguish two family-tier
  users of the same mika deployment.
- **A query pattern to an identity** — barring the query content itself
  (see below), the wire carries no correlator that would let Brave build
  a tenant-scoped session even across many requests from the same user.

The invariant is *dé-liage identité↔requête*: not that Brave can never
see any query, but that Brave cannot ex-post link a query to a mika
tenant/agent/user identity through fields the substrate controls.

---

## Query content leak (out of scope, noted explicitly)

**If the query text itself contains identifying information — e.g. a
tenant types `"quel est mon email vincent@senara-solutions.ai"` and the
agent forwards it verbatim as the search query — the invariant is
broken *by data*, not by protocol.**

The substrate's E3 invariant covers the **protocol** surface: headers,
query params, body, source IP, timing. It cannot inspect the semantic
content of the query without becoming a per-tenant content-classifier,
which would itself violate Q4 STRIP TOTAL (the substrate MUST NOT read
query content). By design, this is out of scope for E3.

**Doctrinal precedent:** [mika#1783](https://github.com/senara-solutions/mika/issues/1783)
(Al leak). The failure mode there was an agent's *scope-of-being* — the
family-tier agent naming Vincent in a chat with Al, delegating a
substrate-config problem across the tenant boundary. Prime bearing tranché
2026-07-19: **"l'être n'appelle jamais la maison"** — the agent's
doctrine layer is responsible for keeping identity out of substrate-
directed content. The same principle applies to search queries: an
agent that forwards a query containing an identifier is a doctrine-layer
failure, not a substrate-layer failure.

**Surface for upstream (agent doctrine layer):** the agent-side skill
that constructs search queries (currently the `web_search` builtin in
`crates/mika-agent/src/skills/builtin_handlers.rs`, pending migration
to `/internal/search`) SHOULD apply the same scope-of-being discipline
before invoking the substrate. This is tracked as an agent-side concern
and not addressed by E3. If a future incident surfaces the class, file
against agent-doctrine (analogous to mika#1783), not against the
egress substrate.

The substrate's contribution to the class is: it does not compound the
leak. A query containing `vincent@senara-solutions.ai` shows up on
Brave's side as a single line in a log stream of thousands of queries
from the shared source IP, with no per-tenant correlator to tie the
line back to the tenant that produced it. The single-request content
is visible; the tenant-attribution across requests is not.

---

## Enforcement summary (build-time invariants + CI)

| Layer                     | What                                                              | Where                                                                                                                    |
|---------------------------|-------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------|
| Rust visibility           | `SearchEgressClient` and friends are `pub(crate)` in mika-gateway | [`egress_search/mod.rs`](../src/egress_search/mod.rs)                                                                    |
| Rust visibility           | The `reqwest::Client` inside `SearchEgressClient` is private      | [`egress_search/mod.rs`](../src/egress_search/mod.rs)                                                                    |
| Structural test           | LOG side: no tenant / query / forbidden fields in tracing         | `mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`                                                    |
| Structural test (E3 new)  | REQUEST side: headers are shared-only                             | `tests_e3_request_shape::outgoing_headers_are_shared_only`                                                               |
| Structural test (E3 new)  | REQUEST side: query params carry no identifier                    | `tests_e3_request_shape::query_params_carry_no_identifier`                                                               |
| CI grep                   | Only `egress_search/*` may name a search-upstream host            | [`scripts/verify-egress-uniqueness.sh`](../../../scripts/verify-egress-uniqueness.sh)                                    |
| CI grep (E3 new)          | Only `egress_search/*` may construct a Brave-targeted request or add a user-identifier field | [`scripts/verify-egress-request-shape.sh`](../../../scripts/verify-egress-request-shape.sh)  |

The three structural tests (LOG + 2× REQUEST) together form the E3 invariant
coverage. Together with `verify-egress-uniqueness.sh` (existing) and
`verify-egress-request-shape.sh` (new), the substrate ships a build-time
guarantee that no user identity crosses the substrate boundary.

Runtime egress firewall (iptables / nft container-level rules) is
[E4 (#1810)](https://github.com/senara-solutions/mika/issues/1810) — a
network-layer defense-in-depth on top of these build-time invariants.

---

## What this doc does NOT cover

- **Retention** — E4 (#1810). No-log on the gateway egress path, no
  per-request row in a database, iptables-level enforcement.
- **Content-layer identity leakage** — mika#1783 precedent. Agent
  doctrine layer, not substrate layer.
- **Fingerprinting via TLS ClientHello / JA3** — the reqwest `rustls-tls`
  fingerprint is constant across the fleet (single reqwest build →
  single JA3 for the gateway); not per-tenant. Recorded here for
  completeness; not a v1 threat.
- **Response-side leaks** — the response body flows back to the calling
  agent unchanged; the substrate emits only `{upstream, latency_ms,
  status}`. If a future agent-side skill logs the raw response, it is
  an agent-side observability concern, not a substrate leak.
