# Egress Search Substrate

Module: [`crates/mika-gateway/src/egress_search.rs`](../src/egress_search.rs)
Owner: mika-gateway
Ticket: [mika#1807](https://github.com/senara-solutions/mika/issues/1807) (E1 keystone of milestone [#1806](https://github.com/senara-solutions/mika/issues/1806))
Plan: [`docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md`](../../../docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md)

---

## Purpose

The **single controlled + instrumented egress point** through which every Mika
agent search request transits before it reaches a search upstream (Brave, and
future contingencies). This is the doctrinal keystone: without it, E2's Brave
integration is a banal API call and the no-log / dé-liaison property that
milestone #1806 promised is lost.

Analogy: same discipline as [`#1796`](https://github.com/senara-solutions/mika/issues/1796)
(voice testimony non-transit build-time invariant). **Build the incapacity;
do not promise the restraint.**

---

## HTTP interface

```
POST /internal/search
Authorization: Bearer <MIKA_INTERNAL_TOKEN>
Content-Type: application/json

Body: {"query": "…", "max_results": 5}
```

| Status | Meaning                                                                            |
| ------ | ---------------------------------------------------------------------------------- |
| `200`  | `SearchResponse` — E2 onward.                                                      |
| `401`  | Missing / wrong bearer token.                                                      |
| `404`  | No upstream configured for this deploy (`MIKA_SEARCH_UPSTREAM` unset).              |
| `501`  | E1 substrate live but E2 (#1808) has not yet wired the concrete upstream call.     |
| `502`  | Upstream returned non-2xx / transport error (E2 onward).                           |

Response bodies carry only a small JSON envelope:
- Success (200): `{"results": [...], "upstream_latency_ms": <u32>}`
- Failure: `{"error": "<taxonomy-label>"}` — never the query, never the tenant,
  never the upstream response body.

**No other endpoint in the platform reaches a search upstream.** Agents inside
per-customer containers call `POST /internal/search` on the shared gateway;
they do not import Brave / SearXNG client code.

---

## Marker-type discipline (build-time invariant)

Q2 tranchage — quadruple discipline, mirroring [`#1796`](https://github.com/senara-solutions/mika/issues/1796):

### 1. Marker types + trait bounds

`SearchEgressClient` privately wraps a `reqwest::Client` and a
`SearchUpstream` enum. No conversion from a general `reqwest::Client` is
possible. The handler accepts only `&SearchEgressClient`. Any code that
wants to reach the upstream **must** go through this type.

```rust
pub(crate) struct SearchEgressClient {
    inner: reqwest::Client,     // private; never returned by reference
    upstream: SearchUpstream,   // enum forced at construction
}
```

The `SearchUpstream` enum is not `#[non_exhaustive]` — adding a variant is
an explicit source change and forces every match site to update.

### 2. Module visibility (`pub(crate)` only)

Every export from `egress_search.rs` is `pub(crate)`. Downstream crates
(`mika-agent`, `mika-a2a`, `mika-cli`, etc.) **cannot import**
`SearchEgressClient`, `SearchUpstream`, `BraveConfig`, or the handler
type. The Rust visibility system is the primary enforcement layer.

### 3. CI lint (`scripts/verify-egress-uniqueness.sh`)

A repo-level grep gate rejects any hit for a known search-upstream
identifier (Brave host, Brave API path, future contingencies) that lives
outside `crates/mika-gateway/src/egress_search*`. The script runs as its
own CI job (`egress-uniqueness-lint`) on every PR.

Adding a new upstream (SearXNG etc.) means:
1. Adding a variant to `SearchUpstream`.
2. Adding a settings arm + validation.
3. Adding the upstream's identifier strings to the CI script's grep
   patterns.

There is no allowlist-by-suppress-comment escape hatch — the whole point
is that the substrate stays the sole reachability path.

### 4. Runtime egress firewall — **OUT OF E1 SCOPE**

iptables / nft container-level rules restricting egress to the whitelisted
upstream domain live in the E4 ticket
([#1810](https://github.com/senara-solutions/mika/issues/1810)) —
network-layer defense-in-depth on top of the build-time invariant here.

---

## Q4 — Instrumentation (STRIP TOTAL v1)

**Sami-tranchée 2026-08-18, re-confirmed 06:41 & 06:51 UTC.** ZERO tenant
identifier of any kind — not raw, not hashed, not bucketed. ZERO query
content. ZERO upstream response body. Two structured `tracing` events
leave this module and nothing else:

### `search_requested` (INFO)

```json
{"event": "search_requested", "upstream": "brave", "message": "search egress requested"}
```

### `search_egress` (INFO — audit event shape)

```json
{"event": "search_egress", "upstream": "brave", "latency_ms": <u32>, "status": "<taxonomy>", "message": "search egress audit event"}
```

Where `status ∈ {"ok", "not_implemented", "upstream_error", "transport_error"}`.

That's it. **No other fields.** A CapturingLayer test
(`egress_search::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`)
asserts by construction that any additional field is a Q4 violation and
would fail CI. This is the load-bearing discipline test: if a future PR
adds a `tenant_hash` / `tenant_id` / `query` / `chat_id` / `customer_id`
attribute to either event, that test fails.

### What is NEVER logged

- Query string content (`user searched for X`)
- User / tenant identifier — **neither raw, nor hashed, nor bucketed**
  (see rationale below)
- Upstream response body
- API key / credentials
- Result URLs (leaks tenant browsing pattern)

### Rationale for strip total (sami 2026-08-18)

Bucket-64 tenant_hash was proposed for SRE cohort trouble-shooting, but
sami arbitrated strip total in v1:

> Raison : bucket-64 sur une famille de quelques dizaines d'utilisateurs
> ≈ pseudonyme quasi-par-tenant → corrélable user-side, ce que la
> doctrine dé-liage interdit. Réversibilité-asymétrie : ajouter de la
> télémétrie-cohorte plus tard (avec bearing Prime) = facile ;
> dé-fuiter = dur. On assume zéro cohort-debug en v1.

**Operational consequences v1:**
- No SRE cohort-debug ("X % errors on bucket Y" — not possible).
- Trouble-shooting relies on global aggregate metrics only (total
  counter + latency histogram — future v2 wiring).
- If cohort-debug becomes necessary, it will be a **feature v2 gated
  Prime** — never a silent additive change.

---

## Q3 — Multi-tenant sharing (Prime 2026-07-19)

One shared `SearchEgressClient` instance in the gateway, `Arc`-carried
across every tenant, every request. No per-tenant state (no session, no
cache, no rate-limit counter). Prime rule:

> centralité ≠ violation ; visibilité = violation. Partagé no-log-vérifié OK.

The Q4 no-log invariant (above) is the guarantee that shared centrality
does not produce ex-post correlation. The audit event carries no data
that could re-tie a request to a tenant.

---

## Configuration

| Env var                 | Required                       | Purpose                                                        |
| ----------------------- | ------------------------------ | -------------------------------------------------------------- |
| `MIKA_SEARCH_UPSTREAM`  | No (endpoint 404s when absent) | Selector. v1 accepts only `brave`. Unknown values hard-fail.   |
| `MIKA_BRAVE_API_KEY`    | Yes when upstream = brave      | `X-Subscription-Token` for the Brave API (E2 wires the send).  |
| `MIKA_BRAVE_ENDPOINT`   | No                             | Override the canonical Brave URL (E2 integration tests / self-hosted mirrors). |

Settings validation (in `crates/mika-gateway/src/settings.rs`) enforces
that `MIKA_SEARCH_UPSTREAM=brave` implies `MIKA_BRAVE_API_KEY` is present.

---

## Cross-references

- Milestone: [#1806](https://github.com/senara-solutions/mika/issues/1806) — egress-controlled search backend
- Sibling tickets: [#1808 (E2)](https://github.com/senara-solutions/mika/issues/1808), [#1809 (E3)](https://github.com/senara-solutions/mika/issues/1809), [#1810 (E4)](https://github.com/senara-solutions/mika/issues/1810), [#1811 (E5)](https://github.com/senara-solutions/mika/issues/1811), [#1812 (E6)](https://github.com/senara-solutions/mika/issues/1812)
- Bearing analogy: [#1796](https://github.com/senara-solutions/mika/issues/1796) — voice testimony non-transit build-time invariant
- Plan document: [`docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md`](../../../docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md)
- CI lint: [`scripts/verify-egress-uniqueness.sh`](../../../scripts/verify-egress-uniqueness.sh)
