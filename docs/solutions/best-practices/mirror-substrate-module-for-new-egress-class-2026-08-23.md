---
title: "Mirror substrate module for a new egress class — reuse the pattern, not the module"
date: 2026-08-23
category: best-practices
module: mika-gateway
problem_type: architecture
component: egress-substrate
severity: high
applies_when:
  - Adding a new class of controlled outbound network I/O to the platform
  - Extending an existing egress substrate to serve a different upstream shape (search → GET fetch, GET fetch → webhook POST, DNS lookup)
  - Reviewing a PR that touches `crates/mika-gateway/src/egress_*/`
  - Deciding whether to mirror an existing substrate module vs add a variant to it
tags:
  - egress
  - substrate
  - controlled-egress
  - mirror-pattern
  - marker-types
  - ci-lint
  - q4-strip-total
  - security
---

# Mirror substrate module for a new egress class — reuse the pattern, not the module

## Context

The E1 substrate (mika#1807, milestone #1806) established the discipline for controlled outbound network I/O in mika-gateway: a single module owns the sole `reqwest::Client` reaching a search upstream (Brave), with marker types (`SearchEgressClient`), `pub(crate)` visibility, a Q4 STRIP TOTAL log invariant, and a CI lint (`scripts/verify-egress-uniqueness.sh`) that grep-fails the build if any file outside the authorized tree names the upstream identifier.

When mika#1969 added a second egress class — GET-only fetch against a compile-time gouv.fr allowlist — the tempting shortcut was to add a `FetchUpstream::GouvFr` variant to `egress_search::SearchUpstream` and reuse the same module. **That shortcut breaks the discipline by design.** The CI lint's uniqueness grep keys off the upstream identifier tokens (`api.search.brave.com`, now the four gouv.fr hosts). A shared module cannot own two upstream classes without either (a) collapsing the marker-type invariant (one client type for two egress classes = the "search" name is a lie) or (b) weakening the CI lint's per-token authoritative-owner claim.

The right answer is to **mirror the module**: `crates/mika-gateway/src/egress_fetch/` sits next to `egress_search/`, carries the same shape, and owns its own upstream identifier tokens in the CI lint.

## Guidance

When you need to add a new class of controlled egress to the platform, follow the mirror-module pattern. Do not extend an existing substrate module.

### Decision — mirror vs extend

Mirror the module when the new egress class shares the **pattern** (marker isolation + Q4 discipline + CI lint enforcement) but not the **upstream** (different host set, different response shape, different threat model, different HTTP method surface). Add a variant to the existing module's `Upstream` enum only when the new upstream is a drop-in replacement for the same class (e.g., "SearXNG as an alternate search backend" — same request shape, same response shape, same allowlist semantics).

Two worked examples:

| Class | Module | Upstream identifiers | Wire endpoint | HTTP method | Marker type |
|-------|--------|----------------------|---------------|-------------|-------------|
| Search (mika#1807) | `egress_search/` | `api.search.brave.com` | `POST /internal/search` | GET (to upstream) | `SearchEgressClient` |
| Fetch (mika#1969) | `egress_fetch/` | `service-public.fr`, `ants.gouv.fr`, `impots.gouv.fr`, `data.gouv.fr` | `POST /internal/fetch` | GET (to upstream) | `FetchEgressClient` |

Both modules are `pub(crate)`. Neither can be constructed from outside its own crate. Both emit exactly two `tracing::info!` events per call (a `*_requested` and a `*_egress` audit event) with a fixed field set the Q4 test enforces.

### Load-bearing invariants — preserve when mirroring

Every mirror carries these forward from the source module:

1. **Marker types.** `<Class>EgressClient` wraps `reqwest::Client` privately. No `pub` conversion. Handlers accept only `&<Class>EgressClient`. This is what the CI lint's uniqueness grep enforces at the file layer.
2. **`pub(crate)` visibility.** No cross-crate import possible. Every export in the module tree is `pub(crate)`. Adding a `pub` item is a review-fail.
3. **Q4 STRIP TOTAL log discipline.** Zero tenant identifiers, zero URL bytes, zero response bytes in any log field emitted by the module. The load-bearing test lives inline (`log_assertion_no_tenant_no_url_no_forbidden_fields` in `egress_fetch/gouv_fr.rs`; `log_assertion_no_tenant_no_query_no_forbidden_fields` in `egress_search/mod.rs`) and asserts:
   - Exactly N events emitted from the module's target prefix (2, for the request + audit pair).
   - Only the allowed field set on each event (`event`, `upstream`, `latency_ms`, `message` for search; add `host_class` for fetch — see § What varies below).
   - Cross-source assertion: sensitive URL/query bytes appear in NO captured field value across ANY event (our module + reqwest + hyper).
4. **CI lint extension.** `scripts/verify-egress-uniqueness.sh` adds the new module's upstream identifier tokens to `PATTERNS` and the module tree + related doc/plan paths to `AUTHORIZED_PATHS`. Absence of a legacy allowlist entry for a new class is load-bearing when the class delegates through the gateway (see § What may vary point 3).
5. **Configuration as code, not env.** The allowlist / upstream selection is a compile-time constant, not a runtime knob. Extending is a code change + deploy; there is no admin API. See `KTD2` in the mika#1969 plan for the founding rationale.

### What may vary per class

Only these four axes are permitted to differ:

1. **Allowlist shape.** Search may accept any URL Brave returns as a result; fetch enforces a compile-time host allowlist. Each class picks the shape that matches its threat model.
2. **Response schema.** `SearchResponse { results, upstream_latency_ms }` vs `FetchResponse { body, content_type, bytes_read }`. Both include a bounded side-channel; neither leaks a per-tenant bit.
3. **Timeout budget.** Search: 5 s hard cap (latency-sensitive). Fetch: 15 s hard cap with 10 s per-request timeout (gouv.fr sites are heavier and slower). Each class picks the budget that matches its upstream characteristics.
4. **Host-class taxonomy.** Search emits a single-value `upstream` label (`"brave"`). Fetch adds a `host_class` label with a bounded four-value taxonomy (`"service_public" | "ants" | "impots" | "data_gouv"`) because the caller can influence which upstream host is reached, and audit visibility on the class distinction is worth the four extra bits — the caller already knows this bit (they chose the URL). The taxonomy is bounded and pre-declared; the raw host bytes never appear in any log field. See `KTD3` in the mika#1969 plan.

## Anti-patterns

- **Extending `SearchEgressClient` to serve non-search calls.** Would fail the CI lint (`scripts/verify-egress-uniqueness.sh`) by design — the gouv.fr host tokens would appear in `crates/mika-gateway/src/egress_search/`, which is not on `AUTHORIZED_PATHS` for those tokens. The CI lint is not a bug to work around; it is the enforcement of the marker-type invariant.
- **Adding a `pub` conversion from `reqwest::Client` into `<Class>EgressClient`.** Defeats the marker-type isolation. If a test needs a client with the same profile, use the module's `pub(crate) fn build_client()` factory (as `egress_search::build_client` and `egress_fetch::build_client` provide).
- **Emitting additional structured fields in the request / audit events.** Breaks the Q4 test. If a new operator dashboard needs a new field, add it to the allowlist AND update every parallel substrate's Q4 test AND update this doc's § Load-bearing invariants point 3 in the same PR.
- **Adding a runtime env var to switch the allowlist.** Moves the security envelope from reviewer-mutable to operator-mutable. The mika#1969 plan's KTD2 documents this rationale; the sibling `INTERNAL_REPOS` const in `crates/mika-gateway/src/github.rs` follows the same reasoning.
- **Adding a `LEGACY_ALLOWLIST` entry for a new class's agent builtin.** Load-bearing absence: the class's agent-side builtin delegates through the gateway (`fetch_url` calls `POST /internal/fetch`; it does not name the hosts). A future reviewer might add a legacy entry defensively — a comment in `verify-egress-uniqueness.sh` documents the absence so this doesn't happen.

## Checklist for the next class

When adding a third egress class (e.g., `egress_webhook`, `egress_dns`), follow this ordered checklist. Each step corresponds to a Unit in the mika#1969 plan and can be tested in isolation.

1. **Module scaffold** — `crates/mika-gateway/src/egress_<class>/mod.rs`. Copy the mod.rs header from an existing substrate; rename nouns; keep the Q1–Q4 discipline block verbatim. Include marker types (`<Class>EgressClient`, `<Class>Upstream` enum, per-upstream config struct), the error taxonomy (`<Class>Error` with `http_status()` + `tracing_status()`), the `build_client()` factory, and an empty-stub `handle_internal_<class>` returning 501 for now.
2. **Substrate implementation** — `crates/mika-gateway/src/egress_<class>/<upstream>.rs`. The concrete client that talks to the upstream. Emits zero tracing calls (the audit event is emitted by the parent module). Handles allowlist / validation, timeout, size caps, response parsing.
3. **Wire the route** — add `pub(crate) fetch_egress_client: Option<egress_<class>::Shared<Class>EgressClient>` to `routes::AppState`, register `POST /internal/<class>` under `require_bearer_token`, construct the client once in `main.rs`, and thread `None` into every AppState test fixture (the compiler is the grep-audit here — every unpatched site fails to compile).
4. **CI lint extension** — add the new class's upstream identifier tokens to `PATTERNS`, the module tree + related plan/doc paths to `AUTHORIZED_PATHS`. Verify on a clean tree (exit 0) and by planting a canary in an unauthorized file (exit 1). Do NOT add a `LEGACY_ALLOWLIST` entry unless the class has a pre-substrate builtin actively being migrated (like `web_search` for Brave); a new class's builtin should delegate through the gateway from day one.
5. **Agent-side builtin** — extend `crates/mika-agent/src/tools/mod.rs::ToolContext` if the builtin needs new fields (mika#1969 added `gateway_url` and `internal_token` alongside the existing `brave_api_key` pattern). Add the builtin name to `KNOWN_BUILTINS` in `crates/mika-agent/src/skills/builtin_handlers.rs`, add a dispatch arm in `execute()`, implement the handler (delegate to the gateway substrate, fail-closed on missing config, Q4-safe error messages). Add the tool name to `BUILTIN_TOOL_NAMES` in `crates/mika-agent/src/tools/mod.rs` and add read/write classification in `crates/mika-agent/src/tools/classification.rs`.
6. **Q4 log-discipline test** — mirror `egress_fetch/gouv_fr.rs::tests::log_assertion_no_tenant_no_url_no_forbidden_fields`. This test is the AC gate; a failure here blocks the whole substrate. Do NOT weaken assertions to make it pass — fix the emit side.
7. **Solution doc** — if the new class exposes a new pattern axis (e.g., streaming responses, per-tenant credentials, retry semantics distinct from search + fetch), extend this doc's § What may vary section rather than writing a parallel one. If the class fits the existing four axes cleanly, no new doc is needed — cite this one.

## Sibling references

- **Origin plan (E1 substrate):** `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md` — the founding sami-tranchée for controlled egress, Q1–Q4 discipline established.
- **This class's plan (E2 fetch):** `docs/plans/1969-egress-fetch-fetch-url-builtin.md` — worked example of the mirror pattern.
- **CI lint:** `scripts/verify-egress-uniqueness.sh` — the marker-discipline enforcement.
- **Substrate source references:** `crates/mika-gateway/src/egress_search/mod.rs`, `crates/mika-gateway/src/egress_search/brave.rs`, `crates/mika-gateway/src/egress_fetch/mod.rs`, `crates/mika-gateway/src/egress_fetch/gouv_fr.rs`.
- **Load-bearing tests:** `egress_search::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`, `egress_fetch::gouv_fr::tests::log_assertion_no_tenant_no_url_no_forbidden_fields`.
- **Related pattern:** `docs/solutions/best-practices/security-hardening-patterns.md` (referenced by `verify-egress-uniqueness.sh:8`) — "construct the incapacity, don't promise the restraint."
