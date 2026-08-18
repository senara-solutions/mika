# Egress Search Substrate

Module: [`crates/mika-gateway/src/egress_search/`](../src/egress_search/) (split — `mod.rs` owns marker types + handler + Q4 test; [`brave.rs`](../src/egress_search/brave.rs) owns the concrete HTTP call for Brave)
Owner: mika-gateway
Tickets:
- [mika#1807](https://github.com/senara-solutions/mika/issues/1807) — E1 keystone of milestone [#1806](https://github.com/senara-solutions/mika/issues/1806) (substrate, marker types, Q4 log-absence test)
- [mika#1808](https://github.com/senara-solutions/mika/issues/1808) — E2 Brave client wired behind the substrate (this doc reflects post-E2 state)
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
| `200`  | `SearchResponse` — results from the upstream. `upstream_latency_ms` is the wall-clock time observed at the substrate. |
| `401`  | Missing / wrong bearer token (mika-spirit → gateway auth).                         |
| `404`  | No upstream configured for this deploy (`MIKA_SEARCH_UPSTREAM` unset).              |
| `501`  | Reserved for a future `SearchUpstream` variant with no concrete client wired.      |
| `502`  | Upstream returned non-2xx (`error: "upstream_error"` / `"unauthorized"` / `"parse_error"`) or transport failed (`error: "transport_error"`). |

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

Where `status ∈ {"ok", "not_implemented", "upstream_error", "unauthorized", "transport_error", "parse_error"}`. The `not_implemented` label is reserved for future upstream variants that don't yet have a concrete client wired — the Brave upstream never emits it post-E2.

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

## Brave upstream — concrete wire format (E2, mika#1808)

The Brave client lives in [`brave.rs`](../src/egress_search/brave.rs) and is
the only concrete `SearchUpstream` variant with a wired network path today.
It is invoked by `SearchEgressClient::search` when constructed with
`SearchUpstream::Brave(BraveConfig { .. })`.

**Endpoint:** `GET https://api.search.brave.com/res/v1/web/search` (overridable via `MIKA_BRAVE_ENDPOINT` for tests and self-hosted mirrors).

**Request:**
- Header `X-Subscription-Token: <MIKA_BRAVE_API_KEY>` (kept in `secrecy::SecretString` end-to-end; `.expose_secret()` is called only at the header write site).
- Header `Accept: application/json`.
- Query params `q=<caller query>` + `count=<max_results clamped to [1,20]>`. The clamp is a defense — a caller asking for 1000 results would burn quota and trip Brave's cap.

**Response shape parsed** (`web.results[].{title, url, description}` → `SearchResult { title, url, snippet }`). Missing `web` block or empty `results` returns an empty `Vec` — this is a valid "no hits" outcome, not a `parse_error`.

**Timeout budget:** the substrate holds an overall 5s wall-clock cap ([`EGRESS_HARD_TIMEOUT_SECS`](../src/egress_search/mod.rs)). The `reqwest::Client` per-request timeout is 3s; the retry window (below) is bounded so initial + retry + backoff cannot exceed the wall-clock cap.

**Retry policy** (at most one retry — Brave freemium is ~2000 requests / month, so retries are expensive):

| Upstream response       | Retry?                                                        | Terminal error taxonomy         |
|-------------------------|---------------------------------------------------------------|---------------------------------|
| 2xx                     | success                                                       | `ok`                            |
| 401 / 403               | **NO** — fail fast so operator can rotate key without burning quota | `unauthorized`             |
| 429                     | YES — honor `Retry-After` (numeric seconds only, capped at 2s); if persistent, terminal | `upstream_error` (with status 429) |
| 5xx                     | YES — 500ms backoff                                           | `upstream_error`                |
| Transport / timeout     | YES — 500ms backoff                                           | `transport_error`               |
| Other 4xx (400/404/…)   | NO — malformed request unlikely to succeed on retry           | `upstream_error`                |
| 2xx body that won't parse | NO — schema drift, not flakiness                            | `parse_error`                   |

The retry wait is `min(nominal_wait, remaining_budget)` — a `Retry-After: 30s` at 4s into the budget returns 429 immediately rather than blocking the caller.

**Rate limits & fallback:** Brave freemium quota is ~2000 requests / month. The substrate does NOT hold a per-tenant counter (Q3 partagé no-log), so quota exhaustion surfaces as `unauthorized` / `upstream_error(429)` — the operator dashboard tracks aggregate counts and rotates keys or upgrades tier. There is no fallback upstream in E2; the SearXNG contingency lives in [E6 (#1812)](https://github.com/senara-solutions/mika/issues/1812) and would be wired as a second `SearchUpstream` variant (same substrate, different client module).

**Security discipline** (enforced structurally):
- `BraveConfig.api_key` is `SecretString` — its `Debug` impl redacts, and it drop-zeroizes.
- The Brave module emits **zero** `tracing` calls. All observability comes from the parent module's two-event audit (Q4 STRIP TOTAL).
- The reqwest error types (whose `Debug` can include URL context, and hence query bytes) are **discarded** on the failure path — the substrate returns only the taxonomy label. See `send_once` and `parse_brave_body` for the explicit drops.
- The `reqwest::Client` is a shared factory ([`build_client`](../src/egress_search/mod.rs)) — no other code path in the crate constructs a client pointed at a search upstream. Enforced by `scripts/verify-egress-uniqueness.sh`.

**Test surface:**
- Unit: URL construction (`count` clamp), `Retry-After` parsing, response schema tolerance for missing fields.
- Wiremock integration (`brave.rs::wiremock_integration`): success + zero-hits + 401 fail-fast + 403 fail-fast + 429 retry-then-succeed + 429 persistent → terminal + 500 retry + 400 no-retry + parse-error + max-results clamp + success-path log-absence.
- The Q4 log-absence discipline test lives in `mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields` and exercises the transport-error path; the wiremock success-path variant complements it.

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
