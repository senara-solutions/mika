---
title: Egress Search — SearXNG Self-Hosted Contingency (E6)
description: Design-only escalation path for a self-hosted SearXNG upstream behind the egress substrate. Contingency — not a build. Activated only on a documented trigger (E4 unverifiable OR concrete threat OR scale pays the infra).
---

# Egress Search — SearXNG self-hosted contingency (E6)

> **Prime line — non-negotiable.**
> *Construis l'incapacité, ne promets pas la retenue.*
> (Build the incapacity, don't promise the restraint.)

> **Prime↔coherence 2026-07-19 (samidarko relay), the reason this is contingency and not a build:**
> *SearXNG re-forward Google/Bing donc ne livre PAS le non-transit ; le vrai levier = l'egress. Self-hosting = un moyen parmi d'autres, à réserver à quand menace/échelle le paient.*

**Status: DESIGN-ONLY.** This document is an architectural option held in reserve. No SearXNG code, no container, no provisioning, and no deployment ship from it. It stays in the backlog as a documented escalation path until a trigger (§ Activation triggers) fires and the operator (Prime bearing) escalates it to a build.

Ticket: [`senara-solutions/mika#1812`](https://github.com/senara-solutions/mika/issues/1812) — E6 of milestone [`#1806`](https://github.com/senara-solutions/mika/issues/1806) (egress-controlled search backend).
Companion docs (the shipped E1–E4 substrate this design extends):
- [`crates/mika-gateway/docs/egress-search.md`](../crates/mika-gateway/docs/egress-search.md) — E1/E2 substrate architecture (`SearchEgressClient`, `POST /internal/search`, Q4 STRIP TOTAL, the `SearchUpstream` enum).
- [`crates/mika-gateway/docs/egress-search-threat-model.md`](../crates/mika-gateway/docs/egress-search-threat-model.md) — E3 dé-liage identité↔requête (what the upstream sees on the wire).
- [`crates/mika-gateway/docs/egress-search-no-log-audit.md`](../crates/mika-gateway/docs/egress-search-no-log-audit.md) — E4 no-log verified end-to-end (the three-layer runbook, and the SearXNG escalation trigger).
- Bearing analogy: [`docs/voice-non-transit-invariant.md`](voice-non-transit-invariant.md) — the same "build the incapacity" discipline for the voice testimony lane.

---

## 1. What E4 is, and why SearXNG is an *escalation from* it

### The current search egress path (E1–E4, shipped)

Milestone [#1806](https://github.com/senara-solutions/mika/issues/1806) established that Mika's web-search backend is **a hosted API (Brave) behind a controlled + instrumented egress**. The deliverable is the *egress-control*, not the choice of Brave. Four sub-issues built it, all shipped:

| Layer | Ticket | What it delivers |
| - | - | - |
| **E1** | [#1807](https://github.com/senara-solutions/mika/issues/1807) | The **single controlled egress point** — `SearchEgressClient` (marker type, `pub(crate)` only), `POST /internal/search`, the `SearchUpstream` enum, Q4 STRIP-TOTAL log discipline, CI uniqueness lint. |
| **E2** | [#1808](https://github.com/senara-solutions/mika/issues/1808) | The Brave client wired as the one concrete `SearchUpstream::Brave` variant ([`brave.rs`](../crates/mika-gateway/src/egress_search/brave.rs)). |
| **E3** | [#1809](https://github.com/senara-solutions/mika/issues/1809) | **Dé-liage identité↔requête** — the outgoing request carries, by construction, no tenant/agent/user field on any header, query param, body, or source-IP. |
| **E4** | [#1810](https://github.com/senara-solutions/mika/issues/1810) | **No-log verified end-to-end** — the RETENTION half of the Prime invariant. |

The four together enforce the Prime 2026-07-19 invariant: **« zéro liage requête↔identité + zéro rétention »**, as a *build-time* property, not a runtime promise.

### What E4 specifically is

E4 ([`egress-search-no-log-audit.md`](../crates/mika-gateway/docs/egress-search-no-log-audit.md)) closes the **retention** half and — crucially — *verifies* it across three layers:

1. **Application logs** — the substrate emits only the two Q4 audit events (`search_requested`, `search_egress`), no query/tenant/URL bytes. Enforced by `scripts/verify-egress-no-log.sh` (CI) + the `CapturingLayer` runtime test.
2. **Network metadata** — iptables/nft, proxy access logs, VPC flow logs on the egress hop must carry no log. This layer is **SPEC-only**: E4 delivers the specification; the actual K8s/iptables/proxy config is a mika-cloud follow-up. `scripts/audit-egress-no-log.sh` exits `2` on every run to force the operator to attest Layer 2 out-of-band.
3. **Persistence** — no file/SQLite writes, no per-query cache. Build-time lint + runtime SQLite probe.

The doctrinal claim is ***no-log par construction, vérifié*** — construction alone is not enough; the active check that nothing leaked is the "vérifié" half.

### Why SearXNG is an E6 escalation, not the default

E4 itself names the escalation ([`egress-search-no-log-audit.md` § Failure mode → SearXNG escalation trigger](../crates/mika-gateway/docs/egress-search-no-log-audit.md)):

> Si l'audit E4 révèle qu'on ne peut PAS vérifier end-to-end (ex : proxy logging obligatoire, ou observability impose per-query trace), c'est LE signal pour escalader à SearXNG self-hosted.

Held *not* to be the default because of the **beauty-trap** the milestone records explicitly:

- SearXNG **re-forwards** the query to Google/Bing/etc. It does **not** deliver "non-transit" — the query still transits to a third-party engine. The real lever on the doctrinal property was always the *egress-control*, which E1–E4 already give against Brave.
- Self-hosting **pays maintenance** for a guarantee (no-log / dé-liage) that egress-control already delivers. Paying that cost before a threat or scale justifies it is the trap.
- A **botched self-host** (default logs, network-layer egress metadata, a persistence cache, a SPOF) is **worse than Brave**, because it re-ties the tenant to the query *on our side* — the exact thing E3+E4 spent their whole surface preventing.

So E6 is reserved. It becomes worth its maintenance cost **only** when one of three conditions makes egress-control-against-a-hosted-API insufficient — see § 3.

---

## 2. The design — how a self-hosted SearXNG slots into the substrate

The load-bearing property of the design is that **nothing on the agent side changes**. SearXNG enters as a second `SearchUpstream` variant behind the *same* `SearchEgressClient`, reachable through the *same* `POST /internal/search` contract. `web_search` and `fetch_url` route exactly as they do today.

### 2.1 The swap seam (interface contract — "mêmes types que E2")

The substrate was built for exactly this extension. From [`egress-search.md` § Marker-type discipline](../crates/mika-gateway/docs/egress-search.md):

> Adding a new upstream (SearXNG etc.) means: (1) Adding a variant to `SearchUpstream`. (2) Adding a settings arm + validation. (3) Adding the upstream's identifier strings to the CI script's grep patterns.

Concretely:

- **Enum variant** — add `SearchUpstream::Searxng(SearxngConfig)` to the enum in [`egress_search/mod.rs`](../crates/mika-gateway/src/egress_search/mod.rs). The enum is deliberately **not** `#[non_exhaustive]`, so the new variant forces every match site to update — a compile error, not a silent fallthrough.
- **Client module** — a `searxng.rs` module mirroring [`brave.rs`](../crates/mika-gateway/src/egress_search/brave.rs): a `pub(crate)` client that issues the concrete HTTP call and parses SearXNG's JSON response into the existing `SearchResult { title, url, snippet }` shape. SearXNG exposes a JSON API (`GET /search?q=…&format=json`); its `results[].{title, url, content}` map onto `SearchResult`. `[TODO: confirm]` the exact JSON field names against the pinned SearXNG version at build time — do not hardcode from memory.
- **The agent surface does not move.** Agents call `POST /internal/search` with `{query, max_results}` and receive `SearchResponse { results, upstream_latency_ms }`. They do not import upstream client code (E1 invariant). The `web_search` builtin already routes through the substrate ([#1971](https://github.com/senara-solutions/mika/issues/1971)); `fetch_url` is a **sibling egress class** (`egress_fetch`, [#1969](https://github.com/senara-solutions/mika/issues/1969)) — see § 5 for whether it is in scope.

**Result: a transparent swap.** Flipping `MIKA_SEARCH_UPSTREAM` from `brave` to `searxng` changes the upstream with zero change to any agent-side code, matching the ticket's "swap transparent, aucun code Mika côté agent ne change".

### 2.2 Configuration

Following the E2/E4 config pattern in [`egress-search.md` § Configuration](../crates/mika-gateway/docs/egress-search.md) and `crates/mika-gateway/src/settings.rs`:

| Env var | Purpose |
| - | - |
| `MIKA_SEARCH_UPSTREAM=searxng` | Selects the SearXNG variant (v1 accepts only `brave` today; add the arm). |
| `MIKA_SEARXNG_ENDPOINT` | The base URL of **our** SearXNG instance. Directly analogous to the already-existing `MIKA_BRAVE_ENDPOINT` override, which the E2 doc describes as being *for tests and self-hosted mirrors* — the seam is already anticipated. |
| `MIKA_SEARXNG_TOKEN` (optional) | If the instance is fronted by a shared bearer, kept in `secrecy::SecretString` end-to-end like `MIKA_BRAVE_API_KEY`. `[TODO: confirm]` whether the chosen deployment fronts SearXNG with auth or relies purely on network placement (§ 2.4). |

Settings validation enforces that `MIKA_SEARCH_UPSTREAM=searxng` implies `MIKA_SEARXNG_ENDPOINT` is present, mirroring the existing `brave ⇒ MIKA_BRAVE_API_KEY` check.

### 2.3 CI uniqueness lint

The SearXNG host/endpoint identifier strings must be added to `scripts/verify-egress-uniqueness.sh`'s grep patterns, so that only `crates/mika-gateway/src/egress_search/*` may name them. Same discipline as Brave: no code path outside the substrate may construct a request pointed at the search upstream, and there is no suppress-comment escape hatch.

### 2.4 Deployment shape and network placement

The instance must sit **behind the egress boundary, close to the gateway** (the ticket's latency preference: *proche gateway*). Placement candidates, in rough preference order:

1. **mika-cloud, co-located with the gateway** (K8s pod / sidecar in the gateway's namespace). Preferred: same trust domain, lowest latency, Layer-2 network policy is authored where the E4 iptables/NetworkPolicy work already lives (mika-cloud follow-up). `[TODO: confirm]` the concrete mika-cloud topology with the mika-cloud circle (MPC) at activation time.
2. **A dedicated VPS near the gateway region.** Simpler to stand up; adds a second host to audit for Layer-2 no-log and a network hop to secure.
3. **gentux / host-local.** Only for a single-host family deploy; not a shared-tenant answer.

**The single-egress invariant, re-examined (the crux).** With a hosted Brave, the gateway is the single egress point and Brave is *outside*. With self-hosted SearXNG there are now **two hops**:

```
agent ──POST /internal/search──▶ gateway (SearchEgressClient)
        ──▶ SearXNG (ours) ──▶ Google / Bing / … (the real engines)
              hop A (internal)      hop B (external, fan-out)
```

- **Hop A** (gateway → SearXNG) is *internal* to our trust domain. It replaces the old external Brave call. It must be no-log at Layer 2 exactly as E4 specifies for the Brave hop.
- **Hop B** (SearXNG → the engines) is the **new external egress surface**. SearXNG itself now makes the outbound calls that Brave used to make on Brave's own infrastructure. The single-controlled-egress property must therefore be *re-established for SearXNG's own egress*: SearXNG becomes a second controlled egress point, and its no-log discipline (§ 2.5) is what keeps the substrate's guarantee intact. This is the added surface § 6 accounts for.

### 2.5 Trust / isolation properties

- **No-log by construction, on the SearXNG instance too.** SearXNG must be configured with query logging OFF, no persistent result cache that stores queries, and no access log on hop A or hop B (the E4 Layer-2 iptables/proxy `access_log off` requirements extend to cover SearXNG's front and its upstream calls). This is non-negotiable: E4's whole point is that a self-host that logs by default is *worse* than Brave.
- **Our instance, not Al's shared one.** Al already operates a SearXNG for SEO work (samidarko context) and help is available — but the escalation requires an instance that is **ours + no-log**, not a shared third-party instance, precisely to avoid re-introducing the re-liage E3 removed. Reusing a shared instance would let a third party correlate our tenants' queries: the exact failure E3 closes.
- **Shared, no-log-verified topology (preferred), per Q3.** Per [`egress-search.md` § Q3](../crates/mika-gateway/docs/egress-search.md), Prime ruled *centralité ≠ violation ; visibilité = violation ; partagé no-log-vérifié OK*. So the preferred instance topology is **one shared no-log-verified SearXNG** across all tenants (matching the shared `SearchEgressClient` Arc). A per-person instance topology is defense-in-depth only — it does not add a doctrinal guarantee beyond what a verified-no-log shared instance already gives, and it multiplies the maintenance surface.
- **Dé-liage carries over unchanged on hop A.** The E3 request-shape invariant (`outgoing_headers_are_shared_only`, `query_params_carry_no_identifier`) applies to the SearXNG-targeted request the same way: no tenant/agent/user field is added. The E3 lint tests must be extended to enumerate the SearXNG request's headers/params (the doc already notes: *if a future upstream requires POST, the E3 lint MUST be extended to enumerate that upstream's body fields explicitly*). SearXNG's JSON API is a `GET` with a `q` param, so the existing shape largely transfers. `[TODO: confirm]` the exact SearXNG request signature at build time.

---

## 3. Activation triggers — concrete and observable

E6 becomes a build ticket if **any one** of the three conditions fires. Each is an **operator decision (Prime bearing), not an automated fallback** (per [`egress-search-no-log-audit.md`](../crates/mika-gateway/docs/egress-search-no-log-audit.md)). The verbatim conditions from [#1812](https://github.com/senara-solutions/mika/issues/1812), made observable:

### Trigger 1 — E4 comes back impossible-to-verify

**Verbatim:** *E4 remonte impossible-à-vérifier : l'audit no-log end-to-end découvre qu'on ne peut pas garantir zéro rétention côté substrat (obligation logs, obligation observability per-query, contrainte de plate-forme cloud). → SearXNG self-host devient nécessaire, car la propriété doctrinale ne peut plus tenir à l'egress seul.*

**Observable signal:** the operator cannot honestly bring `scripts/audit-egress-no-log.sh` to **exit 0** — Layer 2 cannot be confirmed clean and `MIKA_AUDIT_SUPPRESS_L2_WARN=1` cannot be truthfully set. Concretely: the deploy environment *structurally mandates* network-layer logging — a corporate/cloud proxy enforces access logging with no per-route exception, or an observability platform mandates per-request tracing at the network layer. When egress-control-against-Brave can no longer hold the no-log property because the *platform* logs the Brave hop, self-hosting SearXNG lets us own the logging surface end-to-end.

### Trigger 2 — a concrete, documented threat

**Verbatim:** *Menace concrète documentée : évidence qu'un upstream (Brave ou autre) collectant les queries devient une menace matérielle (change ToS, incident sécurité, gouvernement subpoena). → self-host pour reprendre contrôle.*

**Observable signal:** a specific, dated, documented event — a Brave ToS change asserting query retention/resale, a disclosed Brave-side security incident exposing query logs, or a subpoena/legal-process demand reaching the upstream. Not a hypothetical: the trigger is *evidence*, recorded against this ticket, that the hosted upstream became a material collector of our queries.

### Trigger 3 — scale pays the infrastructure

**Verbatim:** *Échelle paie l'infra : plus de tenants Mika qu'un partagé Brave rentable → self-host avec instance partagée devient économique.*

**Observable signal:** the aggregate request counter (the operator dashboard's global metric — the only per-tenant-free telemetry the substrate keeps, per Q4) sustainedly exceeds what the Brave tier sustains economically. Anchor: Brave freemium is **~2000 requests/month** ([`egress-search.md` § Brave upstream](../crates/mika-gateway/docs/egress-search.md)); quota exhaustion already surfaces today as `unauthorized` / `upstream_error(429)`. When the fleet's steady-state volume makes a paid Brave tier cost more than running one shared no-log SearXNG, the economics flip and a shared self-host becomes the cheaper controlled egress.

**On fire:** the ticket's escalation is to relabel [#1812](https://github.com/senara-solutions/mika/issues/1812) `p3-nice-to-have → p1-important` and start the build. The ticket stays OPEN as the contingency reference until then.

---

## 4. What it would take to implement (rough scope) + what stays deferred

### In scope when the trigger fires (rough scope, not a plan)

**mika-gateway (code):**
- `SearchUpstream::Searxng(SearxngConfig)` enum variant + a `searxng.rs` client module mirroring `brave.rs` (request build, timeout/retry budget, JSON parse into `SearchResult`, zero `tracing` in the module — all observability via the parent Q4 audit).
- Settings arm + validation (`searxng ⇒ MIKA_SEARXNG_ENDPOINT`).
- Extend `scripts/verify-egress-uniqueness.sh` grep patterns with the SearXNG identifier tokens.
- Extend the E3 request-shape tests + `scripts/verify-egress-request-shape.sh` to enumerate the SearXNG request's headers/params.
- Extend the E4 `verify-egress-no-log.sh` / `audit-egress-no-log.sh` coverage to the new module (and add the two-hop Layer-2 attestation).

**mika-cloud (infra — separate repo, the E4 Layer-2 pattern):**
- Provision the SearXNG container with a no-log configuration (query logging off, no persistent query cache).
- Network placement behind the egress boundary, close to the gateway; iptables/NetworkPolicy for **both** hops (gateway → SearXNG, and SearXNG → the engines), with no `--log-prefix` / `access_log` on either.
- A no-log audit for the SearXNG host equivalent to the gateway's three-layer runbook.

**Migration (tenant-by-tenant):**
- Default rollout is a **progressive per-tenant dispatch**: flip `MIKA_SEARCH_UPSTREAM` for a canary tenant first, verify search works + the no-log audit stays clean, then widen. A **global flip** is acceptable for a small family fleet, since rollback is a single env-var revert (same "direct switch, no env toggle" philosophy [#1971](https://github.com/senara-solutions/mika/issues/1971) used).
- **Fallback if SearXNG is down:** SearXNG self-host is a SPOF (§ 6). The migration must keep Brave configurable as a fallback upstream so a SearXNG outage degrades to Brave rather than to no-search. `[TODO: confirm]` whether fallback is an automatic in-substrate second-variant attempt or an operator-flipped env revert — the former adds substrate complexity, the latter is simpler and matches the "operator decision" posture; recommend operator-flipped unless scale (Trigger 3) argues for automatic.

### Deferred by design (until the trigger fires)

- Any SearXNG code, `SearxngConfig`, or `searxng.rs`.
- Any container image, compose file, Dockerfile, or Helm/K8s manifest.
- Any provisioning, key/endpoint secret, or deployment.
- Any change to the shipped Brave path.

Per the ticket: *Aucun code SearXNG dans ce ticket. Aucun provisioning. Documentation seule + condition d'activation.*

---

## 5. Scope note — search vs. the `fetch_url` egress class

This design covers the **search** egress class (`egress_search`, the `web_search` builtin). The `fetch_url` builtin is a **separate, mirrored egress class** (`egress_fetch`, [#1969](https://github.com/senara-solutions/mika/issues/1969)) — a compile-time-allowlisted GET substrate for gouv.fr pages, not a search meta-engine consumer. SearXNG does not sit under `egress_fetch`. The same "reserve the self-host until a trigger pays for it" reasoning *could* be restated for `egress_fetch` if one of its allowlisted upstreams became a threat, but that is **out of scope** for E6 and would be its own contingency doc. Recording the boundary here so a future reader does not conflate the two classes.

---

## 6. Risks / tradeoffs

| Risk | Assessment |
| - | - |
| **Maintenance burden** | Running a search meta-engine is ongoing work: SearXNG version upgrades, and engine breakage as Google/Bing rotate scraping defenses (captchas, HTML/JSON drift, rate-limits, IP blocks). Result quality **degrades silently** as engines change; there is no vendor SLA. This is the maintenance the beauty-trap warns about — only worth paying once a trigger justifies it. |
| **Added egress surface** | The single-egress property becomes **two-hop** (§ 2.4). SearXNG's own outbound fan-out to many engines is new surface to audit for no-log. A botched self-host (default logs, a persistence cache, an access log on hop B) is **worse than Brave** — it re-ties tenant↔query on *our* side. E4's three-layer audit must be re-run against the SearXNG host, not just the gateway. |
| **Correctness / non-transit** | SearXNG **re-forwards** to Google/Bing — the query still transits to those engines. Self-hosting does **not** deliver "non-transit"; it only moves *who* makes the outbound call. The doctrinal win (no-log / dé-liage) is one egress-control already provides against Brave. Do not oversell what SearXNG buys. |
| **SPOF / availability** | A self-hosted instance is a single point of failure; Brave's uptime is replaced by ours. Requires a fallback (§ 4) and monitoring, adding operational load. |
| **Al's instance temptation** | Reusing Al's existing shared SearXNG (help-available) would re-introduce third-party correlation — the exact re-liage E3 removes. The instance MUST be ours + no-log-verified. Named here so the convenience is not mistaken for the design. |
| **Reversibility asymmetry** | Same asymmetry E4 records for telemetry: standing up a no-log self-host later (on a real trigger, with Prime bearing) is tractable; un-leaking queries a botched early self-host retained is not. That asymmetry is *why* this is contingency — the safe default is to wait. |

**Net:** self-hosting SearXNG trades a maintained hosted dependency (Brave, ~2000 req/mo freemium, egress-controlled) for owned infrastructure that only pays off when egress-control-against-a-hosted-API stops being sufficient — i.e. exactly the three triggers. Absent a trigger, the shipped E1–E4 substrate already holds the doctrinal property, and this design stays on the shelf.

---

## Acceptance criteria

This is a **design-only** deliverable; the acceptance criteria below govern the *design doc*, not a build (per [#1812](https://github.com/senara-solutions/mika/issues/1812) "Acceptance criteria — pour design-doc, PAS build").

1. **Trigger conditions** captured verbatim + made observable — § 3 (the three: E4-unverifiable / concrete threat / scale).
2. **Ready-to-build architecture** — the swap seam (enum variant + `searxng.rs` mirror), the interface contract (`POST /internal/search` unchanged, "aucun code Mika côté agent ne change"), placement (proche gateway), and the two-hop single-egress re-examination — § 2.
3. **Migration plan tenant-by-tenant** (+ fallback if SearXNG down) — § 4.
4. **Instance topology** — shared no-log-verified (preferred) vs per-person (defense-in-depth only) — § 2.5.
5. **No code shipped** — this PR adds one Markdown design doc and nothing else. No `SearxngConfig`, no `searxng.rs`, no Dockerfile, no manifest, no provisioning.
6. **Ticket stays OPEN** as the contingency reference; activation relabels `p3 → p1` and starts the build (§ 3, on-fire).

---

## Open questions carried forward (`[TODO: confirm]` at activation time)

- Exact SearXNG JSON API field names + request signature against the pinned version (§ 2.1, § 2.5).
- Whether the instance is fronted by a bearer token or relies purely on network placement (§ 2.2).
- Concrete mika-cloud topology for placement (pod/sidecar/VPS) — decided with the mika-cloud circle (MPC) at activation (§ 2.4).
- Fallback mechanism: automatic in-substrate second-variant attempt vs. operator-flipped env revert (§ 4) — recommend operator-flipped absent a scale argument.

---

## Cross-references

- Milestone: [#1806](https://github.com/senara-solutions/mika/issues/1806) — egress-controlled search backend (no-log + minimise-et-délie).
- E1 substrate: [#1807](https://github.com/senara-solutions/mika/issues/1807) → [`egress-search.md`](../crates/mika-gateway/docs/egress-search.md).
- E2 Brave client: [#1808](https://github.com/senara-solutions/mika/issues/1808) → [`brave.rs`](../crates/mika-gateway/src/egress_search/brave.rs).
- E3 dé-liage: [#1809](https://github.com/senara-solutions/mika/issues/1809) → [`egress-search-threat-model.md`](../crates/mika-gateway/docs/egress-search-threat-model.md).
- E4 no-log verified: [#1810](https://github.com/senara-solutions/mika/issues/1810) → [`egress-search-no-log-audit.md`](../crates/mika-gateway/docs/egress-search-no-log-audit.md).
- E5 Al unblock (Brave key provisioning): [#1811](https://github.com/senara-solutions/mika/issues/1811).
- E6 (this doc): [#1812](https://github.com/senara-solutions/mika/issues/1812).
- `web_search` routed through the substrate: [#1971](https://github.com/senara-solutions/mika/issues/1971).
- Sibling egress class `fetch_url` / `egress_fetch`: [#1969](https://github.com/senara-solutions/mika/issues/1969).
- Bearing analogy (build-time incapacity): [#1796](https://github.com/senara-solutions/mika/issues/1796) → [`voice-non-transit-invariant.md`](voice-non-transit-invariant.md).
- CI lints: [`scripts/verify-egress-uniqueness.sh`](../scripts/verify-egress-uniqueness.sh), [`scripts/verify-egress-request-shape.sh`](../scripts/verify-egress-request-shape.sh), [`scripts/verify-egress-no-log.sh`](../scripts/verify-egress-no-log.sh), [`scripts/audit-egress-no-log.sh`](../scripts/audit-egress-no-log.sh).
