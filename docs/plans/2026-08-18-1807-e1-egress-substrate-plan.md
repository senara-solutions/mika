# E1 — Egress substrate for search backend (plan)

**Ticket:** [#1807](https://github.com/senara-solutions/mika/issues/1807) (Sub-issue of [#1806](https://github.com/senara-solutions/mika/issues/1806))
**Filed:** 2026-08-18 by orchestrator-CC per sami-darko routing 2026-08-18 06:33 UTC (Vincent-authorized epic priority via #1806, sami GO grooming-first).
**Status:** DRAFT — awaiting sami review; Prime bearing only if Q1 lands on Option B.

---

## WHY (frame + evidence)

Backlog #1806 (`milestone: egress-controlled search backend`) tranche que le LIVRABLE = **egress-contrôle**, pas Brave. #1807 est le keystone : sans E1, E2 (Brave integration) est un banal appel API et toute la propriété doctrinale (no-log, dé-liage) est perdue.

Analogie build-time invariant : [#1796](https://github.com/senara-solutions/mika/issues/1796) (voice testimony non-transit lane) applique la même discipline — "construis l'incapacité, ne promets pas la retenue". Marker types + trait bounds + CI gate + egress rules. Ce plan adopte la même discipline.

Bearing source :
- [#1806 body](https://github.com/senara-solutions/mika/issues/1806): Prime+coherence reconciliation 2026-07-19 (samidarko relay)
- [#1796 body](https://github.com/senara-solutions/mika/issues/1796): Vincent brief "the testimony lane's non-transit property is a build-time invariant, not a runtime toggle — verify by construction"

---

## Design questions — resolutions

### Q1 — Placement : `mika-gateway` module vs service séparé `mika-egress`

**Options considered:**

**Option A — Module in `mika-gateway`** (`crates/mika-gateway/src/egress_search.rs`)

- Pros:
  - `mika-gateway` déjà endpoint réseau externe (telegram, github clients) — cohérent
  - Zéro nouvelle unité deploy (module addition = normal deploy, no Helm chart change)
  - Faible latence (in-process call, pas hop réseau)
  - Analogie voix `#1796` : la voice-lane est aussi dans gateway
- Cons:
  - Gateway grandit — mixe tenant-router role avec observability substrate
  - Un bug substrate egress = redémarrage gateway = downtime propagée aux autres flux (telegram, github)

**Option B — Service séparé `mika-egress`** (new crate `crates/mika-egress/`, own binary + K8s pod + Helm chart)

- Pros:
  - Single-responsibility strict, auditable en isolation
  - Distinct deploy unit — restart/roll indépendant
  - Trust boundary net entre agent → egress (inter-pod call)
- Cons:
  - Nouveau deploy target : Helm chart mika-cloud, service discovery, K8s config, image build pipeline
  - Latence inter-pod (~1-5ms) vs in-process (µs)
  - Complexité opérationnelle : 2ème pod à monitorer, à alerter
  - **Souverain deploy topology** : engage la topologie K8s (nouveau service, nouveau namespace considération, nouveau ingress si externe)

**Recommandation : Option A (module gateway).**

Rationale :
1. **Cohérence avec l'invariant `#1796`** — voice testimony non-transit vit dans gateway. Egress search = même famille (build-time invariant + module isolé). La discipline transporte : *incapacité par construction*, pas *séparation par déploiement*.
2. **Deploy reversible** — module addition n'engage AUCUNE topologie irréversible côté deploy. Aucun Helm chart nouveau, aucun service discovery à recabler. Si on veut extraire vers `mika-egress` plus tard, c'est un simple crate split (le module reste self-contained).
3. **Discipline compensatrice pour "gateway grandit"** — le module `egress_search` doit être *strictement isolé* : marker types, aucune dépendance sur les autres modules gateway, tests + lint qui bloquent l'importation par d'autres crates hors du path autorisé (voir Q2).
4. **Latence** — le module = in-process, µs vs ms. Sur un chemin utilisateur (search request), 1-5ms compte.

**Impact deploy comparé :**
- Option A : zéro nouveau deploy artifact. Rebuild + roll gateway pod (déjà routine).
- Option B : nouveau crate build, nouveau Docker image, nouveau Helm chart entry, nouveau K8s Service + Deployment, nouveau ServiceMonitor si Prometheus. **Un jour de deploy prep + validation** minimum.

**Souverain check :** Option A ne touche PAS l'irréversible topology deploy. Non-souverain. Sami peut valider directement.

Si tu (sami) juges que la contrainte "gateway ne doit pas grandir davantage" est un vrai axe bearing → route Prime pour trancher Option A vs B. Sinon, GO Option A.

### Q2 — Isolation réseau (build-time invariant)

**Résolution : quadruple discipline compensatrice, comme `#1796`.**

1. **Marker types + trait bounds Rust** — `SearchEgressClient` type wrapper (privé au module), pas de conversion possible depuis un `reqwest::Client` général. Handler `search()` accepte SEULEMENT `&SearchEgressClient`.
2. **Module visibility** — `egress_search` module = `pub(crate)` uniquement pour les points d'appel autorisés (l'endpoint HTTP interne qui reçoit la requête agent). Aucun `pub` cross-crate.
3. **CI lint custom** — clippy rule ou script `scripts/verify-egress-uniqueness.sh` qui grep `Cargo.lock` + code : aucun crate mika hors `mika-gateway::egress_search` ne peut importer `reqwest::Client` pointant vers un upstream de recherche. `RUSTSEC-style` rule bloque au CI.
4. **Runtime egress firewall (defense-in-depth)** — iptables/nft rule au niveau container : seul le process gateway peut sortir vers l'upstream configuré (whitelist explicite domaine Brave/etc.). Documentée dans mika-cloud Helm chart (companion ticket futur si nécessaire — pas dans ce PR).

**Point (4) = out-of-scope pour E1** — c'est infra-layer, appartient à E4 (no-log vérifié bout-en-bout). Ce plan couvre les points 1-3 (build-time + CI). Point 4 sera traité dans le ticket E4.

### Q3 — Concurrence multi-tenants

**Résolué par Prime 2026-07-19 : instance partagée, no-log vérifié.**

Rappel Prime line (via samidarko relay) : "centralité ≠ violation ; visibilité = violation. Partagé no-log-vérifié OK."

Application : une seule `SearchEgressClient` instance dans `mika-gateway`, partagée entre tous les tenants. Aucun state per-tenant côté egress (pas de session, pas de cache, pas de rate-limit compteur per-user). Le no-log invariant (E4) garantit que même partagée, l'instance ne peut pas corréler ex post.

**Pas d'ouverture de cette question dans ce plan.**

### Q4 — Format instrumentation

**Résolution v1 (sami-tranchée 2026-08-18 06:41, re-confirmée 06:51) : `tracing` structuré + Prometheus metrics agrégés, ZÉRO tenant_hash, ZÉRO query content.**

**Metrics exposées :**
- `mika_gateway_egress_search_requests_total{status=ok|error|timeout}` — counter
- `mika_gateway_egress_search_latency_seconds` — histogram (upstream response time)
- `mika_gateway_egress_search_upstream_errors_total{code=4xx|5xx}` — counter per upstream error class

**Logs structurés (tracing crate) :**
- Level INFO : `search_requested{upstream="brave"}` — SEUL le nom du provider est tracé, aucun identifiant per-request/per-tenant.
- Level WARN/ERROR : upstream failures, timeouts (sans contenu requête ni identifier)

**Ce qui N'EST JAMAIS loggé (v1 strip total) :**
- Query string content (`user searched for X`)
- User/tenant ID (identifiable form) — **ni brut, ni hashé, ni bucketé**
- Upstream response body
- API key / credentials

**Format audit tracé :** compatible avec cm audit_events pattern ([cm#99 emitter](https://github.com/senara-solutions/control-monitor/issues/99)). L'egress search event v1 = `{type: "search_egress", upstream, latency_ms, status}` — sans `tenant_hash`, sans `tenant_id`, sans `query`.

**Rationale strip total (sami tranche 2026-08-18) :**

Bucket-64 avait été initialement proposé pour cohort SRE trouble-shooting. Sami a arbitré strip total en v1 :

> "Raison : bucket-64 sur une famille de quelques dizaines d'utilisateurs ≈ pseudonyme quasi-par-tenant → corrélable user-side, ce que la doctrine dé-liage interdit. Réversibilité-asymétrie : ajouter de la télémétrie-cohorte plus tard (avec bearing Prime) = facile ; dé-fuiter = dur. On assume zéro cohort-debug en v1."

**Conséquences opérationnelles v1 :**
- Aucun cohort-debug SRE (impossible de dire "X% erreurs sur bucket Y")
- Trouble-shooting possible UNIQUEMENT via metrics agrégats globaux (total counter + latency histogram)
- Si un jour un cohort-debug s'avère nécessaire, ce sera une **feature v2 gated Prime** (pas un ajout silent)

**Cette discipline transporte** : la Prime rule "construis l'incapacité, ne promets pas la retenue" (analogie #1796). On construit un log path qui NE PEUT PAS écrire un tenant identifier — pas seulement une promise que le code ne le fait pas.

---

## Interface `search()` (générique)

```rust
// crates/mika-gateway/src/egress_search.rs

/// Search egress client — the ONLY code path that talks to the search upstream.
/// Constructed once at gateway startup, shared across all tenants.
pub(crate) struct SearchEgressClient {
    inner: reqwest::Client,
    upstream: SearchUpstream,
}

/// Marker type — bounds what upstream this client can reach.
/// Enum forced at construction; no runtime swap possible.
pub(crate) enum SearchUpstream {
    Brave(BraveConfig),
    // Future: SearXNG(SearXNGConfig) — for E6 contingency only, if E4 fails
}

pub(crate) struct SearchRequest {
    pub query: String,
    pub max_results: usize,
}

pub(crate) struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub(crate) struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub upstream_latency_ms: u32,
}

impl SearchEgressClient {
    pub(crate) fn new(upstream: SearchUpstream) -> Self { ... }

    /// The SINGLE search entry point. All Mika agents call THIS (via HTTP
    /// endpoint on gateway), never the upstream directly.
    pub(crate) async fn search(&self, req: SearchRequest) -> Result<SearchResponse, SearchError> {
        // Instrumented per Q4. No query content in traces.
        ...
    }
}
```

**HTTP endpoint agent-facing** (added to gateway routes) :
```
POST /internal/search
Authorization: Bearer <MIKA_INTERNAL_TOKEN>
Body: {"query": "...", "max_results": 10}
Response: {"results": [...], "upstream_latency_ms": 245}
```

Agent (Mika-spirit) call = `POST /internal/search` — never calls Brave/upstream directly. Cette contrainte est enforced au niveau tooling agent (Brave API client dep = interdite dans `mika-agent` crate, seul `mika-gateway::egress_search` a `reqwest` pour upstream search — voir Q2 CI lint).

---

## Acceptance criteria mapping (from #1807)

| AC | Implementation |
|---|---|
| **AC1** — Interface `search()` générique documentée | `SearchRequest`/`SearchResponse` types + rustdoc |
| **AC2** — Une seule impl du path egress | Q2 marker types + CI lint (voir §Isolation) |
| **AC3** — Instrumentation métriques | Q4 tracing + Prometheus |
| **AC4** — Test build-time (compile fail / lint fail hors path egress) | Q2 (3) CI script |
| **AC5** — Doc `crates/mika-gateway/docs/egress-search.md` | Section architecture + garantie contrôle unique |

---

## Implementation shape (for spawn Agent)

**Files to create :**
- `crates/mika-gateway/src/egress_search.rs` — module principal (client + types + handler)
- `crates/mika-gateway/src/egress_search/tests.rs` — unit tests (marker type safety, no-content-in-logs assertion)
- `crates/mika-gateway/docs/egress-search.md` — doc architecture (AC5)
- `scripts/verify-egress-uniqueness.sh` — CI lint rejetant upstream import hors du path autorisé

**Files to modify :**
- `crates/mika-gateway/src/routes.rs` — register `POST /internal/search` route + auth middleware
- `crates/mika-gateway/src/main.rs` OR settings — inject `SearchEgressClient` into router state
- `crates/mika-gateway/src/settings.rs` — `SearchUpstream` config parsing
- `.github/workflows/ci.yml` OR similar — invoke `scripts/verify-egress-uniqueness.sh`

**Explicit scope boundaries (out of E1) :**
- Brave API client impl → E2 (#1808)
- Dé-liage identity↔request audit → E3 (#1809)
- No-log verified end-to-end (network layer) → E4 (#1810)
- Al testeur unblock (brave_api_key provisionnement) → E5 (#1811) downstream of E1-E4
- SearXNG self-hosted → E6 (#1812) design-only, contingency

**LOE estimate :** ~4-6h spawn Agent (module + tests + doc + CI script). Bounded, non-substrate-critical.

---

## Cross-references

- Milestone : [#1806](https://github.com/senara-solutions/mika/issues/1806) — search backend egress-controlled
- Sibling tickets : #1808 (E2), #1809 (E3), #1810 (E4), #1811 (E5), #1812 (E6)
- Bearing analogy : [#1796](https://github.com/senara-solutions/mika/issues/1796) — voice testimony non-transit build-time invariant
- Bearing source : samidarko relay 2026-07-19 (Prime+coherence reconciliation)
- Cross-repo pattern : voir `docs/solutions/cross-repo-patterns/` (build-time invariant discipline)

---

## Ready for review + spawn

Plan tranche les 4 questions. Aucun point identifié comme souverain sous Option A (placement gateway module).

**Ping sami** pour :
1. Validation ou pushback sur Q1 Option A (gateway module)
2. Validation Q2 (marker types + CI lint discipline)
3. Validation Q4 (tenant_hash cohort — ou strip total si trop invasif per ton e3 lens)

Après validation → spawn Agent impl 1 PR (Closes #1807). Cascade e2→e3→e4 après.

Si Q1 = souverain à ta lecture → route Vincent avant spawn.
